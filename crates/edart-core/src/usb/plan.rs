//! Turn a gesture into the timed list of USB touch, key and button events
//! that performs it.
//!
//! Unlike the Bluetooth path there is no pointer to move: a touch report
//! puts a finger straight on a point. Timings are iphone-use's (MIT,
//! Sunghyun Kim), measured on real iPhones:
//! - a tap needs about 50 ms of contact and several samples, or UIKit takes
//!   it for noise;
//! - a held finger is re-sent every 20 ms;
//! - Shift is pressed on its own before the key and released after it,
//!   because the phone sometimes misses a modifier that shares a report.

use crate::gesture::{Gesture, PlanError, Screen};
use crate::hid::{key_for_char, MOD_SHIFT};

use super::hid::{BUTTON_DOWN, BUTTON_HOME, BUTTON_UP, KEY_LEFT_SHIFT, TOUCH_MAX};

/// A step of a USB plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbStep {
    /// A finger on (or lifted from) a point, in 0..=65535 on both axes.
    Touch {
        down: bool,
        x: u16,
        y: u16,
    },
    /// The full set of keyboard usages held down now. Empty releases all.
    Keys(Vec<u8>),
    /// A hardware button going down or up.
    Button {
        usage: u64,
        state: u64,
    },
    WaitMs(u64),
}

/// How long a tap keeps the finger down.
const TAP_HOLD_MS: u64 = 80;
/// Gap between touch samples while a finger is held still.
const HOLD_SAMPLE_MS: u64 = 20;
/// Gap between the two taps of a double tap.
const DOUBLE_GAP_MS: u64 = 100;
/// Gap between touch samples of a swipe.
const SWIPE_SAMPLE_MS: u64 = 16;
/// How long each keyboard state is held.
const KEY_HOLD_MS: u64 = 20;
/// How long a button is held.
const BUTTON_HOLD_MS: u64 = 80;
const DEFAULT_LONG_PRESS_MS: u64 = 800;
const DEFAULT_SWIPE_MS: u64 = 350;
/// Longest press or swipe accepted (a stuck finger otherwise).
const MAX_GESTURE_MS: u64 = 10_000;
/// Longest text accepted in one gesture.
const MAX_TEXT_CHARS: usize = 2_000;

/// Map a point in screen units to touch coordinates (0..=65535).
///
/// The phone's touch coordinates follow the panel's native orientation, so
/// this is right for an iPhone held upright, which is how screenshots of an
/// iPhone come too.
pub fn to_touch(screen: Screen, x: f64, y: f64) -> Result<(u16, u16), PlanError> {
    let ok = |v: f64| v.is_finite() && v > 0.0;
    if !(ok(screen.width) && ok(screen.height)) {
        return Err(PlanError::BadScreen);
    }
    if !(x.is_finite() && y.is_finite())
        || x < 0.0
        || y < 0.0
        || x > screen.width
        || y > screen.height
    {
        return Err(PlanError::OffScreen);
    }
    let max = f64::from(TOUCH_MAX);
    Ok((
        (x / screen.width * max).round() as u16,
        (y / screen.height * max).round() as u16,
    ))
}

/// Hold a finger on a point for `ms`, then lift it.
fn press(steps: &mut Vec<UsbStep>, x: u16, y: u16, ms: u64) {
    let samples = (ms / HOLD_SAMPLE_MS).max(2);
    for _ in 0..samples {
        steps.push(UsbStep::Touch { down: true, x, y });
        steps.push(UsbStep::WaitMs((ms / samples).max(1)));
    }
    steps.push(UsbStep::Touch { down: false, x, y });
}

