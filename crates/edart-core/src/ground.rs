//! Grounding the agent in what is really on the screen.
//!
//! A model asked to "tap Kalshi" from a picture alone often lands a row
//! off. So each step also carries the screen's text as numbered elements
//! with their boxes, and the model can answer "tap element 12" instead
//! of guessing coordinates. Opening an app is taken out of its hands
//! altogether: [`open_app`] goes Home, finds the app's label in the text
//! and taps the icon above it.

use serde::Serialize;

use crate::gesture::{Gesture, Screen};
use crate::guard::{OcrFrame, OcrLine};

/// A piece of text on the screen the model can point at. The box is in
/// screenshot pixels, like every other coordinate it is given.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Element {
    /// 1, 2, 3… in the order sent.
    pub id: u32,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Most elements sent with a step.
pub const MAX_ELEMENTS: usize = 400;
/// Longest element text sent.
pub const MAX_ELEMENT_CHARS: usize = 200;
/// Text the recogniser is less sure of than this is left out.
const MIN_CONFIDENCE: f32 = 0.3;

impl Element {
    /// The middle of the element: where a tap on it goes.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// The screen's text as elements, top to bottom then left to right, in
/// the units of `screen` (the size the screenshot is described as).
pub fn elements(frame: &OcrFrame, screen: Screen) -> Vec<Element> {
    let sx = screen.width / frame.width.max(1.0);
    let sy = screen.height / frame.height.max(1.0);
    let mut lines: Vec<&OcrLine> = frame
        .lines
        .iter()
        .filter(|l| l.confidence >= MIN_CONFIDENCE && !l.text.trim().is_empty())
        .collect();
    // Rows first: lines whose tops are within a few pixels read as one row.
    let row = (frame.height / 130.0).max(1.0);
    lines.sort_by(|a, b| {
        ((a.y / row).floor(), a.x)
            .partial_cmp(&((b.y / row).floor(), b.x))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let round = |v: f64| (v * 10.0).round() / 10.0;
    lines
        .into_iter()
        .take(MAX_ELEMENTS)
        .enumerate()
        .map(|(i, line)| Element {
            id: i as u32 + 1,
            text: line.text.trim().chars().take(MAX_ELEMENT_CHARS).collect(),
            x: round(line.x * sx),
            y: round(line.y * sy),
            w: round(line.width * sx),
            h: round(line.height * sy),
        })
        .collect()
}

/// The tap that "tap element `id`" means.
pub fn tap_element(elements: &[Element], id: u32) -> Result<Gesture, String> {
    let element = elements.iter().find(|e| e.id == id).ok_or_else(|| {
        format!(
            "there is no element {id} on this screen (the elements are numbered 1 to {})",
            elements.len()
        )
    })?;
    let (x, y) = element.center();
    Ok(Gesture::Tap { x, y })
}

fn norm(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The line that names `app`: one that is exactly the name if there is
/// one, else one that contains it as whole words. Among equals, the
/// first in reading order that `keep` accepts.
fn find_named<'a>(
    frame: &'a OcrFrame,
    app: &str,
    keep: impl Fn(&OcrLine) -> bool,
) -> Option<&'a OcrLine> {
    let name = norm(app);
    if name.is_empty() {
        return None;
    }
    let mut lines: Vec<&OcrLine> = frame
        .lines
        .iter()
        .filter(|l| l.confidence >= MIN_CONFIDENCE && keep(l))
        .collect();
    lines.sort_by(|a, b| {
        (a.y, a.x)
            .partial_cmp(&(b.y, b.x))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let whole_words = |text: &str| {
        let padded = format!(
            " {} ",
            text.split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        );
        let wanted = format!(
            " {} ",
            name.split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        );
        !wanted.trim().is_empty() && padded.contains(&wanted)
    };
    lines
        .iter()
        .find(|l| norm(&l.text) == name)
        .or_else(|| lines.iter().find(|l| whole_words(&norm(&l.text))))
        .copied()
}

/// Where to tap to open the app whose Home Screen label is `label`: the
/// middle of the icon, which sits above its name.
fn icon_point(label: &OcrLine, frame: &OcrFrame) -> (f64, f64) {
    // An icon is about 15% of the screen's width, just above its label.
    let icon = frame.width * 0.155;
    (label.x + label.width / 2.0, label.y - icon * 0.6)
}

/// What [`open_app`] needs from the world.
pub trait Hands {
    /// Send a gesture. An error ends the attempt.
    fn perform(&mut self, gesture: &Gesture, screen: Screen) -> Result<(), String>;
    /// The text on the screen now.
    fn read_screen(&mut self) -> Result<OcrFrame, String>;
    fn wait_ms(&mut self, ms: u64);
}

/// After Home, and after opening search, the screen gets this long.
const SETTLE_MS: u64 = 900;
/// Search results take a little longer than an animation.
const RESULTS_MS: u64 = 1300;

/// Open an app by name, the same way every time: Home, find its label,
/// tap the icon above it. If it is not on that page and `search` is
/// allowed, look it up in Spotlight and tap the first result.
pub fn open_app(
    hands: &mut impl Hands,
    name: &str,
    screen: Screen,
    search: bool,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 || name.contains(['\n', '\r']) {
        return Err("open_app needs the app’s name".into());
    }
    let to_screen = |frame: &OcrFrame, (x, y): (f64, f64)| Gesture::Tap {
        x: (x / frame.width.max(1.0) * screen.width).clamp(0.0, screen.width),
        y: (y / frame.height.max(1.0) * screen.height).clamp(0.0, screen.height),
    };

    hands.perform(&Gesture::PressHome, screen)?;
    hands.wait_ms(SETTLE_MS);
    let home = hands
        .read_screen()
        .map_err(|why| format!("Edart couldn’t read the Home Screen to find {name} ({why})"))?;
    // A label has its icon above it, so it cannot be at the very top.
    let icon = home.width * 0.155;
    if let Some(label) = find_named(&home, name, |l| l.y > icon) {
        return hands.perform(&to_screen(&home, icon_point(label, &home)), screen);
    }
    if !search {
        return Err(format!(
            "{name} isn’t on the Home Screen page that Home goes to. Put it on the first page"
        ));
    }

    // Spotlight: pull down from the middle of the Home Screen and type.
    hands.perform(
        &Gesture::Swipe {
            from_x: screen.width / 2.0,
            from_y: screen.height * 0.4,
            to_x: screen.width / 2.0,
            to_y: screen.height * 0.65,
            ms: Some(300),
        },
        screen,
    )?;
    hands.wait_ms(SETTLE_MS);
    hands.perform(
        &Gesture::TypeText {
            text: name.to_string(),
        },
        screen,
    )?;
    hands.wait_ms(RESULTS_MS);
    let results = hands
        .read_screen()
        .map_err(|why| format!("Edart couldn’t read the search results for {name} ({why})"))?;
    // Results are listed above the keyboard; the search field, which
    // also shows the name, is at the very top or just above the keys.
    let top = results.height * 0.08;
    let bottom = results.height * 0.45;
    let hit = find_named(&results, name, |l| l.y >= top && l.y + l.height <= bottom).ok_or_else(
        || format!("{name} wasn’t found on the Home Screen or in search. Is it installed?"),
    )?;
    let point = (hit.x + hit.width / 2.0, hit.y + hit.height / 2.0);
    hands.perform(&to_screen(&results, point), screen)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f64 = 736.0;
    const H: f64 = 1600.0;
    const PHONE: Screen = Screen {
        width: W,
        height: H,
    };

    fn line(text: &str, x: f64, y: f64, width: f64, height: f64) -> OcrLine {
        OcrLine {
            text: text.into(),
            confidence: 1.0,
            x,
            y,
            width,
            height,
        }
    }

    fn frame(lines: Vec<OcrLine>) -> OcrFrame {
        OcrFrame {
            width: W,
            height: H,
            lines,
        }
    }

    /// Fiverr sits directly above Kalshi, as on the phone where a guessed
    /// tap opened the wrong one.
    fn home() -> OcrFrame {
        frame(vec![
            line("Fiverr", 370.0, 700.0, 90.0, 22.0),
            line("Settings", 70.0, 880.0, 90.0, 22.0),
            line("Kalshi", 370.0, 880.0, 90.0, 22.0),
            line("Notes", 540.0, 880.0, 90.0, 22.0),
            line("Search", 330.0, 1380.0, 80.0, 24.0),
        ])
    }

    #[test]
    fn elements_are_numbered_in_reading_order_and_bounded() {
        let screen = frame(vec![
            line("Down · 78%", 470.0, 1382.0, 170.0, 30.0),
            line("ZEC 15 min", 60.0, 150.0, 220.0, 40.0),
            line("Up · 22%", 110.0, 1380.0, 150.0, 30.0),
            line("  ", 0.0, 0.0, 10.0, 10.0),
            OcrLine {
                confidence: 0.1,
                ..line("smudge", 0.0, 500.0, 10.0, 10.0)
            },
            line(&"x".repeat(500), 60.0, 900.0, 600.0, 30.0),
        ]);
        let found = elements(&screen, PHONE);
        let texts: Vec<&str> = found.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts[..1], ["ZEC 15 min"]);
        assert_eq!(texts[2..], ["Up · 22%", "Down · 78%"]);
        assert_eq!(found[1].text.chars().count(), MAX_ELEMENT_CHARS);
        assert_eq!(found.iter().map(|e| e.id).collect::<Vec<_>>(), [1, 2, 3, 4]);
        assert_eq!(
            serde_json::to_value(&found[0]).unwrap(),
            serde_json::json!({"id": 1, "text": "ZEC 15 min", "x": 60.0, "y": 150.0, "w": 220.0, "h": 40.0})
        );

        let many = frame(
            (0..600)
                .map(|i| line("a", 10.0, f64::from(i) * 2.0, 10.0, 2.0))
                .collect(),
        );
        assert_eq!(elements(&many, PHONE).len(), MAX_ELEMENTS);
    }

    #[test]
    fn elements_are_given_in_the_screens_units() {
        let half = Screen {
            width: W / 2.0,
            height: H / 2.0,
        };
        let found = elements(&home(), half);
        let kalshi = found.iter().find(|e| e.text == "Kalshi").unwrap();
        assert_eq!(
            (kalshi.x, kalshi.y, kalshi.w, kalshi.h),
            (185.0, 440.0, 45.0, 11.0)
        );
    }

    #[test]
    fn tapping_an_element_taps_its_middle() {
        let found = elements(&home(), PHONE);
        let kalshi = found.iter().find(|e| e.text == "Kalshi").unwrap();
        assert_eq!(
            tap_element(&found, kalshi.id),
            Ok(Gesture::Tap { x: 415.0, y: 891.0 })
        );
        for missing in [0, 99] {
            let why = tap_element(&found, missing).unwrap_err();
            assert!(
                why.contains("no element") && why.contains("1 to 5"),
                "{why}"
            );
        }
        assert!(tap_element(&[], 1).is_err());
    }

    /// A phone that shows a script of screens.
    struct Phone {
        screens: Vec<Result<OcrFrame, String>>,
        reads: usize,
        done: Vec<Gesture>,
        fail_on: Option<usize>,
        waited: u64,
    }

    impl Phone {
        fn showing(screens: Vec<OcrFrame>) -> Phone {
            Phone {
                screens: screens.into_iter().map(Ok).collect(),
                reads: 0,
                done: Vec::new(),
                fail_on: None,
                waited: 0,
            }
        }
    }

    impl Hands for Phone {
        fn perform(&mut self, gesture: &Gesture, _screen: Screen) -> Result<(), String> {
            if self.fail_on == Some(self.done.len()) {
                return Err("Trading rails: no.".into());
            }
            self.done.push(gesture.clone());
            Ok(())
        }

        fn read_screen(&mut self) -> Result<OcrFrame, String> {
            let at = self.reads.min(self.screens.len() - 1);
            self.reads += 1;
            self.screens[at].clone()
        }

        fn wait_ms(&mut self, ms: u64) {
            self.waited += ms;
        }
    }

    #[test]
    fn opening_an_app_goes_home_and_taps_the_icon_above_its_name() {
        let mut phone = Phone::showing(vec![home()]);
        open_app(&mut phone, " kalshi ", PHONE, true).unwrap();
        assert_eq!(phone.done.len(), 2);
        assert_eq!(phone.done[0], Gesture::PressHome);
        let Gesture::Tap { x, y } = phone.done[1] else {
            panic!("expected a tap");
        };
        // Over Kalshi's own icon: under Fiverr's label (ends at 722),
        // above Kalshi's (starts at 880), in Kalshi's column.
        assert_eq!(x, 415.0);
        assert!(y > 760.0 && y < 860.0, "{y}");
        assert!(phone.waited >= SETTLE_MS);
    }

    #[test]
    fn the_exact_name_wins_over_a_longer_one() {
        let mut screen = home();
        screen
            .lines
            .insert(0, line("Kalshi Wallet", 70.0, 700.0, 150.0, 22.0));
        let mut phone = Phone::showing(vec![screen]);
        open_app(&mut phone, "Kalshi", PHONE, false).unwrap();
        assert!(matches!(phone.done[1], Gesture::Tap { x, .. } if x == 415.0));
        // A name inside another word is not a match.
        let mut phone = Phone::showing(vec![frame(vec![line(
            "Kalshify", 70.0, 700.0, 150.0, 22.0,
        )])]);
        assert!(open_app(&mut phone, "Kalshi", PHONE, false).is_err());
        // Two words are matched as two words.
        let mut phone = Phone::showing(vec![frame(vec![line(
            "App Store",
            70.0,
            700.0,
            150.0,
            22.0,
        )])]);
        open_app(&mut phone, "app  store", PHONE, false).unwrap();
    }

    #[test]
    fn an_app_on_another_page_is_found_through_search() {
        let mut page = home();
        page.lines.retain(|l| l.text != "Kalshi");
        let results = frame(vec![
            line("Top Hit", 60.0, 180.0, 100.0, 22.0),
            line("Kalshi", 170.0, 260.0, 90.0, 28.0),
            line("Kalshi: Prediction Markets", 60.0, 420.0, 330.0, 24.0),
            // The search field above the keyboard shows the name too.
            line("Kalshi", 110.0, 880.0, 90.0, 26.0),
            line("q w e r t y u i o p", 30.0, 1000.0, 680.0, 30.0),
        ]);
        let mut phone = Phone::showing(vec![page.clone(), results]);
        open_app(&mut phone, "Kalshi", PHONE, true).unwrap();
        assert_eq!(phone.done[0], Gesture::PressHome);
        assert!(matches!(
            phone.done[1],
            Gesture::Swipe { from_y, to_y, .. } if to_y > from_y
        ));
        assert_eq!(
            phone.done[2],
            Gesture::TypeText {
                text: "Kalshi".into()
            }
        );
        assert_eq!(phone.done[3], Gesture::Tap { x: 215.0, y: 274.0 });

        // Search not allowed: say where to put the app, touch nothing else.
        let mut phone = Phone::showing(vec![page.clone()]);
        let why = open_app(&mut phone, "Kalshi", PHONE, false).unwrap_err();
        assert!(why.contains("first page"), "{why}");
        assert_eq!(phone.done, [Gesture::PressHome]);

        // Not in search either.
        let mut phone = Phone::showing(vec![page.clone(), page]);
        let why = open_app(&mut phone, "Kalshi", PHONE, true).unwrap_err();
        assert!(why.contains("Is it installed"), "{why}");
        assert_eq!(phone.done.len(), 3);
    }

    #[test]
    fn opening_an_app_stops_at_the_first_thing_that_fails() {
        let mut phone = Phone::showing(vec![home()]);
        phone.fail_on = Some(1);
        assert_eq!(
            open_app(&mut phone, "Kalshi", PHONE, true),
            Err("Trading rails: no.".into())
        );
        assert_eq!(phone.done, [Gesture::PressHome]);

        let mut phone = Phone::showing(vec![home()]);
        phone.screens = vec![Err("no picture".into())];
        let why = open_app(&mut phone, "Kalshi", PHONE, true).unwrap_err();
        assert!(why.contains("couldn’t read the Home Screen"), "{why}");

        let mut phone = Phone::showing(vec![home()]);
        for bad in ["", "  ", "a\nb", &"x".repeat(61)] {
            assert!(open_app(&mut phone, bad, PHONE, true).is_err());
        }
        assert!(phone.done.is_empty());
    }
}
