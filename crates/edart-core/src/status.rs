//! What the UI shows about the Bluetooth link, and the one-line summary of it.

use serde::Serialize;

/// The state of the Mac's Bluetooth keyboard-and-mouse role.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BluetoothStatus {
    /// Whether this build can act as a Bluetooth keyboard at all (macOS only).
    pub supported: bool,
    /// CoreBluetooth's state: unknown, resetting, unsupported, unauthorized,
    /// poweredOff or poweredOn.
    pub state: String,
    /// All services were added (or refused) and advertising was attempted.
    pub published: bool,
    pub advertising: bool,
    /// An iPhone is listening to at least one input report.
    pub subscribed: bool,
    /// Services macOS refused, by UUID.
    pub failed_services: Vec<String>,
    /// The last error worth showing, if any.
    pub error: Option<String>,
}

/// The coarse step the user is on, for the UI's headline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Unsupported,
    NotStarted,
    NeedsPermission,
    BluetoothOff,
    Starting,
    Broken,
    WaitingForPhone,
    Connected,
}

impl BluetoothStatus {
    pub fn phase(&self) -> Phase {
        if !self.supported || self.state == "unsupported" {
            return Phase::Unsupported;
        }
        match self.state.as_str() {
            "notStarted" => return Phase::NotStarted,
            "unauthorized" => return Phase::NeedsPermission,
            "poweredOff" => return Phase::BluetoothOff,
            "poweredOn" => {}
            _ => return Phase::Starting,
        }
        if self.failed_services.iter().any(|s| s.contains("1812")) {
            return Phase::Broken;
        }
        if self.subscribed {
            Phase::Connected
        } else if self.published && self.advertising {
            Phase::WaitingForPhone
        } else if self.published && self.error.is_some() {
            Phase::Broken
        } else {
            Phase::Starting
        }
    }
}

/// What the UI receives: the raw status plus the phase decided here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusView {
    #[serde(flatten)]
    pub status: BluetoothStatus,
    pub phase: Phase,
}

impl From<BluetoothStatus> for StatusView {
    fn from(status: BluetoothStatus) -> Self {
        let phase = status.phase();
        Self { status, phase }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> BluetoothStatus {
        BluetoothStatus {
            supported: true,
            state: "poweredOn".into(),
            published: true,
            advertising: true,
            ..Default::default()
        }
    }

    #[test]
    fn phases_follow_the_link() {
        assert_eq!(BluetoothStatus::default().phase(), Phase::Unsupported);
        let mut s = on();
        assert_eq!(s.phase(), Phase::WaitingForPhone);
        s.subscribed = true;
        assert_eq!(s.phase(), Phase::Connected);
        s.state = "unauthorized".into();
        assert_eq!(s.phase(), Phase::NeedsPermission);
        s.state = "poweredOff".into();
        assert_eq!(s.phase(), Phase::BluetoothOff);
        s.state = "notStarted".into();
        assert_eq!(s.phase(), Phase::NotStarted);
    }

    #[test]
    fn the_view_carries_the_phase_in_camel_case() {
        let json = serde_json::to_value(StatusView::from(on())).unwrap();
        assert_eq!(json["phase"], "waitingForPhone");
        assert_eq!(json["failedServices"], serde_json::json!([]));
        assert_eq!(json["state"], "poweredOn");
    }

    #[test]
    fn a_refused_hid_service_is_broken_even_if_advertising() {
        let mut s = on();
        s.failed_services = vec!["00001812-0000-1000-8000-00805F9B34FB".into()];
        assert_eq!(s.phase(), Phase::Broken);
    }
}
