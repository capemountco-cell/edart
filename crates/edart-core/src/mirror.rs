//! What the UI shows about the screen mirror, decided from plain facts the
//! capture code reports.
//!
//! A USB-connected iPhone shows up on the Mac as a camera once the app
//! asks CoreMediaIO to list screen-capture devices. This module does not
//! touch any of that: it turns "is camera access allowed, is a phone
//! there, did a frame arrive" into one phase for the headline.

use serde::Serialize;

use crate::gesture::Screen;

/// What macOS says about Camera access for this app.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CameraAccess {
    /// Not asked macOS yet.
    #[default]
    Unknown,
    /// The user has not been prompted.
    NotDetermined,
    Denied,
    /// Blocked by a profile or parental controls; the user cannot change it.
    Restricted,
    Allowed,
}

/// The newest frame, as far as the status cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameFacts {
    pub width: u32,
    pub height: u32,
    /// The picture is black all over: the screen is off.
    pub dark: bool,
}

/// What the capture code knows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirrorFacts {
    /// This build can mirror at all (macOS only).
    pub supported: bool,
    /// Mirroring was switched on (at launch when already allowed, or by
    /// the user).
    pub started: bool,
    pub access: CameraAccess,
    /// The capture device's name, when an iPhone is listed.
    pub device_name: Option<String>,
    /// Milliseconds since capture from that device began.
    pub capturing_for_ms: Option<u64>,
    pub frame: Option<FrameFacts>,
    pub error: Option<String>,
}

/// The coarse step the mirror is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MirrorPhase {
    Unsupported,
    /// Not switched on yet: nothing has been asked of the user.
    Off,
    /// macOS is showing (or about to show) the Camera prompt.
    NeedsPermission,
    /// Camera access was refused; only System Settings can change it.
    Denied,
    /// Allowed, but no iPhone is listed as a capture device.
    NoDevice,
    /// An iPhone is listed and capture just began.
    Starting,
    /// An iPhone is listed but has sent no picture for a while.
    Stalled,
    /// Frames arrive but the screen is off.
    Dark,
    Live,
}

/// Capture gets this long to deliver its first frame before the status
/// says something is wrong.
pub const FIRST_FRAME_MS: u64 = 4000;

