//! Apple's MobileDevice framework (part of macOS), which speaks lockdown to
//! an iPhone on the cable: is one plugged in, does it trust this Mac, is
//! Developer Mode on. Opened at run time, never linked.
//!
//! The call sequence (connect, validate pairing, start session) is
//! iphone-use's `MobileDevice.swift` (MIT, Sunghyun Kim); listing devices
//! with `AMDCreateDeviceList` after a subscription is iPhone Use's
//! `NativeDDIInspection.swift` (MIT, YueChen-C).

// Each symbol is cast to the type of the struct field it fills.
#![allow(clippy::missing_transmute_annotations)]

use std::ffi::{c_int, c_void};
use std::sync::OnceLock;

use edart_core::usb::lockdown::{self, AmfiAction, AmfiReply};
use edart_core::usb::readiness::{DeviceFacts, Link};

use super::sys::{self, CFTypeRef, Cf, Library};

const FRAMEWORK: &str = "/System/Library/PrivateFrameworks/MobileDevice.framework/MobileDevice";

type Device = *mut c_void;
type DeviceCall = unsafe extern "C" fn(Device) -> c_int;
type Callback = extern "C" fn(*mut c_void, *mut c_void);

struct Api {
    subscribe: unsafe extern "C" fn(Callback, u32, u32, *mut c_void, *mut *mut c_void) -> c_int,
    create_list: unsafe extern "C" fn() -> CFTypeRef,
    copy_identifier: unsafe extern "C" fn(Device) -> CFTypeRef,
    interface_type: Option<DeviceCall>,
    connect: DeviceCall,
    disconnect: DeviceCall,
    is_paired: DeviceCall,
    pair: Option<DeviceCall>,
    validate_pairing: DeviceCall,
    start_session: DeviceCall,
    stop_session: DeviceCall,
    copy_value: unsafe extern "C" fn(Device, CFTypeRef, CFTypeRef) -> CFTypeRef,
    start_service: unsafe extern "C" fn(Device, CFTypeRef, CFTypeRef, *mut CFTypeRef) -> c_int,
    send: unsafe extern "C" fn(CFTypeRef, *const c_void, usize) -> c_int,
    receive: unsafe extern "C" fn(CFTypeRef, *mut c_void, usize) -> c_int,
    invalidate: unsafe extern "C" fn(CFTypeRef) -> c_int,
}

/// The framework calls this for every plug and unplug. The list is read
/// with `AMDCreateDeviceList` instead, so there is nothing to do.
extern "C" fn on_device_event(_info: *mut c_void, _context: *mut c_void) {}

fn load() -> Result<Api, String> {
    let library = Library::open(FRAMEWORK)?;
    macro_rules! required {
        ($name:literal) => {{
            let address = library
                .symbol($name)
                .ok_or_else(|| format!("MobileDevice has no {}", $name.to_string_lossy()))?;
            // SAFETY: the symbol is a C function with the signature the
            // field declares, as the reference implementations call it.
            unsafe { std::mem::transmute::<*mut c_void, _>(address) }
        }};
    }
    macro_rules! optional {
        ($name:literal) => {
            library.symbol($name).map(|address| {
                // SAFETY: as above.
                unsafe { std::mem::transmute::<*mut c_void, _>(address) }
            })
        };
    }
    let api = Api {
        subscribe: required!(c"AMDeviceNotificationSubscribe"),
        create_list: required!(c"AMDCreateDeviceList"),
        copy_identifier: required!(c"AMDeviceCopyDeviceIdentifier"),
        interface_type: optional!(c"AMDeviceGetInterfaceType"),
        connect: required!(c"AMDeviceConnect"),
        disconnect: required!(c"AMDeviceDisconnect"),
        is_paired: required!(c"AMDeviceIsPaired"),
        pair: optional!(c"AMDevicePair"),
        validate_pairing: required!(c"AMDeviceValidatePairing"),
        start_session: required!(c"AMDeviceStartSession"),
        stop_session: required!(c"AMDeviceStopSession"),
        copy_value: required!(c"AMDeviceCopyValue"),
        start_service: required!(c"AMDeviceSecureStartService"),
        send: required!(c"AMDServiceConnectionSend"),
        receive: required!(c"AMDServiceConnectionReceive"),
        invalidate: required!(c"AMDServiceConnectionInvalidate"),
    };
    // Devices are announced on this thread's run loop, which `devices`
    // spins. The subscription lives as long as the app.
    let mut subscription = std::ptr::null_mut();
    // SAFETY: a valid callback and out-pointer; no context.
    let code = unsafe {
        (api.subscribe)(
            on_device_event,
            0,
            0,
            std::ptr::null_mut(),
            &mut subscription,
        )
    };
    if code != 0 {
        return Err(failure("AMDeviceNotificationSubscribe", code));
    }
    Ok(api)
}

fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(load).as_ref().map_err(Clone::clone)
}

fn failure(call: &str, code: c_int) -> String {
    format!("{call} failed (0x{:08x})", code as u32)
}

/// The iPhone on the cable, kept alive by the device list it came from.
pub struct Phone {
    api: &'static Api,
    device: Device,
    _list: Cf,
    pub udid: String,
}

/// Find the iPhone plugged in over USB. Must always be called on the same
/// thread: the framework announces devices on that thread's run loop.
pub fn find_phone() -> Result<Option<Phone>, String> {
    let api = api()?;
    // SAFETY: plain CoreFoundation calls; every object used is alive while
    // `list` is.
    unsafe {
        sys::CFRunLoopRunInMode(sys::kCFRunLoopDefaultMode, 0.25, 0);
        let Some(list) = Cf::owned((api.create_list)()) else {
            return Ok(None);
        };
        for index in 0..sys::CFArrayGetCount(list.0) {
            let device = sys::CFArrayGetValueAtIndex(list.0, index) as Device;
            if device.is_null() {
                continue;
            }
            // 1 is USB. Wi-Fi devices cannot open lockdown services here.
            if api.interface_type.is_some_and(|kind| kind(device) != 1) {
                continue;
            }
            let Some(udid) =
                Cf::owned((api.copy_identifier)(device)).and_then(|id| sys::cf_to_string(id.0))
            else {
                continue;
            };
            return Ok(Some(Phone {
                api,
                device,
                _list: list,
                udid,
            }));
        }
    }
    Ok(None)
}

/// An open lockdown session, closed when dropped.
struct Session<'a>(&'a Phone);

impl Drop for Session<'_> {
    fn drop(&mut self) {
        // SAFETY: the session and connection were opened on this device.
        unsafe {
            (self.0.api.stop_session)(self.0.device);
            (self.0.api.disconnect)(self.0.device);
        }
    }
}

