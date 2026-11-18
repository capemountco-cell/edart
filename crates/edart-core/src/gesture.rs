//! Turn a gesture ("tap at 120, 300") into the timed list of HID reports
//! that performs it on the iPhone.
//!
//! How the phone reads them, verified by Mac_iPhone_touch on real hardware:
//! - an absolute report moves the pointer straight to a point, but its
//!   button bits are ignored;
//! - so a click is a relative report with dx = dy = 0 carrying the button;
//! - a held button must be re-sent about every 100 ms, or iOS treats the
//!   pointer as idle and lets go;
//! - a drag needs button and motion in the same report, so it is relative.

use serde::{Deserialize, Serialize};

use crate::hid::{self, Report};

/// The iPhone screen in the units the caller uses for coordinates
/// (screenshot pixels, normally).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub width: f64,
    pub height: f64,
}

/// One thing to do to the phone. Coordinates are in [`Screen`] units,
/// origin top-left.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Gesture {
    Tap {
        x: f64,
        y: f64,
    },
    DoubleTap {
        x: f64,
        y: f64,
    },
    LongPress {
        x: f64,
        y: f64,
        #[serde(default)]
        ms: Option<u64>,
    },
    Swipe {
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        #[serde(default)]
        ms: Option<u64>,
    },
    TypeText {
        text: String,
    },
    PressHome,
}

/// A step of a plan: send a report, or wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Send(Report),
    WaitMs(u64),
}

/// Tunables that depend on the phone. Defaults are the reference's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    /// Relative mouse counts per screen unit, used only by drags. iOS
    /// tracking speed changes it; calibration will set it per phone.
    pub drag_counts_per_unit: f64,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            drag_counts_per_unit: 1.0,
        }
    }
}

/// Why a gesture could not be planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The screen size is zero, negative or not a number.
    BadScreen,
    /// A coordinate is outside the screen or not a number.
    OffScreen,
    /// Text contains a character a US keyboard cannot type.
    UntypableChar(char),
    /// A duration is longer than this module allows (a stuck finger).
    TooLong,
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::BadScreen => write!(f, "the screen size is not valid"),
            PlanError::OffScreen => write!(f, "that point is outside the screen"),
            PlanError::UntypableChar(c) => write!(f, "cannot type {c:?} with a keyboard"),
            PlanError::TooLong => write!(f, "that gesture is too long"),
        }
    }
}

impl std::error::Error for PlanError {}

/// Gap after moving the pointer, before pressing.
const SETTLE_MS: u64 = 30;
/// How long a tap holds the button.
const TAP_HOLD_MS: u64 = 60;
/// Gap between the two taps of a double tap.
const DOUBLE_GAP_MS: u64 = 80;
/// Re-send a held button this often.
const HOLD_RESEND_MS: u64 = 100;
const DEFAULT_LONG_PRESS_MS: u64 = 800;
const DEFAULT_SWIPE_MS: u64 = 300;
/// One relative step of a drag.
const DRAG_STEP_MS: u64 = 15;
/// Longest press or swipe accepted.
const MAX_GESTURE_MS: u64 = 10_000;
/// Gap between typed characters.
const KEY_GAP_MS: u64 = 15;

/// Map a point to absolute-report coordinates (0..=32767 on both axes).
pub fn to_absolute(screen: Screen, x: f64, y: f64) -> Result<(u16, u16), PlanError> {
    check_screen(screen)?;
    if !(x.is_finite() && y.is_finite())
        || x < 0.0
        || y < 0.0
        || x > screen.width
        || y > screen.height
    {
        return Err(PlanError::OffScreen);
    }
    let max = f64::from(hid::ABSOLUTE_MAX);
    let ax = (x / screen.width * max).round() as u16;
    let ay = (y / screen.height * max).round() as u16;
    Ok((ax, ay))
}