impl MirrorFacts {
    pub fn phase(&self) -> MirrorPhase {
        if !self.supported {
            return MirrorPhase::Unsupported;
        }
        match self.access {
            CameraAccess::Denied | CameraAccess::Restricted => return MirrorPhase::Denied,
            _ if !self.started => return MirrorPhase::Off,
            CameraAccess::Unknown | CameraAccess::NotDetermined => {
                return MirrorPhase::NeedsPermission
            }
            CameraAccess::Allowed => {}
        }
        if self.device_name.is_none() {
            return MirrorPhase::NoDevice;
        }
        match self.frame {
            Some(frame) if frame.dark => MirrorPhase::Dark,
            Some(_) => MirrorPhase::Live,
            None if self.capturing_for_ms.unwrap_or(0) < FIRST_FRAME_MS => MirrorPhase::Starting,
            None => MirrorPhase::Stalled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Orientation {
    Portrait,
    Landscape,
}

/// The mirror, as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MirrorStatus {
    pub phase: MirrorPhase,
    pub access: CameraAccess,
    pub device_name: Option<String>,
    /// Size of the frame the UI and the agent get, in pixels. Gestures use
    /// these as their screen units.
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: Option<Orientation>,
    /// Goes up by one with every new frame, so the UI knows when to
    /// fetch the picture again.
    pub seq: u64,
    pub error: Option<String>,
    /// Recent mirror events, oldest first, for the Details panel.
    pub events: Vec<String>,
}

impl MirrorStatus {
    pub fn new(facts: &MirrorFacts, seq: u64, events: Vec<String>) -> Self {
        let phase = facts.phase();
        // A frame from a phone that has since gone is not the screen.
        let frame = match phase {
            MirrorPhase::Live | MirrorPhase::Dark => facts.frame,
            _ => None,
        };
        MirrorStatus {
            phase,
            access: facts.access,
            device_name: facts.device_name.clone(),
            width: frame.map(|f| f.width),
            height: frame.map(|f| f.height),
            orientation: frame.map(|f| {
                if f.width > f.height {
                    Orientation::Landscape
                } else {
                    Orientation::Portrait
                }
            }),
            seq,
            error: facts.error.clone(),
            events,
        }
    }

    /// The screen the agent should act on, when there is a usable picture.
    pub fn screen(&self) -> Option<Screen> {
        match (self.phase, self.width, self.height) {
            (MirrorPhase::Live, Some(width), Some(height)) => Some(Screen {
                width: f64::from(width),
                height: f64::from(height),
            }),
            _ => None,
        }
    }
}

/// Keep at most this many lines in the mirror's event log.
pub const MAX_EVENTS: usize = 60;

/// Append to an event log, dropping the oldest past [`MAX_EVENTS`].
pub fn push_event(events: &mut Vec<String>, line: String) {
    events.push(line);
    if events.len() > MAX_EVENTS {
        let extra = events.len() - MAX_EVENTS;
        events.drain(..extra);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live() -> MirrorFacts {
        MirrorFacts {
            supported: true,
            started: true,
            access: CameraAccess::Allowed,
            device_name: Some("Cape’s iPhone".into()),
            capturing_for_ms: Some(9000),
            frame: Some(FrameFacts {
                width: 739,
                height: 1600,
                dark: false,
            }),
            error: None,
        }
    }

    #[test]
    fn phases_follow_the_facts_in_order() {
        let mut facts = MirrorFacts::default();
        assert_eq!(facts.phase(), MirrorPhase::Unsupported);
        facts.supported = true;
        assert_eq!(facts.phase(), MirrorPhase::Off);
        facts.access = CameraAccess::NotDetermined;
        assert_eq!(facts.phase(), MirrorPhase::Off);
        facts.started = true;
        assert_eq!(facts.phase(), MirrorPhase::NeedsPermission);
        facts.access = CameraAccess::Allowed;
        assert_eq!(facts.phase(), MirrorPhase::NoDevice);
        facts.device_name = Some("iPhone".into());
        facts.capturing_for_ms = Some(500);
        assert_eq!(facts.phase(), MirrorPhase::Starting);
        facts.capturing_for_ms = Some(FIRST_FRAME_MS);
        assert_eq!(facts.phase(), MirrorPhase::Stalled);
        facts.frame = Some(FrameFacts {
            width: 739,
            height: 1600,
            dark: true,
        });
        assert_eq!(facts.phase(), MirrorPhase::Dark);
        assert_eq!(live().phase(), MirrorPhase::Live);
    }

    #[test]
    fn a_refusal_shows_even_before_mirroring_is_switched_on() {
        for access in [CameraAccess::Denied, CameraAccess::Restricted] {
            let facts = MirrorFacts {
                supported: true,
                access,
                ..Default::default()
            };
            assert_eq!(facts.phase(), MirrorPhase::Denied);
        }
    }

    #[test]
    fn the_status_reports_size_and_orientation_only_with_a_picture() {
        let status = MirrorStatus::new(&live(), 7, vec![]);
        assert_eq!((status.width, status.height), (Some(739), Some(1600)));
        assert_eq!(status.orientation, Some(Orientation::Portrait));
        assert_eq!(
            status.screen(),
            Some(Screen {
                width: 739.0,
                height: 1600.0
            })
        );

        let mut sideways = live();
        sideways.frame = Some(FrameFacts {
            width: 1600,
            height: 739,
            dark: false,
        });
        let status = MirrorStatus::new(&sideways, 8, vec![]);
        assert_eq!(status.orientation, Some(Orientation::Landscape));

        // Unplugged: the old frame is not offered as the screen.
        let mut gone = live();
        gone.device_name = None;
        let status = MirrorStatus::new(&gone, 9, vec![]);
        assert_eq!(status.phase, MirrorPhase::NoDevice);
        assert_eq!(status.width, None);
        assert_eq!(status.screen(), None);
    }

    #[test]
    fn a_dark_screen_is_not_something_to_act_on() {
        let mut dark = live();
        dark.frame = Some(FrameFacts {
            width: 739,
            height: 1600,
            dark: true,
        });
        let status = MirrorStatus::new(&dark, 1, vec![]);
        assert_eq!(status.width, Some(739));
        assert_eq!(status.screen(), None);
    }

    #[test]
    fn the_event_log_is_bounded() {
        let mut events = Vec::new();
        for i in 0..MAX_EVENTS + 5 {
            push_event(&mut events, i.to_string());
        }
        assert_eq!(events.len(), MAX_EVENTS);
        assert_eq!(events[0], "5");
    }

    #[test]
    fn the_status_serialises_in_camel_case() {
        let json = serde_json::to_value(MirrorStatus::new(&live(), 3, vec![])).unwrap();
        assert_eq!(json["phase"], "live");
        assert_eq!(json["deviceName"], "Cape’s iPhone");
        assert_eq!(json["orientation"], "portrait");
        assert_eq!(json["access"], "allowed");
    }
}
