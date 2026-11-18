//! Control of the iPhone over the USB cable: platform plumbing only.
//!
//! What to send lives in `edart_core::usb`. Here those messages go to
//! Apple's frameworks: MobileDevice for lockdown ([`mobiledevice`]) and
//! Xcode's CoreDevice service for sockets to the phone's developer services
//! ([`coredevice`]). Both are opened at run time, so a Mac without them
//! gets a message in the setup screen, not a crash.
//!
//! Threading: one worker thread owns everything. MobileDevice announces
//! devices on the run loop of the thread that subscribed, and one gesture
//! at a time is all a phone can take. Other threads read [`status`] (a
//! mutex-guarded snapshot) and send the worker commands.

mod coredevice;
mod mobiledevice;
mod sys;
mod xpc;

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use edart_core::usb::coredevice::means_disk_image_missing;
use edart_core::usb::hid;
use edart_core::usb::lockdown::{amfi_result, AmfiAction, AmfiError};
use edart_core::usb::plan::{self, UsbStep};
use edart_core::usb::readiness::{
    push_event, DeveloperModeOutcome, DeveloperModeStep, Link, Services, UsbFacts, UsbPhase,
    UsbStatus,
};
use edart_core::usb::remotexpc::Connection;

use coredevice::{OpenError, Session};

/// How often the worker looks at the cable when nothing else is going on.
const POLL: Duration = Duration::from_secs(2);
/// How long a socket to the phone may stay silent.
const SOCKET_TIMEOUT: Duration = Duration::from_secs(10);
/// How long mounting the Developer Disk Image may take.
const MOUNT_TIMEOUT: Duration = Duration::from_secs(90);

struct Shared {
    facts: UsbFacts,
    core_device_version: Option<String>,
    events: Vec<String>,
}

static SHARED: Mutex<Option<Shared>> = Mutex::new(None);
static STARTED: OnceLock<Instant> = OnceLock::new();

fn with_shared<T>(f: impl FnOnce(&mut Shared) -> T) -> T {
    let mut guard = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(|| Shared {
        facts: UsbFacts {
            supported: true,
            core_device_installed: coredevice::installed(),
            ..Default::default()
        },
        core_device_version: None,
        events: Vec::new(),
    }))
}

/// Add a line to the Details log, stamped with seconds since start.
fn log(message: impl AsRef<str>) {
    let t = STARTED.get_or_init(Instant::now).elapsed().as_secs_f32();
    let line = format!("{t:7.1}s  {}", message.as_ref());
    with_shared(|s| push_event(&mut s.events, line));
}

/// Change the facts, and log the phase if that changed it.
fn update(f: impl FnOnce(&mut UsbFacts)) {
    let changed = with_shared(|s| {
        let before = s.facts.phase();
        f(&mut s.facts);
        let after = s.facts.phase();
        (before != after).then_some(after)
    });
    if let Some(phase) = changed {
        log(format!("now: {phase:?}"));
    }
}

fn facts() -> UsbFacts {
    with_shared(|s| s.facts.clone())
}

/// A snapshot of the USB setup, safe to call from any thread.
pub fn status() -> UsbStatus {
    with_shared(|s| UsbStatus::new(&s.facts, s.core_device_version.clone(), s.events.clone()))
}

pub fn is_ready() -> bool {
    with_shared(|s| s.facts.phase() == UsbPhase::Ready)
}

enum Command {
    EnableDeveloperMode(Sender<Result<DeveloperModeOutcome, String>>),
    Prepare(Sender<Result<(), String>>),
    Perform(Vec<UsbStep>, Sender<Result<(), String>>),
}

static WORKER: OnceLock<Sender<Command>> = OnceLock::new();

/// Start watching the cable. Safe to call more than once.
pub fn start() {
    worker();
}