fn check_screen(screen: Screen) -> Result<(), PlanError> {
    let ok = |v: f64| v.is_finite() && v > 0.0;
    if ok(screen.width) && ok(screen.height) {
        Ok(())
    } else {
        Err(PlanError::BadScreen)
    }
}

fn press() -> Step {
    Step::Send(hid::relative(hid::BUTTON_LEFT, 0, 0, 0))
}

fn release() -> Step {
    Step::Send(hid::relative(0, 0, 0, 0))
}

/// Hold the (already pressed) button for `ms`, re-sending it so iOS keeps
/// treating it as held.
fn hold(steps: &mut Vec<Step>, ms: u64) {
    let mut left = ms;
    while left > 0 {
        let chunk = left.min(HOLD_RESEND_MS);
        steps.push(Step::WaitMs(chunk));
        left -= chunk;
        if left > 0 {
            steps.push(press());
        }
    }
}

fn move_to(steps: &mut Vec<Step>, screen: Screen, x: f64, y: f64) -> Result<(), PlanError> {
    let (ax, ay) = to_absolute(screen, x, y)?;
    steps.push(Step::Send(hid::absolute(ax, ay)));
    steps.push(Step::WaitMs(SETTLE_MS));
    Ok(())
}

/// Plan a gesture as timed HID reports.
pub fn plan(gesture: &Gesture, screen: Screen, tuning: Tuning) -> Result<Vec<Step>, PlanError> {
    let mut steps = Vec::new();
    match *gesture {
        Gesture::Tap { x, y } => {
            move_to(&mut steps, screen, x, y)?;
            steps.push(press());
            steps.push(Step::WaitMs(TAP_HOLD_MS));
            steps.push(release());
        }
        Gesture::DoubleTap { x, y } => {
            move_to(&mut steps, screen, x, y)?;
            for i in 0..2 {
                if i == 1 {
                    steps.push(Step::WaitMs(DOUBLE_GAP_MS));
                }
                steps.push(press());
                steps.push(Step::WaitMs(TAP_HOLD_MS));
                steps.push(release());
            }
        }
        Gesture::LongPress { x, y, ms } => {
            let ms = ms.unwrap_or(DEFAULT_LONG_PRESS_MS);
            if ms > MAX_GESTURE_MS {
                return Err(PlanError::TooLong);
            }
            move_to(&mut steps, screen, x, y)?;
            steps.push(press());
            hold(&mut steps, ms.max(1));
            steps.push(release());
        }
        Gesture::Swipe {
            from_x,
            from_y,
            to_x,
            to_y,
            ms,
        } => {
            let ms = ms.unwrap_or(DEFAULT_SWIPE_MS);
            if ms > MAX_GESTURE_MS {
                return Err(PlanError::TooLong);
            }
            // Both ends must be on screen, even though only the start is
            // reached with an absolute report.
            to_absolute(screen, to_x, to_y)?;
            move_to(&mut steps, screen, from_x, from_y)?;
            steps.push(press());
            let n = (ms / DRAG_STEP_MS).max(1);
            let total_dx = (to_x - from_x) * tuning.drag_counts_per_unit;
            let total_dy = (to_y - from_y) * tuning.drag_counts_per_unit;
            // Spread the motion evenly; carry rounding so the total is exact.
            let (mut sent_x, mut sent_y) = (0i64, 0i64);
            for i in 1..=n {
                let want_x = (total_dx * i as f64 / n as f64).round() as i64;
                let want_y = (total_dy * i as f64 / n as f64).round() as i64;
                let (mut dx, mut dy) = (want_x - sent_x, want_y - sent_y);
                // A step bigger than a signed byte is split.
                while dx != 0 || dy != 0 {
                    let sx = dx.clamp(-127, 127);
                    let sy = dy.clamp(-127, 127);
                    steps.push(Step::Send(hid::relative(
                        hid::BUTTON_LEFT,
                        sx as i32,
                        sy as i32,
                        0,
                    )));
                    dx -= sx;
                    dy -= sy;
                }
                sent_x = want_x;
                sent_y = want_y;
                steps.push(Step::WaitMs(DRAG_STEP_MS));
            }
            steps.push(release());
        }
        Gesture::TypeText { ref text } => {
            for c in text.chars() {
                let (mods, usage) = hid::key_for_char(c).ok_or(PlanError::UntypableChar(c))?;
                steps.push(Step::Send(hid::keyboard(mods, usage)));
                steps.push(Step::Send(hid::keyboard(0, 0)));
                steps.push(Step::WaitMs(KEY_GAP_MS));
            }
        }
        Gesture::PressHome => {
            steps.push(Step::Send(hid::consumer(hid::USAGE_HOME)));
            steps.push(Step::WaitMs(TAP_HOLD_MS));
            steps.push(Step::Send(hid::consumer(0)));
        }
    }
    Ok(steps)
}

