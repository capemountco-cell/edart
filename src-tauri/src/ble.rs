//! The Mac as a Bluetooth keyboard and mouse (HID over GATT), on CoreBluetooth.
//!
//! This is platform plumbing only. What to publish lives in
//! `edart_core::gatt`, what to send in `edart_core::gesture`. Here the table
//! becomes CoreBluetooth objects and reports become `updateValue:` calls.
//!
//! Threading: the peripheral manager delivers its callbacks on the main
//! queue, and every call into it happens on the main thread too. Other
//! threads read [`status`] (a mutex-guarded snapshot) and reach the
//! peripheral through `AppHandle::run_on_main_thread`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use dispatch2::{DispatchQueue, DispatchTime};

use edart_core::gatt::{self, CharSpec, ServiceSpec, Value};
use edart_core::status::BluetoothStatus;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_core_bluetooth::{
    CBATTError, CBATTRequest, CBAdvertisementDataLocalNameKey, CBAdvertisementDataServiceUUIDsKey,
    CBAttributePermissions, CBCentral, CBCharacteristic, CBCharacteristicProperties,
    CBManagerState, CBMutableCharacteristic, CBMutableDescriptor, CBMutableService,
    CBPeripheralManager, CBPeripheralManagerDelegate, CBService, CBUUID,
};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSString};

/// Most reports kept while CoreBluetooth's send queue is full. Pointer
/// reports go stale fast; replaying old ones only makes the pointer jump.
const MAX_PENDING: usize = 64;
/// How long a phone that read HID Information gets to start listening
/// before Edart decides it is holding an old setup.
const STALE_CHECK: Duration = Duration::from_secs(4);
/// Least time between two Service Changed nudges to the same phone.
const NUDGE_EVERY: Duration = Duration::from_secs(30);

static STATUS: Mutex<Option<BluetoothStatus>> = Mutex::new(None);

fn update_status(f: impl FnOnce(&mut BluetoothStatus)) {
    let mut guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    let status = guard.get_or_insert_with(|| BluetoothStatus {
        supported: true,
        state: "unknown".into(),
        ..Default::default()
    });
    f(status);
}

static STARTED: OnceLock<Instant> = OnceLock::new();

/// Add a line to the Details log, stamped with seconds since start.
pub(crate) fn log(message: impl AsRef<str>) {
    let t = STARTED.get_or_init(Instant::now).elapsed().as_secs_f32();
    let line = format!("{t:7.1}s  {}", message.as_ref());
    update_status(|s| s.log(line));
}

/// The first 8 characters of a central's identifier, enough to tell phones apart.
fn who(central: &CBCentral) -> String {
    let id = unsafe { central.identifier().UUIDString() }.to_string();
    id.chars().take(8).collect()
}

fn uuid_of(ch: &CBCharacteristic) -> String {
    unsafe { ch.UUID().UUIDString() }.to_string()
}

/// A snapshot of the link, safe to call from any thread.
pub fn status() -> BluetoothStatus {
    let guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    guard.clone().unwrap_or(BluetoothStatus {
        supported: true,
        state: "notStarted".into(),
        ..Default::default()
    })
}

/// What a characteristic is, so callbacks can answer for it.
#[derive(Clone)]
enum Role {
    Input(u8),
    Output,
    ControlPoint,
    ServiceChanged,
    Dynamic(Vec<u8>),
    Other,
}

struct Peripheral {
    manager: Retained<CBPeripheralManager>,
    // CoreBluetooth holds its delegate weakly; this keeps it alive.
    _delegate: Retained<Delegate>,
    to_add: VecDeque<ServiceSpec>,
    services: Vec<Retained<CBMutableService>>,
    roles: Vec<(Retained<CBMutableCharacteristic>, Role)>,
    inputs: HashMap<u8, Retained<CBMutableCharacteristic>>,
    subscribers: HashMap<u8, HashSet<String>>,
    pending: VecDeque<(u8, Vec<u8>)>,
    service_changed: Option<Retained<CBMutableCharacteristic>>,
    /// Phones subscribed to Service Changed.
    sc_subscribers: HashSet<String>,
    /// When each phone was last sent a Service Changed.
    sc_sent: HashMap<String, Instant>,
    /// Phones that read HID Information, checked again after
    /// [`STALE_CHECK`] to see whether they started listening.
    watching: HashMap<String, Retained<CBCentral>>,
    /// This launch's Database Hash, so a reinstalled Edart looks new.
    db_hash: [u8; 16],
    /// macOS refused Generic Attribute with the hash; publish it without.
    hash_refused: bool,
}