impl Phone {
    /// Connect, check that the phone trusts this Mac, and start a session.
    fn session(&self, ask_to_trust: bool) -> Result<Session<'_>, Link> {
        let api = self.api;
        // SAFETY: `device` is alive (see `Phone`); the calls are made in
        // the order lockdown requires and undone on every early return.
        unsafe {
            let code = (api.connect)(self.device);
            if code != 0 {
                return Err(Link::for_error("AMDeviceConnect", code as u32));
            }
            if (api.is_paired)(self.device) != 1 {
                // Asking to pair is what makes the phone show "Trust This
                // Computer?".
                if let (true, Some(pair)) = (ask_to_trust, api.pair) {
                    pair(self.device);
                }
                (api.disconnect)(self.device);
                return Err(Link::NotPaired);
            }
            let code = (api.validate_pairing)(self.device);
            if code != 0 {
                (api.disconnect)(self.device);
                return Err(Link::for_error("AMDeviceValidatePairing", code as u32));
            }
            let code = (api.start_session)(self.device);
            if code != 0 {
                (api.disconnect)(self.device);
                return Err(Link::for_error("AMDeviceStartSession", code as u32));
            }
        }
        Ok(Session(self))
    }

    fn value(&self, domain: Option<&str>, key: &str) -> Option<Cf> {
        let domain = domain.and_then(Cf::string);
        let key = Cf::string(key)?;
        let domain_ref = domain.as_ref().map_or(std::ptr::null(), |d| d.0);
        // SAFETY: called inside a session, with CFString arguments.
        Cf::owned(unsafe { (self.api.copy_value)(self.device, domain_ref, key.0) })
    }

    /// What lockdown says about the phone right now.
    pub fn facts(&self, ask_to_trust: bool) -> DeviceFacts {
        let mut facts = DeviceFacts {
            udid: self.udid.clone(),
            name: None,
            ios_version: None,
            link: Link::Connected,
            developer_mode: None,
        };
        match self.session(ask_to_trust) {
            Err(link) => facts.link = link,
            Ok(_session) => {
                let text = |key| self.value(None, key).and_then(|v| sys::cf_to_string(v.0));
                facts.name = text("DeviceName");
                facts.ios_version = text("ProductVersion");
                facts.developer_mode = self
                    .value(Some(lockdown::AMFI_DOMAIN), lockdown::DEVELOPER_MODE_KEY)
                    .and_then(|v| sys::cf_to_bool(v.0));
            }
        }
        facts
    }

    /// Send one Developer Mode action and read the phone's answer.
    pub fn amfi(&self, action: AmfiAction) -> Result<AmfiReply, String> {
        let _session = self
            .session(false)
            .map_err(|_| "The iPhone is locked or no longer trusts this Mac.".to_string())?;
        let name = Cf::string(lockdown::AMFI_SERVICE).ok_or("out of memory")?;
        let mut connection: CFTypeRef = std::ptr::null();
        // SAFETY: inside a session; a CFString name, no options, a valid
        // out-pointer.
        let code = unsafe {
            (self.api.start_service)(self.device, name.0, std::ptr::null(), &mut connection)
        };
        let Some(connection) = Cf::owned(connection).filter(|_| code == 0) else {
            return Err(failure("Starting the Developer Mode service", code));
        };
        let result = self.exchange(&connection, &lockdown::amfi_request(action));
        // SAFETY: the connection is ours and still alive.
        unsafe { (self.api.invalidate)(connection.0) };
        result
    }

    fn exchange(&self, connection: &Cf, request: &[u8]) -> Result<AmfiReply, String> {
        // SAFETY: a live service connection and a buffer of the length passed.
        let sent = unsafe { (self.api.send)(connection.0, request.as_ptr().cast(), request.len()) };
        if sent < 0 || sent as usize != request.len() {
            return Err("Could not send to the iPhone's Developer Mode service.".into());
        }
        let mut header = [0u8; 4];
        self.read_exact(connection, &mut header)?;
        let mut body = vec![0u8; lockdown::reply_len(header)?];
        self.read_exact(connection, &mut body)?;
        let plist = sys::plist_dictionary(&body)
            .ok_or("The iPhone's Developer Mode service sent a reply Edart cannot read.")?;
        Ok(AmfiReply {
            success: sys::cf_to_bool(sys::dictionary_get(&plist, "success")),
            error: sys::cf_to_string(sys::dictionary_get(&plist, "Error")),
        })
    }

    fn read_exact(&self, connection: &Cf, buffer: &mut [u8]) -> Result<(), String> {
        let mut got = 0;
        while got < buffer.len() {
            // SAFETY: the remaining part of `buffer` is valid for writes.
            let n = unsafe {
                (self.api.receive)(
                    connection.0,
                    buffer[got..].as_mut_ptr().cast(),
                    buffer.len() - got,
                )
            };
            if n <= 0 {
                return Err("The iPhone's Developer Mode service closed the connection.".into());
            }
            got += n as usize;
        }
        Ok(())
    }
}