/// Plan a gesture as timed USB events.
pub fn plan(gesture: &Gesture, screen: Screen) -> Result<Vec<UsbStep>, PlanError> {
    let mut steps = Vec::new();
    match *gesture {
        Gesture::Tap { x, y } => {
            let (x, y) = to_touch(screen, x, y)?;
            press(&mut steps, x, y, TAP_HOLD_MS);
        }
        Gesture::DoubleTap { x, y } => {
            let (x, y) = to_touch(screen, x, y)?;
            press(&mut steps, x, y, TAP_HOLD_MS);
            steps.push(UsbStep::WaitMs(DOUBLE_GAP_MS));
            press(&mut steps, x, y, TAP_HOLD_MS);
        }
        Gesture::LongPress { x, y, ms } => {
            let ms = ms.unwrap_or(DEFAULT_LONG_PRESS_MS);
            if ms > MAX_GESTURE_MS {
                return Err(PlanError::TooLong);
            }
            let (x, y) = to_touch(screen, x, y)?;
            press(&mut steps, x, y, ms.max(TAP_HOLD_MS));
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
            let (fx, fy) = to_touch(screen, from_x, from_y)?;
            let (tx, ty) = to_touch(screen, to_x, to_y)?;
            let n = (ms / SWIPE_SAMPLE_MS).max(2);
            let lerp = |a: u16, b: u16, i: u64| {
                let (a, b) = (f64::from(a), f64::from(b));
                (a + (b - a) * i as f64 / n as f64).round() as u16
            };
            for i in 0..=n {
                steps.push(UsbStep::Touch {
                    down: true,
                    x: lerp(fx, tx, i),
                    y: lerp(fy, ty, i),
                });
                steps.push(UsbStep::WaitMs((ms / n).max(1)));
            }
            steps.push(UsbStep::Touch {
                down: false,
                x: tx,
                y: ty,
            });
        }
        Gesture::TypeText { ref text } => {
            if text.chars().count() > MAX_TEXT_CHARS {
                return Err(PlanError::TooLong);
            }
            let key = |steps: &mut Vec<UsbStep>, usages: Vec<u8>| {
                steps.push(UsbStep::Keys(usages));
                steps.push(UsbStep::WaitMs(KEY_HOLD_MS));
            };
            for c in text.chars() {
                let (mods, usage) = key_for_char(c).ok_or(PlanError::UntypableChar(c))?;
                if mods & MOD_SHIFT != 0 {
                    key(&mut steps, vec![KEY_LEFT_SHIFT]);
                    key(&mut steps, vec![KEY_LEFT_SHIFT, usage]);
                    key(&mut steps, vec![KEY_LEFT_SHIFT]);
                } else {
                    key(&mut steps, vec![usage]);
                }
                key(&mut steps, vec![]);
            }
        }
        Gesture::PressHome => {
            steps.push(UsbStep::Button {
                usage: BUTTON_HOME,
                state: BUTTON_DOWN,
            });
            steps.push(UsbStep::WaitMs(BUTTON_HOLD_MS));
            steps.push(UsbStep::Button {
                usage: BUTTON_HOME,
                state: BUTTON_UP,
            });
        }
    }
    Ok(steps)
}

/// Which of the phone's services a plan needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Needs {
    /// Touch or keyboard reports.
    pub reports: bool,
    /// Hardware buttons.
    pub buttons: bool,
}

pub fn needs(steps: &[UsbStep]) -> Needs {
    Needs {
        reports: steps
            .iter()
            .any(|s| matches!(s, UsbStep::Touch { .. } | UsbStep::Keys(_))),
        buttons: steps.iter().any(|s| matches!(s, UsbStep::Button { .. })),
    }
}