thread_local! {
    static PERIPHERAL: RefCell<Option<Peripheral>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "EdartPeripheralDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CBPeripheralManagerDelegate for Delegate {
        #[unsafe(method(peripheralManagerDidUpdateState:))]
        fn did_update_state(&self, manager: &CBPeripheralManager) {
            on_state(manager);
        }

        #[unsafe(method(peripheralManager:didAddService:error:))]
        fn did_add_service(
            &self,
            _manager: &CBPeripheralManager,
            service: &CBService,
            error: Option<&NSError>,
        ) {
            on_service_added(service, error);
        }

        #[unsafe(method(peripheralManagerDidStartAdvertising:error:))]
        fn did_start_advertising(&self, manager: &CBPeripheralManager, error: Option<&NSError>) {
            let advertising = unsafe { manager.isAdvertising() };
            let message = error.map(|e| e.localizedDescription().to_string());
            match &message {
                None => log(format!(
                    "advertising: iPhones can now find “{}” in AssistiveTouch",
                    gatt::ADVERTISED_NAME
                )),
                Some(m) => log(format!("advertising failed: {m}")),
            }
            update_status(|s| {
                s.advertising = advertising || message.is_none();
                if message.is_some() {
                    s.error = message;
                }
            });
        }

        #[unsafe(method(peripheralManager:central:didSubscribeToCharacteristic:))]
        fn did_subscribe(
            &self,
            _manager: &CBPeripheralManager,
            central: &CBCentral,
            characteristic: &CBCharacteristic,
        ) {
            on_subscription(central, characteristic, true);
        }

        #[unsafe(method(peripheralManager:central:didUnsubscribeFromCharacteristic:))]
        fn did_unsubscribe(
            &self,
            _manager: &CBPeripheralManager,
            central: &CBCentral,
            characteristic: &CBCharacteristic,
        ) {
            on_subscription(central, characteristic, false);
        }

        #[unsafe(method(peripheralManager:didReceiveReadRequest:))]
        fn did_receive_read(&self, manager: &CBPeripheralManager, request: &CBATTRequest) {
            on_read(manager, request);
        }

        #[unsafe(method(peripheralManager:didReceiveWriteRequests:))]
        fn did_receive_writes(
            &self,
            manager: &CBPeripheralManager,
            requests: &NSArray<CBATTRequest>,
        ) {
            on_writes(requests);
            // Nothing to store: LED state and control-point writes carry no
            // input. Answer once for the whole batch, as CoreBluetooth wants.
            if let Some(first) = requests.firstObject() {
                unsafe { manager.respondToRequest_withResult(&first, CBATTError::Success) };
            }
        }

        #[unsafe(method(peripheralManagerIsReadyToUpdateSubscribers:))]
        fn ready_to_update(&self, _manager: &CBPeripheralManager) {
            drain();
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn state_name(state: CBManagerState) -> &'static str {
    match state {
        CBManagerState::Resetting => "resetting",
        CBManagerState::Unsupported => "unsupported",
        CBManagerState::Unauthorized => "unauthorized",
        CBManagerState::PoweredOff => "poweredOff",
        CBManagerState::PoweredOn => "poweredOn",
        _ => "unknown",
    }
}

/// Start the peripheral. Call once, on the main thread. Bluetooth permission
/// is asked for here, the first time.
pub fn start(mtm: MainThreadMarker) {
    PERIPHERAL.with(|cell| {
        if cell.borrow().is_some() {
            return;
        }
        let mac_name = computer_name();
        update_status(|s| {
            s.state = "unknown".into();
            s.mac_name = mac_name;
        });
        let delegate = Delegate::new(mtm);
        let manager = unsafe {
            CBPeripheralManager::initWithDelegate_queue(
                CBPeripheralManager::alloc(),
                Some(ProtocolObject::from_ref(&*delegate)),
                None,
            )
        };
        *cell.borrow_mut() = Some(Peripheral {
            manager,
            _delegate: delegate,
            to_add: VecDeque::new(),
            services: Vec::new(),
            roles: Vec::new(),
            inputs: HashMap::new(),
            subscribers: HashMap::new(),
            pending: VecDeque::new(),
            service_changed: None,
            sc_subscribers: HashSet::new(),
            sc_sent: HashMap::new(),
            watching: HashMap::new(),
            db_hash: fresh_database_hash(),
            hash_refused: false,
        });
    });
    log("started");
}

/// Pause: take the keyboard-and-mouse services down so the iPhone is free
/// (its pointer goes away), but keep the pairing. Main thread only.
pub fn pause() {
    PERIPHERAL.with(|cell| {
        if let Some(p) = cell.borrow_mut().as_mut() {
            unsafe {
                p.manager.stopAdvertising();
                p.manager.removeAllServices();
            }
            p.to_add.clear();
            p.services.clear();
            p.roles.clear();
            p.inputs.clear();
            p.subscribers.clear();
            p.pending.clear();
            p.service_changed = None;
            p.sc_subscribers.clear();
            p.sc_sent.clear();
            p.watching.clear();
        }
    });
    update_status(|s| {
        s.paused = true;
        s.refreshing = false;
        s.published = false;
        s.advertising = false;
        s.subscribed = false;
        s.phone_seen = false;
    });
    log("paused: services taken down, the iPhone is free (pairing kept)");
}

/// Undo [`pause`]: publish the services again; the paired iPhone
/// reconnects by itself. Main thread only.
pub fn resume() {
    update_status(|s| s.paused = false);
    log("resuming");
    restart();
}

/// Throw the GATT database away and publish it again. Phones that cached
/// the old one are told to rediscover.
pub fn restart() {
    let on = PERIPHERAL.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|p| unsafe { p.manager.state() } == CBManagerState::PoweredOn)
            .unwrap_or(false)
    });
    if on {
        log("restarting Bluetooth services");
        rebuild();
    }
}

