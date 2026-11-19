//! How far along the USB setup is, decided from plain facts the native side
//! reports: is a phone plugged in, does it trust this Mac, is Developer Mode
//! on, are Xcode's device tools installed, do the phone's developer
//! services answer.

use serde::Serialize;

/// The oldest iOS whose developer services Edart can reach this way.
pub const MIN_IOS_MAJOR: u32 = 17;

/// What lockdown says about the cable link to the phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// The phone has not been told to trust this Mac (or said no).
    NotPaired,
    /// Trusted, but locked with a passcode since it was plugged in.
    Locked,
    /// Trusted and answering.
    Connected,
    /// Something else went wrong; the text is the framework's error.
    Failed(String),
}

impl Link {
    /// What a failed MobileDevice call means. The codes are the
    /// framework's `kAMD…Error` values.
    pub fn for_error(call: &str, code: u32) -> Link {
        match code {
            // Password protected: locked since it was plugged in.
            0xe800_001a => Link::Locked,
            // The person tapped Don't Trust, or has not answered yet.
            0xe800_0095 | 0xe800_0096 => Link::NotPaired,
            _ => Link::Failed(format!("{call} failed (0x{code:08x})")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFacts {
    pub udid: String,
    pub name: Option<String>,
    /// For example "27.3".
    pub ios_version: Option<String>,
    pub link: Link,
    /// `None` when the phone would not say.
    pub developer_mode: Option<bool>,
}

/// Where the person is in turning Developer Mode on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeveloperModeStep {
    /// Nothing asked of the phone yet.
    #[default]
    Idle,
    /// The switch now shows in Settings; the phone has a passcode, so the
    /// person has to flip it.
    Revealed,
    /// The phone accepted and is restarting, or is about to.
    Armed,
}

/// What asking the phone to turn Developer Mode on led to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeveloperModeOutcome {
    /// The phone accepted and restarts by itself.
    Restarting,
    /// The phone has a passcode: the switch now shows in Settings and the
    /// person has to flip it there.
    UseSettings,
}

impl DeveloperModeOutcome {
    pub fn step(self) -> DeveloperModeStep {
        match self {
            DeveloperModeOutcome::Restarting => DeveloperModeStep::Armed,
            DeveloperModeOutcome::UseSettings => DeveloperModeStep::Revealed,
        }
    }
}

/// Whether the phone's developer HID services can be reached.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Services {
    #[default]
    NotPrepared,
    Preparing,
    /// The Developer Disk Image is not mounted and could not be.
    DiskImageMissing(String),
    Unavailable(String),
    Ready,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsbFacts {
    /// This build can use USB at all (macOS only).
    pub supported: bool,
    /// Why Apple's MobileDevice framework could not be used, if it could not.
    pub host_error: Option<String>,
    pub device: Option<DeviceFacts>,
    pub developer_mode_step: DeveloperModeStep,
    /// Xcode's CoreDevice framework is on this Mac.
    pub core_device_installed: bool,
    pub services: Services,
}

/// The step the person is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UsbPhase {
    Unsupported,
    HostMissing,
    NoDevice,
    /// Developer Mode was accepted and the phone went away to restart.
    Restarting,
    Untrusted,
    Locked,
    OldIos,
    DeveloperModeOff,
    /// The switch is in Settings now; the person has to flip it.
    DeveloperModeManual,
    /// Restarted; the phone is asking "Turn On Developer Mode?".
    ConfirmDeveloperMode,
    XcodeMissing,
    NotPrepared,
    Preparing,
    DiskImageMissing,
    ServiceUnavailable,
    Ready,
}

/// The major version of "27.3" is 27.
pub fn ios_major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

impl UsbFacts {
    pub fn phase(&self) -> UsbPhase {
        if !self.supported {
            return UsbPhase::Unsupported;
        }
        if self.host_error.is_some() {
            return UsbPhase::HostMissing;
        }
        let Some(device) = &self.device else {
            return if self.developer_mode_step == DeveloperModeStep::Armed {
                UsbPhase::Restarting
            } else {
                UsbPhase::NoDevice
            };
        };
        match device.link {
            Link::NotPaired | Link::Failed(_) => return UsbPhase::Untrusted,
            Link::Locked => return UsbPhase::Locked,
            Link::Connected => {}
        }
        if device
            .ios_version
            .as_deref()
            .and_then(ios_major)
            .is_some_and(|major| major < MIN_IOS_MAJOR)
        {
            return UsbPhase::OldIos;
        }
        // Services that answer are proof enough, whatever the phone said
        // about Developer Mode.
        if self.services == Services::Ready {
            return UsbPhase::Ready;
        }
        if self.services == Services::Preparing {
            return UsbPhase::Preparing;
        }
        if device.developer_mode != Some(true) {
            return match self.developer_mode_step {
                DeveloperModeStep::Idle => UsbPhase::DeveloperModeOff,
                DeveloperModeStep::Revealed => UsbPhase::DeveloperModeManual,
                DeveloperModeStep::Armed => UsbPhase::ConfirmDeveloperMode,
            };
        }
        if !self.core_device_installed {
            return UsbPhase::XcodeMissing;
        }
        match self.services {
            Services::NotPrepared => UsbPhase::NotPrepared,
            Services::Preparing => UsbPhase::Preparing,
            Services::DiskImageMissing(_) => UsbPhase::DiskImageMissing,
            Services::Unavailable(_) => UsbPhase::ServiceUnavailable,
            Services::Ready => UsbPhase::Ready,
        }
    }