fn worker() -> &'static Sender<Command> {
    WORKER.get_or_init(|| {
        let (sender, inbox) = mpsc::channel::<Command>();
        let spawned = std::thread::Builder::new()
            .name("edart-usb".into())
            .spawn(move || {
                log("watching for an iPhone on USB");
                let mut worker = Worker::default();
                loop {
                    worker.refresh();
                    match inbox.recv_timeout(POLL) {
                        Ok(Command::EnableDeveloperMode(reply)) => {
                            let _ = reply.send(worker.enable_developer_mode());
                        }
                        Ok(Command::Prepare(reply)) => {
                            let _ = reply.send(worker.prepare());
                        }
                        Ok(Command::Perform(steps, reply)) => {
                            let _ = reply.send(worker.perform(&steps));
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            });
        if let Err(e) = spawned {
            update(|f| f.host_error = Some(format!("could not start the USB thread: {e}")));
        }
        sender
    })
}

fn ask<T>(command: impl FnOnce(Sender<Result<T, String>>) -> Command) -> Result<T, String> {
    let (reply, answer) = mpsc::channel();
    worker()
        .send(command(reply))
        .map_err(|_| "The USB thread has stopped. Quit and reopen Edart.".to_string())?;
    answer
        .recv()
        .map_err(|_| "The USB thread stopped while working. Quit and reopen Edart.".to_string())?
}

/// Ask the iPhone to turn Developer Mode on. Blocks until it answers.
pub fn enable_developer_mode() -> Result<DeveloperModeOutcome, String> {
    ask(Command::EnableDeveloperMode)
}

/// Connect to the iPhone's developer services. Blocks; can take a minute
/// the first time, while the Developer Disk Image is mounted.
pub fn prepare() -> Result<(), String> {
    ask(Command::Prepare)
}

/// Perform a planned gesture. Blocks until the last report is delivered.
pub fn perform(steps: Vec<UsbStep>) -> Result<(), String> {
    ask(|reply| Command::Perform(steps, reply))
}

#[derive(Default)]
struct Worker {
    session: Option<Session>,
    /// The phone Edart already asked to trust this Mac, so it asks once.
    asked_to_trust: Option<String>,
    /// The phone went away after Developer Mode was armed: it restarted.
    restarted: bool,
    /// The post-restart confirmation was already sent.
    confirmed: bool,
}

type Socket = Connection<TcpStream>;

impl Worker {
    /// Look at the cable and bring the facts up to date.
    fn refresh(&mut self) {
        let phone = match mobiledevice::find_phone() {
            Ok(phone) => phone,
            Err(why) => {
                update(|f| f.host_error = Some(why));
                return;
            }
        };
        let Some(phone) = phone else {
            if facts().device.is_some() {
                log("iPhone unplugged");
            }
            self.session = None;
            self.asked_to_trust = None;
            self.restarted |= facts().developer_mode_step == DeveloperModeStep::Armed;
            update(|f| {
                f.host_error = None;
                f.device = None;
                f.services = Services::NotPrepared;
            });
            return;
        };

        let known = facts();
        let same = known.device.as_ref().is_some_and(|d| d.udid == phone.udid);
        if same && known.services == Services::Ready {
            // Everything works; do not open a lockdown session every two
            // seconds just to look. A dead service connection is found here.
            if self.session.as_ref().is_some_and(|s| !s.alive()) {
                log("Xcode’s device service went away; prepare again");
                self.session = None;
                update(|f| f.services = Services::NotPrepared);
            }
            return;
        }
        if !same {
            log(format!("iPhone plugged in ({})", short(&phone.udid)));
            self.session = None;
        }

        let ask = self.asked_to_trust.as_deref() != Some(&phone.udid);
        let device = phone.facts(ask);
        if device.link == Link::NotPaired && ask {
            self.asked_to_trust = Some(phone.udid.clone());
            log("asked the iPhone to trust this Mac");
        }
        if known.device.as_ref() != Some(&device) {
            log(format!(
                "{} · iOS {} · {:?} · Developer Mode {}",
                device.name.as_deref().unwrap_or("iPhone"),
                device.ios_version.as_deref().unwrap_or("?"),
                device.link,
                match device.developer_mode {
                    Some(true) => "on",
                    Some(false) => "off",
                    None => "unknown",
                }
            ));
        }
        update(|f| {
            f.host_error = None;
            f.core_device_installed = coredevice::installed();
            if !same {
                f.services = Services::NotPrepared;
            }
            f.device = Some(device);
        });

        let now = facts();
        if now.developer_mode_done() {
            log("Developer Mode is on");
            self.restarted = false;
            self.confirmed = false;
            update(|f| f.developer_mode_step = DeveloperModeStep::Idle);
        } else if now.should_confirm_developer_mode() && self.restarted && !self.confirmed {
            // Works on a phone without a passcode. With one, the phone asks
            // the person instead, and this is refused.
            self.confirmed = true;
            match phone.amfi(AmfiAction::Accept).map(|r| amfi_result(&r)) {
                Ok(Ok(())) => log("confirmed Developer Mode after the restart"),
                Ok(Err(why)) => log(format!("confirming Developer Mode: {why}")),
                Err(why) => log(format!("confirming Developer Mode: {why}")),
            }
        }
    }

    fn connected_phone(&self) -> Result<mobiledevice::Phone, String> {
        let phone = mobiledevice::find_phone()?
            .ok_or("No iPhone is plugged in. Connect it with a cable.")?;
        match facts().device.map(|d| d.link) {
            Some(Link::Connected) => Ok(phone),
            Some(Link::Locked) => Err("Unlock your iPhone first.".into()),
            _ => Err("Unlock your iPhone and tap Trust first.".into()),
        }
    }

    fn enable_developer_mode(&mut self) -> Result<DeveloperModeOutcome, String> {
        let phone = self.connected_phone()?;
        log("asking the iPhone to show Developer Mode");
        let reveal = phone.amfi(AmfiAction::Reveal).inspect_err(|e| log(e))?;
        if let Err(why) = amfi_result(&reveal) {
            log(format!("show Developer Mode: {why}"));
            return Err(format!(
                "Could not show Developer Mode on the iPhone: {why}."
            ));
        }
        log("Developer Mode now shows in Settings › Privacy & Security");

        log("asking the iPhone to turn Developer Mode on");
        let arm = phone.amfi(AmfiAction::Arm).inspect_err(|e| log(e))?;
        let outcome = match amfi_result(&arm) {
            Ok(()) => {
                log("accepted: the iPhone restarts now");
                DeveloperModeOutcome::Restarting
            }
            Err(AmfiError::PasscodeSet) => {
                log("the iPhone has a passcode: turn Developer Mode on in Settings");
                DeveloperModeOutcome::UseSettings
            }
            Err(AmfiError::Refused(why)) => {
                log(format!(
                    "turn on Developer Mode: refused ({why}); use Settings"
                ));
                DeveloperModeOutcome::UseSettings
            }
        };
        self.restarted = false;
        self.confirmed = false;
        update(|f| f.developer_mode_step = outcome.step());
        Ok(outcome)
    }

    fn prepare(&mut self) -> Result<(), String> {
        let phone = self.connected_phone()?;
        self.session = None;
        update(|f| f.services = Services::Preparing);
        log("connecting to the iPhone’s developer services");
        let result = self.open_services(&phone.udid);
        match &result {
            Ok(()) => log("developer services ready: touch and keyboard available"),
            Err(services) => log(format!("developer services: {services:?}")),
        }
        let services = result.as_ref().err().cloned().unwrap_or(Services::Ready);
        update(|f| {
            f.core_device_installed = coredevice::installed();
            f.services = services;
        });
        result.map_err(|services| match services {
            Services::DiskImageMissing(why) | Services::Unavailable(why) => why,
            _ => "The iPhone’s developer services are not available.".into(),
        })
    }

    /// Take the usage assertion, then prove the HID service answers by
    /// opening it and getting a PING back.
    fn open_services(&mut self, udid: &str) -> Result<(), Services> {
        let session = match Session::open(udid, &|line| log(line)) {
            Ok(session) => session,
            Err(OpenError::NoFramework(why)) => {
                log(&why);
                return Err(Services::Unavailable(if coredevice::installed() {
                    format!("macOS would not load Xcode’s device framework: {why}")
                } else {
                    "Xcode is not installed. Install Xcode from the App Store and open it once."
                        .into()
                }));
            }
            Err(OpenError::Failed(why)) => return Err(Services::Unavailable(why)),
        };
        with_shared(|s| s.core_device_version = Some(coredevice::version()));
        log("connected to the iPhone through Xcode’s device service");

        let fd = match session.socket(hid::UNIVERSAL_HID_SERVICE) {
            Ok(fd) => fd,
            Err(why) if means_disk_image_missing(&why) => {
                // The HID services live on the Developer Disk Image, which
                // is unmounted after every restart of the phone. Asking
                // devicectl about them makes CoreDevice mount it.
                log(format!(
                    "no HID service yet ({why}); mounting the Developer Disk Image"
                ));
                match mount_disk_image(udid) {
                    Ok(()) => log("devicectl finished"),
                    Err(e) => log(format!("devicectl: {e}")),
                }
                session.socket(hid::UNIVERSAL_HID_SERVICE).map_err(|why| {
                    Services::DiskImageMissing(format!(
                        "The iPhone’s Developer Disk Image is not mounted ({why}). Open Xcode, choose Window › Devices and Simulators, select the iPhone and wait until it is ready."
                    ))
                })?
            }
            Err(why) => return Err(Services::Unavailable(why)),
        };
        let mut socket = open_socket(fd).map_err(Services::Unavailable)?;
        socket
            .ping(coredevice::ping_token())
            .map_err(|e| Services::Unavailable(e.to_string()))?;
        close(socket);
        self.session = Some(session);
        Ok(())
    }

    /// A socket to a service, reconnecting to CoreDevice once if the
    /// session went stale (the phone was locked, replugged, restarted).
    fn socket(&mut self, service: &str) -> Result<Socket, String> {
        let udid = facts()
            .device
            .map(|d| d.udid)
            .ok_or("No iPhone is plugged in.")?;
        let first = match &self.session {
            Some(session) => session.socket(service).and_then(open_socket),
            None => Err("not connected".into()),
        };
        let why = match first {
            Ok(socket) => return Ok(socket),
            Err(why) => why,
        };
        log(format!("{service}: {why}; reconnecting"));
        self.session = None;
        let retry = Session::open(&udid, &|line| log(line))
            .map_err(|e| match e {
                OpenError::NoFramework(why) | OpenError::Failed(why) => why,
            })
            .and_then(|session| {
                let socket = session.socket(service).and_then(open_socket)?;
                self.session = Some(session);
                Ok(socket)
            });
        retry.inspect_err(|why| {
            log(format!("{service}: {why}"));
            let state = if means_disk_image_missing(why) {
                Services::NotPrepared
            } else {
                Services::Unavailable(why.clone())
            };
            update(|f| f.services = state);
        })
    }

    fn perform(&mut self, steps: &[UsbStep]) -> Result<(), String> {
        if facts().phase() != UsbPhase::Ready {
            return Err("The iPhone is not ready over USB yet.".into());
        }
        let needs = plan::needs(steps);
        // A fresh connection per gesture: the phone builds its touchscreen
        // and keyboard for each connection, and a long-held one can stop
        // delivering while still answering PINGs.
        let mut reports = match needs.reports {
            true => Some(self.socket(hid::UNIVERSAL_HID_SERVICE)?),
            false => None,
        };
        let mut buttons = match needs.buttons {
            true => Some(
                self.socket(hid::INDIGO_SERVICE)
                    .map_err(|why| format!("The iPhone’s button service did not open: {why}"))?,
            ),
            false => None,
        };
        if reports.is_some() {
            // Reports sent before the phone has built the surfaces are
            // dropped without an error.
            std::thread::sleep(Duration::from_millis(hid::SURFACE_SETTLE_MS));
        }

        let mut done = 0;
        let mut result = Ok(());
        for step in steps {
            if let Err(e) = send_step(step, &mut reports, &mut buttons) {
                result = Err(format!("The iPhone stopped listening: {e}"));
                break;
            }
            done += 1;
        }
        if result.is_err() {
            // Never leave a finger or a key down.
            for step in plan::release_after(&steps[..done]) {
                let _ = send_step(&step, &mut reports, &mut buttons);
            }
        }
        // Reports get no reply. A PING answered after them shows the phone
        // read them all; closing before that could discard them.
        for socket in [&mut reports, &mut buttons].into_iter().flatten() {
            let pinged = socket
                .ping(coredevice::ping_token())
                .map_err(|e| format!("The iPhone did not confirm the gesture: {e}"));
            result = result.and(pinged);
        }
        if buttons.is_some() {
            // Closing sooner cancels a Home press.
            std::thread::sleep(Duration::from_millis(hid::BUTTON_SETTLE_MS));
        }
        for socket in [reports, buttons].into_iter().flatten() {
            close(socket);
        }
        if let Err(why) = &result {
            log(why);
        }
        result
    }
}

fn send_step(
    step: &UsbStep,
    reports: &mut Option<Socket>,
    buttons: &mut Option<Socket>,
) -> Result<(), String> {
    // SAFETY: a system clock read with a valid clock id.
    let now = || unsafe { sys::clock_gettime_nsec_np(sys::CLOCK_UPTIME_RAW) };
    let (socket, message) = match step {
        UsbStep::WaitMs(ms) => {
            std::thread::sleep(Duration::from_millis(*ms));
            return Ok(());
        }
        UsbStep::Touch { down, x, y } => (
            reports,
            hid::report_message(hid::TOUCH_SURFACE, hid::touch_report(*down, *x, *y, now())),
        ),
        UsbStep::Keys(usages) => (
            reports,
            hid::report_message(hid::KEYBOARD_SURFACE, hid::keyboard_report(usages, now())),
        ),
        UsbStep::Button { usage, state } => (buttons, hid::button_message(*usage, *state)),
    };
    socket
        .as_mut()
        .ok_or("no connection")?
        .send(&message)
        .map_err(|e| e.to_string())
}

/// Turn a socket from CoreDevice into a RemoteXPC connection.
fn open_socket(fd: std::os::fd::OwnedFd) -> Result<Socket, String> {
    let stream = TcpStream::from(fd);
    // CoreDevice hands the socket over non-blocking; left that way every
    // read fails at once and looks like a timeout.
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(SOCKET_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SOCKET_TIMEOUT));
    Connection::open(stream).map_err(|e| e.to_string())
}

/// Close at once. A polite half-close makes the phone's HID service hold
/// the connection and delay the next one by seconds.
fn close<S: Read + Write + Into<TcpStream>>(socket: Connection<S>) {
    let stream: TcpStream = socket.into_inner().into();
    let _ = stream.shutdown(Shutdown::Both);
}

/// Have `devicectl` (part of Xcode) query the phone's Developer Disk Image
/// services, which makes CoreDevice mount the image.
fn mount_disk_image(udid: &str) -> Result<(), String> {
    use std::process::{Command as Process, Stdio};
    let bundled = format!(
        "{}/Versions/A/Resources/bin/devicectl",
        edart_core::usb::coredevice::FRAMEWORK_DIR
    );
    let mut command = if std::path::Path::new(&bundled).exists() {
        Process::new(bundled)
    } else {
        let mut xcrun = Process::new("/usr/bin/xcrun");
        xcrun.arg("devicectl");
        xcrun
    };
    let mut child = command
        .args(["device", "info", "ddiServices", "--device", udid, "-q"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run devicectl: {e}"))?;
    let deadline = Instant::now() + MOUNT_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("devicectl ended with {status}")),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("devicectl did not finish in time".into());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// The first 8 characters of a UDID: enough to tell phones apart in the
/// log without printing the whole identifier.
fn short(udid: &str) -> String {
    udid.chars().take(8).collect()
}