fn on_state(manager: &CBPeripheralManager) {
    let state = unsafe { manager.state() };
    update_status(|s| s.state = state_name(state).into());
    log(format!("Bluetooth {}", state_name(state)));
    // Paused: stay down until Resume, even if Bluetooth comes back on.
    if status().paused {
        return;
    }
    if state != CBManagerState::PoweredOn {
        update_status(|s| {
            s.advertising = false;
            s.subscribed = false;
        });
        return;
    }
    rebuild();
}

/// After power-on or a reset the old database is gone: build it again.
fn rebuild() {
    PERIPHERAL.with(|cell| {
        if let Some(p) = cell.borrow_mut().as_mut() {
            unsafe {
                p.manager.stopAdvertising();
                p.manager.removeAllServices();
            }
            p.services.clear();
            p.roles.clear();
            p.inputs.clear();
            p.subscribers.clear();
            p.pending.clear();
            p.service_changed = None;
            p.sc_subscribers.clear();
            p.sc_sent.clear();
            p.watching.clear();
            let mut services = gatt::services(p.db_hash);
            if p.hash_refused {
                services[0] = gatt::generic_attribute(None);
            }
            p.to_add = services.into();
        }
    });
    update_status(|s| {
        s.published = false;
        s.advertising = false;
        s.subscribed = false;
        s.phone_seen = false;
        s.refreshing = false;
        s.failed_services.clear();
        s.error = None;
    });
    add_next();
}

/// Services are added one at a time: each waits for the previous callback.
fn add_next() {
    let advertise = PERIPHERAL.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(p) = borrow.as_mut() else {
            return false;
        };
        let Some(spec) = p.to_add.pop_front() else {
            return true;
        };
        let service = build_service(p, &spec);
        unsafe { p.manager.addService(&service) };
        p.services.push(service);
        false
    });
    if advertise {
        start_advertising();
    }
}

fn uuid(s: &str) -> Retained<CBUUID> {
    unsafe { CBUUID::UUIDWithString(&NSString::from_str(s)) }
}

fn build_service(p: &mut Peripheral, spec: &ServiceSpec) -> Retained<CBMutableService> {
    let mut chars: Vec<Retained<CBCharacteristic>> = Vec::new();
    for c in &spec.characteristics {
        let ch = build_characteristic(c);
        let role = match (c.report, &c.value) {
            (Some((id, gatt::REPORT_TYPE_INPUT)), _) => Role::Input(id),
            (Some(_), _) => Role::Output,
            (None, _) if c.uuid == gatt::CHAR_CONTROL_POINT => Role::ControlPoint,
            (None, _) if c.uuid == gatt::CHAR_SERVICE_CHANGED => Role::ServiceChanged,
            (None, Value::Dynamic(v)) => Role::Dynamic(v.clone()),
            _ => Role::Other,
        };
        if let Role::Input(id) = role {
            p.inputs.insert(id, ch.clone());
        }
        if let Role::ServiceChanged = role {
            p.service_changed = Some(ch.clone());
        }
        p.roles.push((ch.clone(), role));
        chars.push(Retained::into_super(ch));
    }
    let service = unsafe {
        CBMutableService::initWithType_primary(
            CBMutableService::alloc(),
            &uuid(&spec.wire_uuid()),
            true,
        )
    };
    unsafe { service.setCharacteristics(Some(&NSArray::from_retained_slice(&chars))) };
    service
}