/// Total time a plan takes, in milliseconds.
pub fn duration_ms(steps: &[Step]) -> u64 {
    steps
        .iter()
        .map(|s| match s {
            Step::WaitMs(ms) => *ms,
            Step::Send(_) => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: Screen = Screen {
        width: 390.0,
        height: 844.0,
    };

    fn sends(steps: &[Step]) -> Vec<&Report> {
        steps
            .iter()
            .filter_map(|s| match s {
                Step::Send(r) => Some(r),
                Step::WaitMs(_) => None,
            })
            .collect()
    }

    #[test]
    fn corners_map_to_the_absolute_range() {
        assert_eq!(to_absolute(PHONE, 0.0, 0.0), Ok((0, 0)));
        assert_eq!(to_absolute(PHONE, 390.0, 844.0), Ok((32767, 32767)));
        assert_eq!(to_absolute(PHONE, 195.0, 422.0), Ok((16384, 16384)));
    }

    #[test]
    fn rejects_points_off_screen_and_bad_screens() {
        assert_eq!(to_absolute(PHONE, -1.0, 10.0), Err(PlanError::OffScreen));
        assert_eq!(to_absolute(PHONE, 10.0, 845.0), Err(PlanError::OffScreen));
        assert_eq!(to_absolute(PHONE, f64::NAN, 1.0), Err(PlanError::OffScreen));
        let flat = Screen {
            width: 0.0,
            height: 10.0,
        };
        assert_eq!(to_absolute(flat, 0.0, 0.0), Err(PlanError::BadScreen));
    }

    #[test]
    fn tap_moves_then_clicks_through_the_relative_channel() {
        let steps = plan(
            &Gesture::Tap { x: 195.0, y: 422.0 },
            PHONE,
            Tuning::default(),
        )
        .unwrap();
        let s = sends(&steps);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].id, hid::RID_ABSOLUTE);
        assert_eq!(s[1], &hid::relative(hid::BUTTON_LEFT, 0, 0, 0));
        assert_eq!(s[2], &hid::relative(0, 0, 0, 0));
    }

    #[test]
    fn double_tap_clicks_twice() {
        let steps = plan(
            &Gesture::DoubleTap { x: 1.0, y: 1.0 },
            PHONE,
            Tuning::default(),
        )
        .unwrap();
        let presses = sends(&steps)
            .iter()
            .filter(|r| r.id == hid::RID_RELATIVE && r.payload[0] == hid::BUTTON_LEFT)
            .count();
        assert_eq!(presses, 2);
    }

    #[test]
    fn long_press_resends_the_button_at_least_every_100ms() {
        let g = Gesture::LongPress {
            x: 10.0,
            y: 10.0,
            ms: Some(450),
        };
        let steps = plan(&g, PHONE, Tuning::default()).unwrap();
        // Between the first press and the release no wait exceeds 100 ms.
        let first_press = steps.iter().position(|s| *s == press()).unwrap();
        let rel = steps.iter().rposition(|s| *s == release()).unwrap();
        let mut held = 0;
        for s in &steps[first_press..rel] {
            if let Step::WaitMs(ms) = s {
                assert!(*ms <= HOLD_RESEND_MS);
                held += ms;
            }
        }
        assert_eq!(held, 450);
    }

    #[test]
    fn long_press_refuses_a_stuck_finger() {
        let g = Gesture::LongPress {
            x: 1.0,
            y: 1.0,
            ms: Some(60_000),
        };
        assert_eq!(plan(&g, PHONE, Tuning::default()), Err(PlanError::TooLong));
    }

    #[test]
    fn swipe_moves_exactly_the_requested_distance_with_the_button_down() {
        let g = Gesture::Swipe {
            from_x: 195.0,
            from_y: 700.0,
            to_x: 195.0,
            to_y: 100.0,
            ms: Some(300),
        };
        let tuning = Tuning {
            drag_counts_per_unit: 1.5,
        };
        let steps = plan(&g, PHONE, tuning).unwrap();
        let (mut dx, mut dy) = (0i64, 0i64);
        for r in sends(&steps) {
            if r.id == hid::RID_RELATIVE && r.payload[0] == hid::BUTTON_LEFT {
                dx += r.payload[1] as i8 as i64;
                dy += r.payload[2] as i8 as i64;
            }
        }
        assert_eq!((dx, dy), (0, -900));
        assert_eq!(steps.last(), Some(&release()));
    }

    #[test]
    fn swipe_end_must_be_on_screen() {
        let g = Gesture::Swipe {
            from_x: 10.0,
            from_y: 10.0,
            to_x: 1000.0,
            to_y: 10.0,
            ms: None,
        };
        assert_eq!(
            plan(&g, PHONE, Tuning::default()),
            Err(PlanError::OffScreen)
        );
    }

    #[test]
    fn typing_presses_and_releases_each_key() {
        let steps = plan(
            &Gesture::TypeText { text: "Hi".into() },
            PHONE,
            Tuning::default(),
        )
        .unwrap();
        let s = sends(&steps);
        assert_eq!(s.len(), 4);
        assert_eq!(s[0], &hid::keyboard(hid::MOD_SHIFT, 0x0B));
        assert_eq!(s[1], &hid::keyboard(0, 0));
        assert_eq!(s[2], &hid::keyboard(0, 0x0C));
    }

    #[test]
    fn typing_refuses_characters_a_keyboard_lacks() {
        let g = Gesture::TypeText {
            text: "café".into(),
        };
        assert_eq!(
            plan(&g, PHONE, Tuning::default()),
            Err(PlanError::UntypableChar('é'))
        );
    }

    #[test]
    fn home_presses_and_releases_ac_home() {
        let steps = plan(&Gesture::PressHome, PHONE, Tuning::default()).unwrap();
        let s = sends(&steps);
        assert_eq!(s[0], &hid::consumer(hid::USAGE_HOME));
        assert_eq!(s[1], &hid::consumer(0));
    }

    #[test]
    fn gestures_parse_from_the_api_json() {
        let g: Gesture = serde_json::from_str(r#"{"type":"tap","x":195,"y":442}"#).unwrap();
        assert_eq!(g, Gesture::Tap { x: 195.0, y: 442.0 });
        let g: Gesture = serde_json::from_str(r#"{"type":"press_home"}"#).unwrap();
        assert_eq!(g, Gesture::PressHome);
        let g: Gesture = serde_json::from_str(r#"{"type":"long_press","x":1,"y":2}"#).unwrap();
        assert_eq!(
            g,
            Gesture::LongPress {
                x: 1.0,
                y: 2.0,
                ms: None
            }
        );
    }

    #[test]
    fn duration_adds_the_waits() {
        let steps = plan(&Gesture::Tap { x: 1.0, y: 1.0 }, PHONE, Tuning::default()).unwrap();
        assert_eq!(duration_ms(&steps), SETTLE_MS + TAP_HOLD_MS);
    }
}
