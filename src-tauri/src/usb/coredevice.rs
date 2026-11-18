//! A client of the Mac's CoreDevice service (installed by Xcode), which
//! holds the tunnel to the iPhone and hands out a connected socket for a
//! service on it. This is what `devicectl` does; it needs no entitlement
//! and leaves Xcode's own connection alone.
//!
//! Ported from iphone-use's `CoreDeviceClient.swift` (MIT, Sunghyun Kim).
//! The messages are built and read in `edart_core::usb::coredevice`; this
//! file only moves them over libxpc.

use std::ffi::{c_void, CString};
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use block2::RcBlock;
use edart_core::usb::coredevice as messages;
use edart_core::usb::xpc::XpcValue;

use super::sys::{self, Library, XpcObject};
use super::xpc::{describe, from_xpc, is_connection, is_dictionary, is_fd, to_xpc, Xpc};

const CHECK_IN_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(20);

fn binary_path() -> String {
    format!("{}/Versions/A/CoreDevice", messages::FRAMEWORK_DIR)
}

/// Whether Xcode's CoreDevice framework is on this Mac.
pub fn installed() -> bool {
    std::path::Path::new(&binary_path()).exists()
}

/// Load the framework. The CoreDevice service lives inside it, and macOS
/// only finds a service inside a framework once that framework is loaded.
fn library() -> Result<&'static Library, String> {
    static LIBRARY: OnceLock<Result<Library, String>> = OnceLock::new();
    LIBRARY
        .get_or_init(|| Library::open(&binary_path()))
        .as_ref()
        .map_err(Clone::clone)
}

/// The installed framework's version, which the service expects its
/// clients to state.
pub fn version() -> String {
    let path = format!(
        "{}/Versions/A/Resources/Info.plist",
        messages::FRAMEWORK_DIR
    );
    let Ok(bytes) = std::fs::read(path) else {
        return messages::FALLBACK_VERSION.into();
    };
    std::str::from_utf8(&bytes)
        .ok()
        .and_then(messages::bundle_version)
        .or_else(|| {
            // A binary property list.
            let plist = sys::plist_dictionary(&bytes)?;
            sys::cf_to_string(sys::dictionary_get(&plist, "CFBundleVersion"))
        })
        .unwrap_or_else(|| messages::FALLBACK_VERSION.into())
}

/// iPhone Use registers the framework's services by hand through these two
/// private functions. iphone-use, on current Xcode, does not need to. They
/// are tried only after a plain connection was refused, so a version where
/// they are gone or different is never worse off for them.
fn register_services(library: &Library) -> bool {
    let (Some(add), Some(init)) = (
        library.symbol(c"_coredevice_xpc_add_bundle"),
        library.symbol(c"_coredevice_xpc_init_services"),
    ) else {
        return false;
    };
    let Some(class) = objc2::runtime::AnyClass::get(c"NSBundle") else {
        return false;
    };
    let path = objc2_foundation::NSString::from_str(messages::FRAMEWORK_DIR);
    // SAFETY: +[NSBundle bundleWithPath:] takes an NSString and returns an
    // autoreleased bundle or nil; the two functions take a bundle and
    // nothing, as iPhone Use calls them.
    unsafe {
        let bundle: *mut objc2::runtime::AnyObject =
            objc2::msg_send![class, bundleWithPath: &*path];
        if bundle.is_null() {
            return false;
        }
        let add: unsafe extern "C" fn(*mut objc2::runtime::AnyObject) = std::mem::transmute(add);
        let init: unsafe extern "C" fn() = std::mem::transmute(init);
        add(bundle);
        init();
    }
    true
}

fn random_bytes<const N: usize>() -> [u8; N] {
    use std::io::Read;
    let mut bytes = [0u8; N];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_ok();
    if !filled {
        // Uniqueness is all these are for; the clock will do.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (nanos >> ((i % 16) * 8)) as u8 ^ (i as u8).wrapping_mul(31);
        }
    }
    bytes
}

pub fn ping_token() -> [u8; 8] {
    random_bytes()
}

fn random_uuid() -> [u8; 16] {
    edart_core::usb::xpc::uuid_v4(random_bytes())
}

/// Peer connections opened to the assertion's listener. They must stay
/// alive for the assertion to hold.
#[derive(Default)]
struct Peers(Mutex<Vec<Xpc>>);

/// A usage assertion on one iPhone: while this lives, CoreDevice keeps the
/// tunnel up and hands out sockets. Dropping it gives the tunnel back.
pub struct Session {
    queue: *mut c_void,
    connection: Xpc,
    listener: Xpc,
    peers: Arc<Peers>,
    /// The service went away (Xcode updated, the daemon restarted).
    dead: Arc<AtomicBool>,
    device_identifier: String,
    version: String,
}