fn build_characteristic(c: &CharSpec) -> Retained<CBMutableCharacteristic> {
    let value = match &c.value {
        Value::Static(bytes) => Some(NSData::with_bytes(bytes)),
        _ => None,
    };
    let ch = unsafe {
        CBMutableCharacteristic::initWithType_properties_value_permissions(
            CBMutableCharacteristic::alloc(),
            &uuid(&gatt::short(c.uuid)),
            CBCharacteristicProperties(c.properties as _),
            value.as_deref(),
            CBAttributePermissions(c.permissions as _),
        )
    };
    if let Some((id, kind)) = c.report {
        let data = NSData::with_bytes(&[id, kind]);
        let data: &AnyObject = &data;
        let descriptor = unsafe {
            CBMutableDescriptor::initWithType_value(
                CBMutableDescriptor::alloc(),
                &uuid(&gatt::short(gatt::DESC_REPORT_REFERENCE)),
                Some(data),
            )
        };
        let descriptors = NSArray::from_retained_slice(&[Retained::into_super(descriptor)]);
        unsafe { ch.setDescriptors(Some(&descriptors)) };
    }
    ch
}

fn on_service_added(service: &CBService, error: Option<&NSError>) {
    let id = unsafe { service.UUID().UUIDString() }.to_string();
    match error {
        None => log(format!("service added {id}")),
        Some(e) => log(format!(
            "service REFUSED {id}: {}",
            e.localizedDescription()
        )),
    }
    if error.is_some() && id.to_uppercase().contains("1801") {
        // Most likely the Database Hash. Service Changed matters more:
        // publish Generic Attribute again without the hash.
        let retry = PERIPHERAL.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let p = borrow.as_mut()?;
            if p.hash_refused {
                return None;
            }
            p.hash_refused = true;
            p.to_add.push_front(gatt::generic_attribute(None));
            Some(())
        });
        if retry.is_some() {
            log("publishing Generic Attribute again without the Database Hash");
            add_next();
            return;
        }
    }
    if let Some(error) = error {
        // Keep going: an optional service being refused must not stop
        // advertising. A refused HID service shows up in the status.
        let id = unsafe { service.UUID().UUIDString() }.to_string();
        let message = error.localizedDescription().to_string();
        update_status(|s| {
            s.failed_services.push(id);
            s.error = Some(message);
        });
    }
    add_next();
}

fn start_advertising() {
    PERIPHERAL.with(|cell| {
        let borrow = cell.borrow();
        let Some(p) = borrow.as_ref() else { return };
        if unsafe { p.manager.isAdvertising() } {
            log(format!(
                "already advertising as “{}”",
                gatt::ADVERTISED_NAME
            ));
            return;
        }
        log(format!(
            "asking macOS to advertise as “{}” (keyboard and mouse)",
            gatt::ADVERTISED_NAME
        ));
        // Name and the 16-bit HID UUID only: more overflows the 31-byte
        // packet and the phone spins forever in its device list.
        let name = NSString::from_str(gatt::ADVERTISED_NAME);
        let services = NSArray::from_retained_slice(&[uuid(&gatt::short(gatt::SERVICE_HID))]);
        let name: &AnyObject = &name;
        let services: &AnyObject = &services;
        let keys = unsafe {
            [
                CBAdvertisementDataLocalNameKey,
                CBAdvertisementDataServiceUUIDsKey,
            ]
        };
        let data = NSDictionary::from_slices(&keys, &[name, services]);
        unsafe { p.manager.startAdvertising(Some(&data)) };
    });
    update_status(|s| s.published = true);
    // With the services up, connect to linked iPhones from the Mac side.
    crate::ble_central::reach_out();
}

fn same(a: &CBCharacteristic, b: &CBMutableCharacteristic) -> bool {
    std::ptr::eq(
        a as *const CBCharacteristic as *const u8,
        b as *const _ as *const u8,
    )
}