/// What to send so nothing stays pressed if a plan stops part-way: lift the
/// finger where it last was, release every key, cancel a held button.
pub fn release_after(done: &[UsbStep]) -> Vec<UsbStep> {
    let mut out = Vec::new();
    if let Some(&UsbStep::Touch { down: true, x, y }) = done
        .iter()
        .rev()
        .find(|s| matches!(s, UsbStep::Touch { .. }))
    {
        out.push(UsbStep::Touch { down: false, x, y });
    }
    if let Some(UsbStep::Keys(keys)) = done.iter().rev().find(|s| matches!(s, UsbStep::Keys(_))) {
        if !keys.is_empty() {
            out.push(UsbStep::Keys(vec![]));
        }
    }
    if let Some(&UsbStep::Button { usage, state }) = done
        .iter()
        .rev()
        .find(|s| matches!(s, UsbStep::Button { .. }))
    {
        if state == BUTTON_DOWN {
            out.push(UsbStep::Button {
                usage,
                state: BUTTON_UP,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: Screen = Screen {
        width: 390.0,
        height: 844.0,
    };

    fn touches(steps: &[UsbStep]) -> Vec<(bool, u16, u16)> {
        steps
            .iter()
            .filter_map(|s| match *s {
                UsbStep::Touch { down, x, y } => Some((down, x, y)),
                _ => None,
            })
            .collect()
    }

    fn total_ms(steps: &[UsbStep]) -> u64 {
        steps
            .iter()
            .map(|s| match s {
                UsbStep::WaitMs(ms) => *ms,
                _ => 0,
            })
            .sum()
    }

    #[test]
    fn points_scale_to_the_touch_range() {
        assert_eq!(to_touch(PHONE, 0.0, 0.0), Ok((0, 0)));
        assert_eq!(to_touch(PHONE, 390.0, 844.0), Ok((65535, 65535)));
        assert_eq!(to_touch(PHONE, 195.0, 422.0), Ok((32768, 32768)));
        assert_eq!(to_touch(PHONE, 391.0, 1.0), Err(PlanError::OffScreen));
        assert_eq!(to_touch(PHONE, f64::NAN, 1.0), Err(PlanError::OffScreen));
        let flat = Screen {
            width: 0.0,
            height: 1.0,
        };
        assert_eq!(to_touch(flat, 0.0, 0.0), Err(PlanError::BadScreen));
    }

    #[test]
    fn a_tap_is_several_contacts_then_a_lift_at_the_same_point() {
        let steps = plan(&Gesture::Tap { x: 195.0, y: 422.0 }, PHONE).unwrap();
        let t = touches(&steps);
        assert_eq!(t.len(), 5);
        assert!(t[..4].iter().all(|&p| p == (true, 32768, 32768)));
        assert_eq!(t[4], (false, 32768, 32768));
        assert_eq!(total_ms(&steps), 80);
        assert_eq!(
            steps.last(),
            Some(&UsbStep::Touch {
                down: false,
                x: 32768,
                y: 32768
            })
        );
    }

    #[test]
    fn a_double_tap_lifts_between_taps() {
        let steps = plan(&Gesture::DoubleTap { x: 10.0, y: 10.0 }, PHONE).unwrap();
        let lifts = touches(&steps).iter().filter(|t| !t.0).count();
        assert_eq!(lifts, 2);
        assert_eq!(total_ms(&steps), 80 + 100 + 80);
    }

    #[test]
    fn a_long_press_holds_for_its_duration() {
        let g = Gesture::LongPress {
            x: 10.0,
            y: 10.0,
            ms: Some(1000),
        };
        let steps = plan(&g, PHONE).unwrap();
        assert_eq!(total_ms(&steps), 1000);
        assert_eq!(touches(&steps).len(), 51);
        let default = Gesture::LongPress {
            x: 10.0,
            y: 10.0,
            ms: None,
        };
        assert_eq!(total_ms(&plan(&default, PHONE).unwrap()), 800);
        let stuck = Gesture::LongPress {
            x: 10.0,
            y: 10.0,
            ms: Some(10_001),
        };
        assert_eq!(plan(&stuck, PHONE), Err(PlanError::TooLong));
    }

    #[test]
    fn a_swipe_starts_and_ends_on_its_points_and_moves_one_way() {
        let g = Gesture::Swipe {
            from_x: 195.0,
            from_y: 640.0,
            to_x: 195.0,
            to_y: 240.0,
            ms: Some(320),
        };
        let steps = plan(&g, PHONE).unwrap();
        let t = touches(&steps);
        let (_, sx, sy) = t[0];
        let (down, ex, ey) = *t.last().unwrap();
        assert_eq!((sx, sy), to_touch(PHONE, 195.0, 640.0).unwrap());
        assert_eq!((ex, ey), to_touch(PHONE, 195.0, 240.0).unwrap());
        assert!(!down);
        // 20 intervals: 21 contacts and the lift.
        assert_eq!(t.len(), 22);
        assert_eq!(t[t.len() - 2], (true, ex, ey));
        assert!(t.windows(2).all(|w| w[1].2 <= w[0].2 && w[1].1 == w[0].1));
        assert!(t[..t.len() - 1].iter().all(|p| p.0));
    }

    #[test]
    fn a_swipe_with_an_end_off_screen_is_refused() {
        let g = Gesture::Swipe {
            from_x: 10.0,
            from_y: 10.0,
            to_x: 10.0,
            to_y: 900.0,
            ms: None,
        };
        assert_eq!(plan(&g, PHONE), Err(PlanError::OffScreen));
    }

    #[test]
    fn typing_presses_shift_on_its_own_around_a_capital() {
        let g = Gesture::TypeText { text: "aB".into() };
        let keys: Vec<Vec<u8>> = plan(&g, PHONE)
            .unwrap()
            .into_iter()
            .filter_map(|s| match s {
                UsbStep::Keys(k) => Some(k),
                _ => None,
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                vec![0x04],
                vec![],
                vec![0xE1],
                vec![0xE1, 0x05],
                vec![0xE1],
                vec![],
            ]
        );
    }

    #[test]
    fn typing_refuses_what_a_keyboard_cannot_type() {
        let g = Gesture::TypeText { text: "é".into() };
        assert_eq!(plan(&g, PHONE), Err(PlanError::UntypableChar('é')));
        let long = Gesture::TypeText {
            text: "a".repeat(2001),
        };
        assert_eq!(plan(&long, PHONE), Err(PlanError::TooLong));
    }

    #[test]
    fn home_is_a_button_down_and_up() {
        let steps = plan(&Gesture::PressHome, PHONE).unwrap();
        assert_eq!(
            steps,
            vec![
                UsbStep::Button {
                    usage: 0x40,
                    state: 1
                },
                UsbStep::WaitMs(80),
                UsbStep::Button {
                    usage: 0x40,
                    state: 2
                },
            ]
        );
    }

    #[test]
    fn a_plan_says_which_services_it_needs() {
        let tap = plan(&Gesture::Tap { x: 1.0, y: 1.0 }, PHONE).unwrap();
        assert_eq!(
            needs(&tap),
            Needs {
                reports: true,
                buttons: false
            }
        );
        let home = plan(&Gesture::PressHome, PHONE).unwrap();
        assert_eq!(
            needs(&home),
            Needs {
                reports: false,
                buttons: true
            }
        );
    }

    #[test]
    fn every_plan_ends_with_nothing_held() {
        let gestures = [
            Gesture::Tap { x: 1.0, y: 1.0 },
            Gesture::DoubleTap { x: 1.0, y: 1.0 },
            Gesture::LongPress {
                x: 1.0,
                y: 1.0,
                ms: None,
            },
            Gesture::Swipe {
                from_x: 1.0,
                from_y: 1.0,
                to_x: 9.0,
                to_y: 9.0,
                ms: None,
            },
            Gesture::TypeText { text: "Hi!".into() },
            Gesture::PressHome,
        ];
        for g in gestures {
            let steps = plan(&g, PHONE).unwrap();
            assert_eq!(release_after(&steps), vec![], "{g:?}");
        }
    }

    #[test]
    fn an_interrupted_plan_releases_what_it_holds() {
        let steps = plan(&Gesture::Tap { x: 195.0, y: 422.0 }, PHONE).unwrap();
        assert_eq!(
            release_after(&steps[..1]),
            vec![UsbStep::Touch {
                down: false,
                x: 32768,
                y: 32768
            }]
        );
        let typing = plan(&Gesture::TypeText { text: "A".into() }, PHONE).unwrap();
        assert_eq!(release_after(&typing[..3]), vec![UsbStep::Keys(vec![])]);
        let home = plan(&Gesture::PressHome, PHONE).unwrap();
        assert_eq!(
            release_after(&home[..1]),
            vec![UsbStep::Button {
                usage: 0x40,
                state: 2
            }]
        );
        assert_eq!(release_after(&[]), vec![]);
    }
}
