//! The iPhone's screen, live, over the USB cable.
//!
//! macOS can treat a plugged-in iPhone as a camera whose picture is the
//! phone's screen (it is how QuickTime records an iPhone). The device is
//! hidden until a process sets the CoreMediaIO system property
//! `kCMIOHardwarePropertyAllowScreenCaptureDevices`; after that it is an
//! ordinary `AVCaptureDevice` and frames come from an `AVCaptureSession`.
//! Nothing is installed on the phone, and it needs no Developer Mode: the
//! user taps Trust once, and allows Camera access on the Mac.
//!
//! Ported from iPhone Use's `ScreenCapture.swift` (MIT, © 2026 YueChen,
//! https://github.com/YueChen-C/iPhone-use): the CoreMediaIO switch,
//! finding the device by its "iOS Device" model among muxed capture
//! devices, the BGRA video output, and treating an all-black frame as a
//! screen that is off. Its recovery for phones that never start sending
//! (re-selecting the USB configuration) is not ported; this file restarts
//! the capture session instead.
//!
//! This is platform plumbing only. What the status means lives in
//! `edart_core::mirror`; shrinking and JPEG encoding in `edart_core::image`
//! and `edart_core::jpeg`.
//!
//! Threading: one background thread owns the capture session and checks
//! on it once a second. Frames arrive on a serial dispatch queue. Both
//! write to a mutex-guarded snapshot that any thread may read.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use edart_core::gesture::Screen;
use edart_core::image::{self, Layout, Rgb};
use edart_core::jpeg;
use edart_core::mirror::{self as core, CameraAccess, FrameFacts, MirrorFacts, MirrorStatus};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread};
use objc2_av_foundation::{
    AVAuthorizationStatus, AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceInput,
    AVCaptureOutput, AVCaptureSession, AVCaptureVideoDataOutput,
    AVCaptureVideoDataOutputSampleBufferDelegate, AVMediaTypeMuxed, AVMediaTypeVideo,
};
use objc2_core_foundation::CFString;
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::{
    kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA, CVPixelBufferGetBaseAddress,
    CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString};

/// Frames are kept at most this many pixels on the longer edge. It is what
/// the agent is sent and what gestures are measured in: large enough to
/// read small text, small enough to encode quickly.
const KEPT_EDGE: usize = 1600;
/// The live preview is smaller still; the phone frame in the window is
/// never taller than this many device pixels.
const PREVIEW_EDGE: usize = 1000;
const PREVIEW_QUALITY: u8 = 72;
const SCREENSHOT_QUALITY: u8 = 82;
/// Keep about 15 frames a second; the phone sends up to 60.
const FRAME_GAP: Duration = Duration::from_millis(66);
/// No first frame after this long: start the capture session again.
const RESTART_AFTER: Duration = Duration::from_secs(8);

#[derive(Default)]
struct Shared {
    started: bool,
    access: CameraAccess,
    device_name: Option<String>,
    capturing_since: Option<Instant>,
    frame: Option<Arc<Rgb>>,
    dark: bool,
    /// Size of the phone's own picture, before shrinking.
    source_size: Option<(usize, usize)>,
    seq: u64,
    last_kept: Option<Instant>,
    error: Option<String>,
    events: Vec<String>,
    /// The preview JPEG of frame `seq`, so a second request for the same
    /// frame does not encode it again.
    preview: Option<(u64, Arc<Vec<u8>>)>,
}

static SHARED: Mutex<Option<Shared>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
static STARTED_AT: OnceLock<Instant> = OnceLock::new();

fn with_shared<T>(f: impl FnOnce(&mut Shared) -> T) -> T {
    let mut guard = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Shared::default))
}

/// Add a line to the Details log, stamped with seconds since start.
fn log(message: impl AsRef<str>) {
    let t = STARTED_AT.get_or_init(Instant::now).elapsed().as_secs_f32();
    let line = format!("{t:7.1}s  {}", message.as_ref());
    with_shared(|s| core::push_event(&mut s.events, line));
}

/// Record a problem for the status line; log it once, not every second.
fn set_error(message: Option<String>) {
    let changed = with_shared(|s| {
        let changed = s.error != message;
        s.error.clone_from(&message);
        changed
    });
    if let (true, Some(message)) = (changed, message) {
        log(message);
    }
}

fn camera_access() -> CameraAccess {
    // SAFETY: a constant and a class method with no preconditions.
    let Some(video) = (unsafe { AVMediaTypeVideo }) else {
        return CameraAccess::Unknown;
    };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(video) } {
        AVAuthorizationStatus::Authorized => CameraAccess::Allowed,
        AVAuthorizationStatus::Denied => CameraAccess::Denied,
        AVAuthorizationStatus::Restricted => CameraAccess::Restricted,
        _ => CameraAccess::NotDetermined,
    }
}