fn role_of(p: &Peripheral, ch: &CBCharacteristic) -> Role {
    p.roles
        .iter()
        .find(|(c, _)| same(ch, c))
        .map(|(_, r)| r.clone())
        .unwrap_or(Role::Other)
}

fn on_read(manager: &CBPeripheralManager, request: &CBATTRequest) {
    update_status(|s| s.phone_seen = true);
    let ch = unsafe { request.characteristic() };
    let central = unsafe { request.central() };
    log(format!(
        "read {} offset {} by {}",
        uuid_of(&ch),
        unsafe { request.offset() },
        who(&central)
    ));
    if uuid_of(&ch) == gatt::short(gatt::CHAR_HID_INFO) {
        watch_for_stale_setup(central.clone());
    }
    let value = PERIPHERAL.with(|cell| {
        let borrow = cell.borrow();
        let p = borrow.as_ref()?;
        let ch = unsafe { request.characteristic() };
        match role_of(p, &ch) {
            Role::Input(id) => Some(vec![0; edart_core::hid::report_len(id)]),
            Role::Output => Some(vec![0]),
            Role::Dynamic(v) => Some(v),
            Role::ControlPoint | Role::ServiceChanged => None,
            Role::Other => unsafe { ch.value() }.map(|d| d.to_vec()),
        }
    });
    let result = match value {
        None => CBATTError::AttributeNotFound,
        Some(v) => {
            let offset = unsafe { request.offset() };
            if offset > v.len() {
                CBATTError::InvalidOffset
            } else {
                unsafe { request.setValue(Some(&NSData::with_bytes(&v[offset..]))) };
                CBATTError::Success
            }
        }
    };
    unsafe { manager.respondToRequest_withResult(request, result) };
}

fn on_subscription(central: &CBCentral, ch: &CBCharacteristic, subscribed: bool) {
    update_status(|s| s.phone_seen = true);
    log(format!(
        "{} {} by {}",
        if subscribed {
            "subscribe"
        } else {
            "unsubscribe"
        },
        uuid_of(ch),
        who(central)
    ));
    let any = PERIPHERAL.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let p = borrow.as_mut()?;
        let who = unsafe { central.identifier().UUIDString() }.to_string();
        let id = match role_of(p, ch) {
            Role::Input(id) => id,
            Role::ServiceChanged => {
                if subscribed {
                    p.sc_subscribers.insert(who);
                } else {
                    p.sc_subscribers.remove(&who);
                }
                return None;
            }
            _ => return None,
        };
        let set = p.subscribers.entry(id).or_default();
        if subscribed {
            set.insert(who);
        } else {
            set.remove(&who);
        }
        Some(p.subscribers.values().any(|s| !s.is_empty()))
    });
    if let Some(any) = any {
        let mut refreshed = false;
        update_status(|s| {
            // Losing the last subscriber means the phone went away.
            if s.subscribed && !any {
                s.phone_seen = false;
            }
            s.subscribed = any;
            if any && s.refreshing {
                s.refreshing = false;
                refreshed = true;
            }
        });
        if refreshed {
            log("the iPhone refreshed its copy of Edart and is listening");
        }
    }
}

fn on_writes(requests: &NSArray<CBATTRequest>) {
    update_status(|s| s.phone_seen = true);
    for request in requests.iter() {
        let ch = unsafe { request.characteristic() };
        let central = unsafe { request.central() };
        let len = unsafe { request.value() }.map(|d| d.len()).unwrap_or(0);
        log(format!(
            "write {} ({len} bytes) by {}",
            uuid_of(&ch),
            who(&central)
        ));
        let is_control = PERIPHERAL.with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|p| matches!(role_of(p, &ch), Role::ControlPoint))
                .unwrap_or(false)
        });
        if is_control {
            recover_stale_cache(&central, "wrote the Control Point");
        }
    }
}

/// A phone that read HID Information should subscribe to input within a
/// moment. Check again after [`STALE_CHECK`]; one that still has not is
/// holding an old copy of Edart's services (typically after Edart was
/// reinstalled) and is told to look again.
fn watch_for_stale_setup(central: Retained<CBCentral>) {
    let id = unsafe { central.identifier().UUIDString() }.to_string();
    let first = PERIPHERAL.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let p = borrow.as_mut()?;
        Some(p.watching.insert(id.clone(), central).is_none())
    });
    if first != Some(true) {
        return;
    }
    let Ok(when) = DispatchTime::try_from(STALE_CHECK) else {
        return;
    };
    // The main queue is where every other callback runs, so the
    // peripheral is reachable from the closure.
    let _ = DispatchQueue::main().after(when, move || check_stale_setup(id));
}

