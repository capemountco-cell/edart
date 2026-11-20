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
    /// An iPhone has read or written one of Edart's characteristics: it
    /// found Edart and is pairing, even if it has not subscribed yet.
    pub phone_seen: bool,
    /// The user paused Edart: its keyboard-and-mouse services are taken
    /// down so the iPhone is free, but the pairing is kept for Resume.
    pub paused: bool,
    /// The iPhone read Edart's setup but never started listening: it is
    /// holding an old copy from before Edart was reinstalled, and Edart
    /// has told it to look again.
    pub refreshing: bool,
    /// The name the iPhone lists Edart under: the Mac's computer name, not
    /// "Edart" (iOS shows the device, not the app).
    pub mac_name: Option<String>,
    /// Services macOS refused, by UUID.
    pub failed_services: Vec<String>,
    /// The last error worth showing, if any.
    pub error: Option<String>,
    /// Recent Bluetooth events, oldest first, for the Details panel.
    pub events: Vec<String>,
}

/// Most events kept in [`BluetoothStatus::events`].
pub const MAX_EVENTS: usize = 60;

impl BluetoothStatus {
    /// Record an event, dropping the oldest past [`MAX_EVENTS`].
    pub fn log(&mut self, event: impl Into<String>) {
        self.events.push(event.into());
        if self.events.len() > MAX_EVENTS {
            let extra = self.events.len() - MAX_EVENTS;
            self.events.drain(..extra);
        }
    }
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
    Pairing,
    Refreshing,
    Connected,
    Paused,
}

impl BluetoothStatus {
    pub fn phase(&self) -> Phase {
        if !self.supported || self.state == "unsupported" {
            return Phase::Unsupported;
        }
        if self.paused {
            return Phase::Paused;
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
        } else if self.refreshing {
            Phase::Refreshing
        } else if self.phone_seen {
            Phase::Pairing
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
        s.phone_seen = true;
        assert_eq!(s.phase(), Phase::Pairing);
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
    fn the_event_log_keeps_the_newest() {
        let mut s = BluetoothStatus::default();
        for i in 0..(MAX_EVENTS + 5) {
            s.log(format!("e{i}"));
        }
        assert_eq!(s.events.len(), MAX_EVENTS);
        assert_eq!(s.events[0], "e5");
        assert_eq!(s.events.last().unwrap(), &format!("e{}", MAX_EVENTS + 4));
    }

    #[test]
    fn paused_wins_over_the_link_state() {
        let mut s = on();
        s.subscribed = true;
        s.paused = true;
        assert_eq!(s.phase(), Phase::Paused);
        s.paused = false;
        assert_eq!(s.phase(), Phase::Connected);
    }

    #[test]
    fn a_phone_with_an_old_setup_shows_as_refreshing_until_it_listens() {
        let mut s = on();
        s.phone_seen = true;
        s.refreshing = true;
        assert_eq!(s.phase(), Phase::Refreshing);
        s.subscribed = true;
        assert_eq!(s.phase(), Phase::Connected);
    }

    #[test]
    fn a_refused_hid_service_is_broken_even_if_advertising() {
        let mut s = on();
        s.failed_services = vec!["00001812-0000-1000-8000-00805F9B34FB".into()];
        assert_eq!(s.phase(), Phase::Broken);
    }
}