/// True when the user already allowed Camera access, so starting will not
/// show a permission prompt.
pub fn already_allowed() -> bool {
    camera_access() == CameraAccess::Allowed
}

/// A snapshot of the mirror, safe to call from any thread.
pub fn status() -> MirrorStatus {
    let access = camera_access();
    with_shared(|s| {
        s.access = access;
        let facts = MirrorFacts {
            supported: true,
            started: s.started,
            access,
            device_name: s.device_name.clone(),
            capturing_for_ms: s.capturing_since.map(|t| t.elapsed().as_millis() as u64),
            frame: s.frame.as_ref().map(|f| FrameFacts {
                width: f.width as u32,
                height: f.height as u32,
                dark: s.dark,
            }),
            error: s.error.clone(),
        };
        MirrorStatus::new(&facts, s.seq, s.events.clone())
    })
}

/// The newest frame as a small JPEG, for the live preview.
pub fn preview_jpeg() -> Option<Vec<u8>> {
    let (frame, seq, cached) = with_shared(|s| {
        let cached = match &s.preview {
            Some((seq, jpeg)) if *seq == s.seq => Some(jpeg.clone()),
            _ => None,
        };
        (s.frame.clone(), s.seq, cached)
    });
    if let Some(jpeg) = cached {
        return Some(jpeg.to_vec());
    }
    let jpeg = Arc::new(jpeg::encode(&frame?.shrunk(PREVIEW_EDGE), PREVIEW_QUALITY)?);
    with_shared(|s| s.preview = Some((seq, jpeg.clone())));
    Some(jpeg.to_vec())
}

/// The newest frame as a JPEG for the agent, with the screen size its
/// coordinates are in. `None` unless the mirror is live.
pub fn screenshot() -> Option<(Vec<u8>, Screen)> {
    let screen = status().screen()?;
    let frame = with_shared(|s| s.frame.clone())?;
    Some((jpeg::encode(&frame, SCREENSHOT_QUALITY)?, screen))
}

/// The newest frame itself, its number, and how long ago it arrived.
/// `None` unless the mirror is live.
pub fn frame() -> Option<(Arc<Rgb>, u64, Duration)> {
    status().screen()?;
    with_shared(|s| {
        let age = s.last_kept.map(|at| at.elapsed())?;
        Some((s.frame.clone()?, s.seq, age))
    })
}

/// Switch mirroring on. Asks for Camera access the first time. Safe to
/// call again.
pub fn start() {
    with_shared(|s| s.started = true);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    log("mirror started");
    let spawned = std::thread::Builder::new()
        .name("edart-mirror".into())
        .spawn(watch);
    if let Err(e) = spawned {
        RUNNING.store(false, Ordering::SeqCst);
        set_error(Some(format!("couldn’t start the mirror: {e}")));
    }
}

/// Tell CoreMediaIO to list screen-capture devices (plugged-in iPhones)
/// to this process. Without it an iPhone never appears as a capture device.
fn allow_screen_capture_devices() -> i32 {
    #[repr(C)]
    struct CMIOObjectPropertyAddress {
        selector: u32,
        scope: u32,
        element: u32,
    }
    #[link(name = "CoreMediaIO", kind = "framework")]
    extern "C" {
        fn CMIOObjectSetPropertyData(
            object_id: u32,
            address: *const CMIOObjectPropertyAddress,
            qualifier_data_size: u32,
            qualifier_data: *const c_void,
            data_size: u32,
            data: *const c_void,
        ) -> i32;
    }
    /// `kCMIOHardwarePropertyAllowScreenCaptureDevices`, the four-character
    /// code 'yes '.
    const ALLOW_SCREEN_CAPTURE_DEVICES: u32 = u32::from_be_bytes(*b"yes ");
    /// `kCMIOObjectPropertyScopeGlobal`, 'glob'.
    const SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
    /// `kCMIOObjectPropertyElementMain`.
    const ELEMENT_MAIN: u32 = 0;
    /// `kCMIOObjectSystemObject`.
    const SYSTEM_OBJECT: u32 = 1;

    let address = CMIOObjectPropertyAddress {
        selector: ALLOW_SCREEN_CAPTURE_DEVICES,
        scope: SCOPE_GLOBAL,
        element: ELEMENT_MAIN,
    };
    let allow: u32 = 1;
    // SAFETY: the address and value outlive the call, and the sizes match
    // the property's type (UInt32).
    unsafe {
        CMIOObjectSetPropertyData(
            SYSTEM_OBJECT,
            &address,
            0,
            std::ptr::null(),
            std::mem::size_of::<u32>() as u32,
            (&raw const allow).cast(),
        )
    }
}