    /// The framework's own words for what went wrong, if the phase has any.
    pub fn error(&self) -> Option<String> {
        if let Some(e) = &self.host_error {
            return Some(e.clone());
        }
        if let Some(DeviceFacts {
            link: Link::Failed(e),
            ..
        }) = &self.device
        {
            return Some(e.clone());
        }
        match &self.services {
            Services::DiskImageMissing(e) | Services::Unavailable(e) => Some(e.clone()),
            _ => None,
        }
    }

    /// After the phone restarts with Developer Mode armed, it asks the
    /// person to confirm. Edart answers for them once, which works when the
    /// phone has no passcode; with one, the person confirms on the phone.
    pub fn should_confirm_developer_mode(&self) -> bool {
        self.developer_mode_step == DeveloperModeStep::Armed
            && self
                .device
                .as_ref()
                .is_some_and(|d| d.link == Link::Connected && d.developer_mode != Some(true))
    }

    /// Developer Mode came on: the setup step is over.
    pub fn developer_mode_done(&self) -> bool {
        self.developer_mode_step != DeveloperModeStep::Idle
            && self
                .device
                .as_ref()
                .is_some_and(|d| d.developer_mode == Some(true))
    }
}

/// Most events kept in [`UsbStatus::events`].
pub const MAX_EVENTS: usize = 80;

/// What the UI receives about the USB path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbStatus {
    pub phase: UsbPhase,
    pub device_name: Option<String>,
    pub ios_version: Option<String>,
    pub developer_mode: Option<bool>,
    /// The version of Xcode's CoreDevice framework, once it is loaded.
    pub core_device_version: Option<String>,
    pub error: Option<String>,
    /// Recent USB events, oldest first, for the Details panel.
    pub events: Vec<String>,
}

impl UsbStatus {
    pub fn new(facts: &UsbFacts, core_device_version: Option<String>, events: Vec<String>) -> Self {
        let device = facts.device.as_ref();
        UsbStatus {
            phase: facts.phase(),
            device_name: device.and_then(|d| d.name.clone()),
            ios_version: device.and_then(|d| d.ios_version.clone()),
            developer_mode: device.and_then(|d| d.developer_mode),
            core_device_version,
            error: facts.error(),
            events,
        }
    }
}

/// Add a line to an event log, dropping the oldest past [`MAX_EVENTS`].
pub fn push_event(events: &mut Vec<String>, event: String) {
    events.push(event);
    if events.len() > MAX_EVENTS {
        let extra = events.len() - MAX_EVENTS;
        events.drain(..extra);
    }
}

/// Which way a gesture goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Usb,
    Bluetooth,
}