// SAFETY: the libxpc objects are thread-safe, and the queue is only passed
// back to libxpc.
unsafe impl Send for Session {}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: cancelling is always allowed; the wrappers release after.
        unsafe {
            sys::xpc_connection_cancel(self.listener.0);
            for peer in self
                .peers
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain(..)
            {
                sys::xpc_connection_cancel(peer.0);
            }
            sys::xpc_connection_cancel(self.connection.0);
        }
    }
}

/// Why a session could not be opened.
pub enum OpenError {
    /// Xcode's CoreDevice framework is missing or would not load.
    NoFramework(String),
    Failed(String),
}

impl Session {
    /// Check in with the service, find the phone, and take the assertion.
    pub fn open(udid: &str, log: &dyn Fn(String)) -> Result<Session, OpenError> {
        let library = library().map_err(OpenError::NoFramework)?;
        let version = version();
        log(format!("CoreDevice {version} loaded"));
        match Self::connect(udid, &version) {
            Err(ConnectError::NoService(why)) => {
                log(format!(
                    "CoreDevice service not found ({why}); registering it"
                ));
                if !register_services(library) {
                    return Err(OpenError::Failed(format!(
                        "The CoreDevice service did not answer ({why}). Open Xcode once so it finishes installing its components."
                    )));
                }
                Self::connect(udid, &version).map_err(|e| OpenError::Failed(e.into_text()))
            }
            other => other.map_err(|e| OpenError::Failed(e.into_text())),
        }
    }

    fn connect(udid: &str, version: &str) -> Result<Session, ConnectError> {
        let name = CString::new(messages::SERVICE_NAME).unwrap_or_default();
        // SAFETY: valid C strings; a null attribute makes a serial queue.
        let (queue, connection) = unsafe {
            let queue =
                sys::dispatch_queue_create(c"app.edart.coredevice".as_ptr(), std::ptr::null());
            (queue, Xpc(sys::xpc_connection_create(name.as_ptr(), queue)))
        };
        if connection.0.is_null() {
            return Err(ConnectError::NoService("no connection".into()));
        }

        // The device list arrives as an event on the connection, not as the
        // reply to the check-in.
        let (events, inbox) = mpsc::channel::<Result<XpcValue, String>>();
        let checked_in = Arc::new(AtomicBool::new(false));
        let dead = Arc::new(AtomicBool::new(false));
        let handler = {
            let (checked_in, dead) = (Arc::clone(&checked_in), Arc::clone(&dead));
            RcBlock::new(move |event: XpcObject| {
                if !is_dictionary(event) {
                    dead.store(true, Ordering::SeqCst);
                    let _ = events.send(Err(describe(event)));
                } else if !checked_in.load(Ordering::SeqCst) {
                    let _ = events.send(Ok(from_xpc(event)));
                }
            })
        };
        let ignore_reply = RcBlock::new(|_reply: XpcObject| {});
        let check_in = to_xpc(&messages::check_in_request(random_uuid()));
        // SAFETY: a live connection, queue and message; libxpc copies the
        // blocks.
        unsafe {
            sys::xpc_connection_set_event_handler(connection.0, &handler);
            sys::xpc_connection_activate(connection.0);
            sys::xpc_connection_send_message_with_reply(
                connection.0,
                check_in.0,
                queue,
                &ignore_reply,
            );
        }

        let deadline = Instant::now() + CHECK_IN_TIMEOUT;
        let device_identifier = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match inbox.recv_timeout(left) {
                Ok(Ok(message)) => {
                    if let Some(snapshots) = messages::check_in_snapshots(&message) {
                        break messages::device_identifier(snapshots, udid);
                    }
                }
                Ok(Err(why)) => {
                    // SAFETY: cancelling a live connection.
                    unsafe { sys::xpc_connection_cancel(connection.0) };
                    return Err(ConnectError::NoService(why));
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    // SAFETY: as above.
                    unsafe { sys::xpc_connection_cancel(connection.0) };
                    return Err(ConnectError::Other(
                        "The CoreDevice service did not answer the check-in.".into(),
                    ));
                }
            }
        };
        checked_in.store(true, Ordering::SeqCst);
        let Some(device_identifier) = device_identifier else {
            // SAFETY: as above.
            unsafe { sys::xpc_connection_cancel(connection.0) };
            return Err(ConnectError::Other(
                "Xcode’s device service does not list this iPhone. Unlock it, replug the cable and try again.".into(),
            ));
        };