/// The first connected iPhone (or iPad) among the capture devices.
fn find_iphone() -> Option<Retained<AVCaptureDevice>> {
    // SAFETY: a constant and plain getters.
    let muxed = unsafe { AVMediaTypeMuxed }?;
    // The discovery-session API needs a device type that only exists from
    // macOS 14; this older call lists the same devices on every version
    // this app supports.
    #[allow(deprecated)]
    let devices = unsafe { AVCaptureDevice::devicesWithMediaType(muxed) };
    devices.iter().find(|device| unsafe {
        device.isConnected() && device.modelID().to_string().contains("iOS")
    })
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class has
    // no ivars and no Drop.
    #[unsafe(super(NSObject))]
    #[name = "EdartFrameDelegate"]
    struct FrameDelegate;

    unsafe impl NSObjectProtocol for FrameDelegate {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for FrameDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn did_output(
            &self,
            _output: &AVCaptureOutput,
            sample: &CMSampleBuffer,
            _connection: &AVCaptureConnection,
        ) {
            on_frame(sample);
        }
    }
);

impl FrameDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: NSObject's init, on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

/// A frame arrived (on the capture queue). Keep about 15 a second, shrunk
/// and converted to RGB; drop the rest.
fn on_frame(sample: &CMSampleBuffer) {
    let now = Instant::now();
    let keep = with_shared(|s| {
        let due = s.last_kept.is_none_or(|last| now - last >= FRAME_GAP);
        if due {
            s.last_kept = Some(now);
        }
        due
    });
    if !keep {
        return;
    }
    // SAFETY: the sample buffer is valid for this callback.
    let Some(buffer) = (unsafe { sample.image_buffer() }) else {
        return;
    };
    if CVPixelBufferGetPixelFormatType(&buffer) != kCVPixelFormatType_32BGRA {
        set_error(Some(
            "the iPhone sent frames in a format Edart doesn’t read".into(),
        ));
        return;
    }
    // SAFETY: lock and unlock are paired, and the bytes are only read
    // between them.
    if unsafe { CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly) } != 0 {
        return;
    }
    let width = CVPixelBufferGetWidth(&buffer);
    let height = CVPixelBufferGetHeight(&buffer);
    let stride = CVPixelBufferGetBytesPerRow(&buffer);
    let base = CVPixelBufferGetBaseAddress(&buffer) as *const u8;
    let shrunk = if base.is_null() || width == 0 || height == 0 || stride < width * 4 {
        None
    } else {
        // The last row may stop at the last pixel, without padding.
        let len = stride * (height - 1) + width * 4;
        // SAFETY: a locked 32BGRA buffer holds at least this many bytes
        // from its base address.
        let bytes = unsafe { std::slice::from_raw_parts(base, len) };
        image::shrink(bytes, width, height, stride, Layout::Bgra, KEPT_EDGE)
    };
    // SAFETY: paired with the lock above.
    unsafe { CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly) };
    let Some(frame) = shrunk else {
        return;
    };

    let dark = frame.is_dark();
    let kept_size = (frame.width, frame.height);
    let (first, resized, went_dark) = with_shared(|s| {
        let first = s.frame.is_none();
        let resized = s.source_size.is_some_and(|size| size != (width, height));
        let went_dark = !first && dark != s.dark;
        s.source_size = Some((width, height));
        s.frame = Some(Arc::new(frame));
        s.dark = dark;
        s.seq += 1;
        s.error = None;
        (first, resized, went_dark)
    });
    let shape = if width > height {
        "landscape"
    } else {
        "portrait"
    };
    if first {
        log(format!(
            "first frame: {width}x{height} {shape}, kept at {}x{}{}",
            kept_size.0,
            kept_size.1,
            if dark { " (screen is off)" } else { "" }
        ));
    } else if resized {
        log(format!("screen is now {width}x{height} {shape}"));
    }
    if went_dark {
        log(if dark {
            "screen went dark"
        } else {
            "screen is on again"
        });
    }
}

/// A running capture session. Only the watcher thread touches it.
struct Capture {
    device: Retained<AVCaptureDevice>,
    session: Retained<AVCaptureSession>,
    output: Retained<AVCaptureVideoDataOutput>,
    // The output holds its delegate weakly; these keep it and its queue
    // alive.
    _delegate: Retained<FrameDelegate>,
    _queue: DispatchRetained<DispatchQueue>,
    opened_at: Instant,
}

