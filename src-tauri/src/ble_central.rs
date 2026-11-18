//! The Mac reaching out to the iPhone over Bluetooth (central role).
//!
//! An iPhone that already knows the Mac (same Apple Account: iCloud pairing
//! for Handoff and AirDrop) lists it as "Connected" and never opens a
//! keyboard-and-mouse session with it. iPhone Use (MIT) connects to such a
//! phone from the Mac side to wake its HID client. This module finds iPhones
//! already linked to the Mac and opens an app-level connection to each, then
//! logs what happens, so a real test shows whether it helps.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_core_bluetooth::{
    CBCentralManager, CBCentralManagerDelegate, CBManagerState, CBPeripheral, CBUUID,
};
use objc2_foundation::{NSArray, NSError, NSString};

use crate::ble::log;

/// Services an iPhone linked to this Mac typically exposes. Any match is
/// enough for `retrieveConnectedPeripheralsWithServices:` to return it.
const LINKED_SERVICES: &[&str] = &[
    "180A",                                 // Device Information
    "180F",                                 // Battery
    "1805",                                 // Current Time
    "D0611E78-BBB4-4591-A5F8-487910AE4366", // Apple Continuity
    "9FA480E0-4967-4542-9390-D343DC5D04AE", // Apple Nearby
    "7905F431-B5CE-4E99-A40F-4B1E122D00D0", // Apple Notification Center
    "89D3502B-0F36-433A-8EF4-C502AD55F8DC", // Apple Media
];

struct Central {
    manager: Retained<CBCentralManager>,
    _delegate: Retained<CentralDelegate>,
    /// CoreBluetooth drops a connection when its CBPeripheral is released.
    held: Vec<Retained<CBPeripheral>>,
}

thread_local! {
    static CENTRAL: RefCell<Option<Central>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "EdartCentralDelegate"]
    struct CentralDelegate;

    unsafe impl NSObjectProtocol for CentralDelegate {}

    unsafe impl CBCentralManagerDelegate for CentralDelegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn did_update_state(&self, central: &CBCentralManager) {
            if unsafe { central.state() } == CBManagerState::PoweredOn {
                reach_out();
            }
        }

        #[unsafe(method(centralManager:didConnectPeripheral:))]
        fn did_connect(&self, _central: &CBCentralManager, peripheral: &CBPeripheral) {
            log(format!("central: connected to {}", describe(peripheral)));
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn did_fail(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            error: Option<&NSError>,
        ) {
            let why = error
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_default();
            log(format!(
                "central: could not connect to {}: {why}",
                describe(peripheral)
            ));
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn did_disconnect(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
            error: Option<&NSError>,
        ) {
            let why = error
                .map(|e| e.localizedDescription().to_string())
                .unwrap_or_default();
            log(format!(
                "central: disconnected from {} {why}",
                describe(peripheral)
            ));
        }
    }
);

impl CentralDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn describe(p: &CBPeripheral) -> String {
    let name = unsafe { p.name() }
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unnamed device".into());
    let id: String = unsafe { p.identifier().UUIDString() }
        .to_string()
        .chars()
        .take(8)
        .collect();
    format!("{name} ({id})")
}

/// Create the central manager. Main thread only. Reaching out starts once
/// Bluetooth reports powered on.
pub fn start(mtm: MainThreadMarker) {
    CENTRAL.with(|cell| {
        if cell.borrow().is_some() {
            return;
        }
        let delegate = CentralDelegate::new(mtm);
        let manager = unsafe {
            CBCentralManager::initWithDelegate_queue(
                CBCentralManager::alloc(),
                Some(ProtocolObject::from_ref(&*delegate)),
                None,
            )
        };
        *cell.borrow_mut() = Some(Central {
            manager,
            _delegate: delegate,
            held: Vec::new(),
        });
    });
}

/// Find devices already linked to this Mac and connect to each from the Mac
/// side. Safe to call again: it re-lists and reconnects. Main thread only.
pub fn reach_out() {
    CENTRAL.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(c) = borrow.as_mut() else {
            log("central: not started");
            return;
        };
        if unsafe { c.manager.state() } != CBManagerState::PoweredOn {
            log("central: Bluetooth not on yet");
            return;
        }
        let uuids: Vec<Retained<CBUUID>> = LINKED_SERVICES
            .iter()
            .map(|s| unsafe { CBUUID::UUIDWithString(&NSString::from_str(s)) })
            .collect();
        let found = unsafe {
            c.manager
                .retrieveConnectedPeripheralsWithServices(&NSArray::from_retained_slice(&uuids))
        };
        log(format!("central: {} linked device(s) found", found.len()));
        c.held.clear();
        for p in found.iter() {
            log(format!("central: connecting to {}", describe(&p)));
            unsafe { c.manager.connectPeripheral_options(&p, None) };
            c.held.push(p);
        }
    });
}