        // SAFETY: an anonymous listener on the same queue.
        let listener = Xpc(unsafe { sys::xpc_connection_create(std::ptr::null(), queue) });
        let mut session = Session {
            queue,
            connection,
            listener,
            peers: Arc::new(Peers::default()),
            dead,
            device_identifier,
            version: version.to_string(),
        };
        session.acquire_assertion()?;
        Ok(session)
    }

    /// Ask for trusted connectivity and a power assertion, as `devicectl`
    /// does before any action. The action's reply only comes when the
    /// assertion ends; that it took hold is announced on our listener.
    fn acquire_assertion(&mut self) -> Result<(), ConnectError> {
        let (outcome, inbox) = mpsc::channel::<Result<(), String>>();
        let on_peer = {
            let (peers, outcome) = (Arc::clone(&self.peers), outcome.clone());
            RcBlock::new(move |peer: XpcObject| {
                if !is_connection(peer) {
                    return;
                }
                let outcome = outcome.clone();
                let on_message = RcBlock::new(move |message: XpcObject| {
                    if is_dictionary(message)
                        && messages::is_assertion_fulfilled(&from_xpc(message))
                    {
                        let _ = outcome.send(Ok(()));
                    }
                });
                // SAFETY: `peer` is a live connection for this call; it is
                // retained to outlive it.
                unsafe {
                    sys::xpc_connection_set_event_handler(peer, &on_message);
                    peers
                        .0
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(Xpc(sys::xpc_retain(peer)));
                    sys::xpc_connection_activate(peer);
                }
            })
        };
        let request = to_xpc(&messages::action_request(
            messages::ACTION_ACQUIRE_ASSERTION,
            &self.device_identifier,
            &self.version,
            random_uuid(),
            messages::assertion_input(),
        ));
        let on_reply = RcBlock::new(move |reply: XpcObject| {
            // A reply before fulfilment means it was refused, or the
            // service went away.
            let why = if is_dictionary(reply) {
                messages::reply_error(&from_xpc(reply))
                    .unwrap_or_else(|| "the assertion ended early".into())
            } else {
                describe(reply)
            };
            let _ = outcome.send(Err(why));
        });
        // SAFETY: live objects throughout; the endpoint is retained by the
        // dictionary it is put in.
        unsafe {
            sys::xpc_connection_set_event_handler(self.listener.0, &on_peer);
            sys::xpc_connection_activate(self.listener.0);
            let input = sys::xpc_dictionary_get_value(request.0, c"CoreDevice.input".as_ptr());
            let endpoint = Xpc(sys::xpc_endpoint_create(self.listener.0));
            sys::xpc_dictionary_set_value(input, c"endpoint".as_ptr(), endpoint.0);
            sys::xpc_connection_send_message_with_reply(
                self.connection.0,
                request.0,
                self.queue,
                &on_reply,
            );
        }
        match inbox.recv_timeout(ACTION_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(why)) => Err(ConnectError::Other(format!(
                "Xcode’s device service would not connect to the iPhone: {why}"
            ))),
            Err(_) => Err(ConnectError::Other(
                "Xcode’s device service did not connect to the iPhone in time. Unlock the iPhone and try again.".into(),
            )),
        }
    }

    /// Whether the service connection is still up.
    pub fn alive(&self) -> bool {
        !self.dead.load(Ordering::SeqCst)
    }

    /// A connected socket to one service on the phone. RemoteXPC services
    /// still need their handshake on top.
    pub fn socket(&self, service: &str) -> Result<OwnedFd, String> {
        let request = to_xpc(&messages::action_request(
            messages::ACTION_CREATE_SOCKET,
            &self.device_identifier,
            &self.version,
            random_uuid(),
            messages::socket_input(service),
        ));
        let (outcome, inbox) = mpsc::channel::<Result<OwnedFd, String>>();
        let on_reply = RcBlock::new(move |reply: XpcObject| {
            let result = if !is_dictionary(reply) {
                Err(describe(reply))
            } else if let Some(why) = messages::reply_error(&from_xpc(reply)) {
                Err(why)
            } else {
                // SAFETY: `reply` is a live dictionary; the descriptor is
                // duplicated, so it outlives the reply.
                unsafe {
                    let output =
                        sys::xpc_dictionary_get_value(reply, c"CoreDevice.output".as_ptr());
                    let descriptor = if is_dictionary(output) {
                        sys::xpc_dictionary_get_value(output, c"fileDescriptor".as_ptr())
                    } else {
                        std::ptr::null_mut()
                    };
                    let fd = if is_fd(descriptor) {
                        sys::xpc_fd_dup(descriptor)
                    } else {
                        -1
                    };
                    if fd >= 0 {
                        Ok(OwnedFd::from_raw_fd(fd))
                    } else {
                        Err("the reply carried no socket".to_string())
                    }
                }
            };
            // If nobody is waiting any more, the socket closes here.
            let _ = outcome.send(result);
        });
        // SAFETY: live connection, message and queue.
        unsafe {
            sys::xpc_connection_send_message_with_reply(
                self.connection.0,
                request.0,
                self.queue,
                &on_reply,
            );
        }
        inbox
            .recv_timeout(ACTION_TIMEOUT)
            .map_err(|_| "no answer from Xcode’s device service".to_string())?
    }
}

enum ConnectError {
    /// The service itself could not be reached.
    NoService(String),
    Other(String),
}

impl ConnectError {
    fn into_text(self) -> String {
        match self {
            ConnectError::NoService(why) => format!(
                "The CoreDevice service did not answer ({why}). Open Xcode once so it finishes installing its components."
            ),
            ConnectError::Other(text) => text,
        }
    }
}