impl Capture {
    fn open(device: Retained<AVCaptureDevice>) -> Result<Capture, String> {
        // SAFETY: AVFoundation calls on objects this thread owns, with
        // the "can add" checks the API requires before adding.
        unsafe {
            let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
                .map_err(|e| e.localizedDescription().to_string())?;
            let session = AVCaptureSession::new();
            if !session.canAddInput(&input) {
                return Err("macOS won’t let Edart capture this iPhone".into());
            }
            session.addInput(&input);

            let output = AVCaptureVideoDataOutput::new();
            // CFString and NSString are the same object.
            let key: &CFString = kCVPixelBufferPixelFormatTypeKey;
            let key: &NSString = &*(key as *const CFString).cast::<NSString>();
            let format = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
            let format: &AnyObject = &format;
            let settings = NSDictionary::from_slices(&[key], &[format]);
            output.setVideoSettings(Some(&settings));
            output.setAlwaysDiscardsLateVideoFrames(true);
            let delegate = FrameDelegate::new();
            let queue =
                DispatchQueue::new("com.capemountco.edart.mirror", DispatchQueueAttr::SERIAL);
            output.setSampleBufferDelegate_queue(
                Some(ProtocolObject::from_ref(&*delegate)),
                Some(&queue),
            );
            if !session.canAddOutput(&output) {
                return Err("macOS won’t let Edart read frames from this iPhone".into());
            }
            session.addOutput(&output);
            session.startRunning();
            Ok(Capture {
                device,
                session,
                output,
                _delegate: delegate,
                _queue: queue,
                opened_at: Instant::now(),
            })
        }
    }

    fn close(self) {
        // SAFETY: stops delivery before the delegate is released.
        unsafe {
            self.output.setSampleBufferDelegate_queue(None, None);
            self.session.stopRunning();
        }
    }
}

fn forget_frames(s: &mut Shared) {
    s.device_name = None;
    s.capturing_since = None;
    s.frame = None;
    s.source_size = None;
    s.preview = None;
    s.dark = false;
}

/// The watcher: once a second, make the capture match the world (access
/// granted? iPhone plugged in? frames arriving?).
fn watch() {
    let mut asked = false;
    let mut allowed_devices = false;
    let mut capture: Option<Capture> = None;
    let mut last_access = CameraAccess::Unknown;
    loop {
        autoreleasepool(|_| {
            let access = camera_access();
            if access != last_access {
                log(format!("camera access: {access:?}"));
                last_access = access;
            }
            match access {
                CameraAccess::Allowed => {
                    if !allowed_devices {
                        let status = allow_screen_capture_devices();
                        log(if status == 0 {
                            "asked macOS to list plugged-in iPhones as screens".to_string()
                        } else {
                            format!("macOS refused to list iPhones as screens (error {status})")
                        });
                        allowed_devices = true;
                    }
                    capture = tend(capture.take());
                }
                CameraAccess::NotDetermined | CameraAccess::Unknown => {
                    if !asked {
                        asked = true;
                        request_access();
                    }
                }
                CameraAccess::Denied | CameraAccess::Restricted => {
                    if let Some(old) = capture.take() {
                        old.close();
                        with_shared(forget_frames);
                    }
                }
            }
        });
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn request_access() {
    // SAFETY: a constant.
    let Some(video) = (unsafe { AVMediaTypeVideo }) else {
        return;
    };
    log("asking for camera access");
    let answered = RcBlock::new(|granted: Bool| {
        log(if granted.as_bool() {
            "camera access granted"
        } else {
            "camera access refused"
        });
    });
    // SAFETY: the block matches the handler's signature and may be called
    // on any thread.
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(video, &answered) };
}

/// One check of the capture: open it when an iPhone appears, close it when
/// the iPhone goes, restart it when it never sends a picture.
fn tend(capture: Option<Capture>) -> Option<Capture> {
    match capture {
        Some(current) => {
            // SAFETY: a plain getter.
            if !unsafe { current.device.isConnected() } {
                log("iPhone unplugged");
                current.close();
                with_shared(forget_frames);
                return None;
            }
            let silent = with_shared(|s| s.frame.is_none());
            if silent && current.opened_at.elapsed() >= RESTART_AFTER {
                // Keep the status as it is ("no picture"), and try again.
                log("no picture from the iPhone yet; starting capture again");
                current.close();
                return tend(None);
            }
            Some(current)
        }
        None => {
            let device = find_iphone()?;
            // SAFETY: plain getters.
            let name = unsafe { device.localizedName() }.to_string();
            let model = unsafe { device.modelID() }.to_string();
            let fresh = with_shared(|s| s.device_name.as_deref() != Some(name.as_str()));
            if fresh {
                log(format!("found {name} ({model})"));
            }
            match Capture::open(device) {
                Ok(opened) => {
                    with_shared(|s| {
                        s.device_name = Some(name);
                        s.capturing_since.get_or_insert_with(Instant::now);
                        s.last_kept = None;
                    });
                    set_error(None);
                    Some(opened)
                }
                Err(why) => {
                    with_shared(|s| {
                        s.device_name = Some(name);
                        s.capturing_since.get_or_insert_with(Instant::now);
                    });
                    set_error(Some(format!("couldn’t capture the iPhone’s screen: {why}")));
                    None
                }
            }
        }
    }
}