fn check_stale_setup(id: String) {
    let central = PERIPHERAL.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let p = borrow.as_mut()?;
        let central = p.watching.remove(&id)?;
        let listening = p.subscribers.values().any(|s| s.contains(&id));
        (!listening).then_some(central)
    });
    if let Some(central) = central {
        recover_stale_cache(&central, "read HID Information but never subscribed");
    }
}

/// A bonded phone that trusts a stale copy of the Mac's services reads or
/// writes a little but never subscribes to input. Service Changed makes it
/// rediscover. Sent at most every [`NUDGE_EVERY`] per phone.
fn recover_stale_cache(central: &CBCentral, why: &str) {
    let sent = PERIPHERAL.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let p = borrow.as_mut()?;
        let id = unsafe { central.identifier().UUIDString() }.to_string();
        let has_input = p.subscribers.values().any(|s| s.contains(&id));
        let recent = p
            .sc_sent
            .get(&id)
            .is_some_and(|t| t.elapsed() < NUDGE_EVERY);
        if has_input || recent {
            return None;
        }
        let ch = p.service_changed.clone()?;
        p.sc_sent.insert(id.clone(), Instant::now());
        let subscribed = p.sc_subscribers.contains(&id);
        let centrals = NSArray::from_slice(&[central]);
        let ok = unsafe {
            p.manager
                .updateValue_forCharacteristic_onSubscribedCentrals(
                    &NSData::with_bytes(&gatt::SERVICE_CHANGED_ALL),
                    &ch,
                    Some(&centrals),
                )
        };
        Some((ok, subscribed))
    });
    if let Some((ok, subscribed)) = sent {
        update_status(|s| s.refreshing = true);
        log(format!(
            "old setup on {} ({why}): sent Service Changed (queued {ok}, phone listening for it {subscribed})",
            who(central)
        ));
    }
}

/// A Database Hash that differs on every launch.
fn fresh_database_hash() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let random = |n: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(n);
        h.finish()
    };
    gatt::database_hash(((random(1) as u128) << 64) | random(2) as u128)
}

/// Send one input report. Main thread only. Returns false when there is no
/// published characteristic for it (Bluetooth off or not started).
pub fn send(report_id: u8, payload: &[u8]) -> bool {
    PERIPHERAL.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(p) = borrow.as_mut() else {
            return false;
        };
        let Some(ch) = p.inputs.get(&report_id).cloned() else {
            return false;
        };
        // Keep order: if anything is already waiting, queue behind it.
        if !p.pending.is_empty() || !update(p, &ch, payload) {
            p.pending.push_back((report_id, payload.to_vec()));
            while p.pending.len() > MAX_PENDING {
                p.pending.pop_front();
            }
        }
        true
    })
}

fn update(p: &Peripheral, ch: &CBMutableCharacteristic, payload: &[u8]) -> bool {
    unsafe {
        p.manager
            .updateValue_forCharacteristic_onSubscribedCentrals(
                &NSData::with_bytes(payload),
                ch,
                None,
            )
    }
}

fn drain() {
    PERIPHERAL.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(p) = borrow.as_mut() else { return };
        while let Some((id, payload)) = p.pending.front().cloned() {
            let Some(ch) = p.inputs.get(&id).cloned() else {
                p.pending.pop_front();
                continue;
            };
            if !update(p, &ch, &payload) {
                break;
            }
            p.pending.pop_front();
        }
    });
}

/// True when the user already allowed Bluetooth, so starting will not
/// show a permission prompt.
pub fn already_allowed() -> bool {
    use objc2_core_bluetooth::{CBManager, CBManagerAuthorization};
    unsafe { CBManager::authorization_class() == CBManagerAuthorization::AllowedAlways }
}

/// The Mac's computer name ("Nick's MacBook"), which is what the iPhone
/// shows in its Bluetooth device list.
#[allow(deprecated)] // NSHost still answers this; the replacement needs SystemConfiguration.
fn computer_name() -> Option<String> {
    objc2_foundation::NSHost::currentHost()
        .localizedName()
        .map(|n| n.to_string())
}