/// USB when it is ready, else Bluetooth when a phone is listening.
pub fn route(usb_ready: bool, bluetooth_subscribed: bool) -> Result<Route, &'static str> {
    if usb_ready {
        Ok(Route::Usb)
    } else if bluetooth_subscribed {
        Ok(Route::Bluetooth)
    } else {
        Err("No iPhone is connected for control. Pair it over Bluetooth in Permissions first.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone() -> DeviceFacts {
        DeviceFacts {
            udid: "00008140-001A2B3C4D5E6F70".into(),
            name: Some("iPhone Air".into()),
            ios_version: Some("27.3".into()),
            link: Link::Connected,
            developer_mode: Some(false),
        }
    }

    fn mac() -> UsbFacts {
        UsbFacts {
            supported: true,
            core_device_installed: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_setup_walks_from_no_phone_to_ready() {
        let mut f = UsbFacts::default();
        assert_eq!(f.phase(), UsbPhase::Unsupported);
        f = mac();
        assert_eq!(f.phase(), UsbPhase::NoDevice);

        let mut d = phone();
        d.link = Link::NotPaired;
        f.device = Some(d.clone());
        assert_eq!(f.phase(), UsbPhase::Untrusted);
        d.link = Link::Locked;
        f.device = Some(d.clone());
        assert_eq!(f.phase(), UsbPhase::Locked);
        d.link = Link::Connected;
        f.device = Some(d.clone());
        assert_eq!(f.phase(), UsbPhase::DeveloperModeOff);

        f.developer_mode_step = DeveloperModeStep::Armed;
        assert_eq!(f.phase(), UsbPhase::ConfirmDeveloperMode);
        assert!(f.should_confirm_developer_mode());
        f.device = None;
        assert_eq!(f.phase(), UsbPhase::Restarting);
        assert!(!f.should_confirm_developer_mode());

        d.developer_mode = Some(true);
        f.device = Some(d);
        assert!(f.developer_mode_done());
        assert!(!f.should_confirm_developer_mode());
        f.developer_mode_step = DeveloperModeStep::Idle;
        assert_eq!(f.phase(), UsbPhase::NotPrepared);

        f.services = Services::Preparing;
        assert_eq!(f.phase(), UsbPhase::Preparing);
        f.services = Services::DiskImageMissing("not supported".into());
        assert_eq!(f.phase(), UsbPhase::DiskImageMissing);
        assert_eq!(f.error().as_deref(), Some("not supported"));
        f.services = Services::Unavailable("timed out".into());
        assert_eq!(f.phase(), UsbPhase::ServiceUnavailable);
        f.services = Services::Ready;
        assert_eq!(f.phase(), UsbPhase::Ready);
        assert_eq!(f.error(), None);
    }

    #[test]
    fn a_passcode_phone_is_sent_to_settings() {
        let mut f = mac();
        f.device = Some(phone());
        f.developer_mode_step = DeveloperModeStep::Revealed;
        assert_eq!(f.phase(), UsbPhase::DeveloperModeManual);
        assert!(!f.should_confirm_developer_mode());
    }

    #[test]
    fn an_unknown_developer_mode_counts_as_off_until_services_answer() {
        let mut f = mac();
        let mut d = phone();
        d.developer_mode = None;
        f.device = Some(d);
        assert_eq!(f.phase(), UsbPhase::DeveloperModeOff);
        f.services = Services::Ready;
        assert_eq!(f.phase(), UsbPhase::Ready);
    }

    #[test]
    fn xcode_is_asked_for_only_once_developer_mode_is_on() {
        let mut f = mac();
        f.core_device_installed = false;
        f.device = Some(phone());
        assert_eq!(f.phase(), UsbPhase::DeveloperModeOff);
        f.device.as_mut().unwrap().developer_mode = Some(true);
        assert_eq!(f.phase(), UsbPhase::XcodeMissing);
    }

    #[test]
    fn a_missing_framework_or_an_old_ios_stops_early() {
        let mut f = mac();
        f.host_error = Some("dlopen failed".into());
        f.device = Some(phone());
        assert_eq!(f.phase(), UsbPhase::HostMissing);
        assert_eq!(f.error().as_deref(), Some("dlopen failed"));

        let mut f = mac();
        let mut d = phone();
        d.ios_version = Some("16.7.2".into());
        d.developer_mode = Some(true);
        f.device = Some(d);
        assert_eq!(f.phase(), UsbPhase::OldIos);
        assert_eq!(ios_major("27.3"), Some(27));
        assert_eq!(ios_major("beta"), None);
    }

    #[test]
    fn a_failed_link_shows_its_error() {
        let mut f = mac();
        let mut d = phone();
        d.link = Link::Failed("AMDeviceStartSession failed (0xe8000025)".into());
        f.device = Some(d);
        assert_eq!(f.phase(), UsbPhase::Untrusted);
        assert!(f.error().unwrap().contains("0xe8000025"));
    }

    #[test]
    fn framework_errors_become_a_link_state() {
        assert_eq!(
            Link::for_error("AMDeviceStartSession", 0xe800_001a),
            Link::Locked
        );
        assert_eq!(
            Link::for_error("AMDeviceValidatePairing", 0xe800_0096),
            Link::NotPaired
        );
        assert_eq!(
            Link::for_error("AMDeviceConnect", 0xe800_0084),
            Link::Failed("AMDeviceConnect failed (0xe8000084)".into())
        );
    }

    #[test]
    fn the_status_serialises_in_camel_case() {
        let mut f = mac();
        f.device = Some(phone());
        let status = UsbStatus::new(&f, Some("701.2".into()), vec!["plugged in".into()]);
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["phase"], "developerModeOff");
        assert_eq!(json["deviceName"], "iPhone Air");
        assert_eq!(json["iosVersion"], "27.3");
        assert_eq!(json["developerMode"], false);
        assert_eq!(json["coreDeviceVersion"], "701.2");
        assert_eq!(json["error"], serde_json::Value::Null);
        assert_eq!(json["events"][0], "plugged in");
    }

    #[test]
    fn the_event_log_keeps_the_newest() {
        let mut events = Vec::new();
        for i in 0..(MAX_EVENTS + 3) {
            push_event(&mut events, format!("e{i}"));
        }
        assert_eq!(events.len(), MAX_EVENTS);
        assert_eq!(events[0], "e3");
    }

    #[test]
    fn gestures_prefer_usb_and_fall_back_to_bluetooth() {
        assert_eq!(route(true, true), Ok(Route::Usb));
        assert_eq!(route(true, false), Ok(Route::Usb));
        assert_eq!(route(false, true), Ok(Route::Bluetooth));
        assert!(route(false, false).is_err());
    }
}
