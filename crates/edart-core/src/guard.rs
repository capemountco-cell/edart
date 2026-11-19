//! The trading rails: what the agent (or anyone driving the phone through
//! Edart) may do while trading mode is on.
//!
//! Before a gesture is sent, the text on the iPhone's screen is read on
//! this Mac and the gesture is judged against it here. The model is never
//! trusted to keep the limits: it can ask for anything, and only what
//! passes [`decide`] reaches the phone.
//!
//! The rules, in short:
//! - on the Home Screen, only a tap on the Kalshi icon;
//! - in any other app or on a screen that cannot be recognised, only
//!   Home;
//! - in Kalshi, anything except committing a buy, which needs the order
//!   sheet to read exactly the stake, the right market, room in the day's
//!   limits and (when asked for) the user's approval;
//! - selling is allowed, since it adds no risk.
//!
//! Everything errs towards refusing: an unreadable amount, a control
//! that cannot be told apart from a buy, or no text at all is a "no".

use serde::{Deserialize, Serialize};

use crate::gesture::{Gesture, Screen};
use crate::ledger::{Counters, EventKind, Ledger, LedgerEvent, Now, Side};
use crate::trading::{dollars, Cents, TradingLimits};

/// One line of text found on the screen. The box is in screenshot
/// pixels, origin top-left.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrLine {
    pub text: String,
    /// 0 to 1.
    pub confidence: f32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The text of one frame of the mirror.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrFrame {
    /// Size of the frame the boxes are measured in.
    pub width: f64,
    pub height: f64,
    pub lines: Vec<OcrLine>,
}

impl OcrLine {
    /// A line from macOS Vision, whose boxes are fractions of the image
    /// with the origin at the bottom-left.
    pub fn from_vision(
        text: String,
        confidence: f32,
        (nx, ny, nw, nh): (f64, f64, f64, f64),
        frame_width: f64,
        frame_height: f64,
    ) -> OcrLine {
        OcrLine {
            text,
            confidence,
            x: nx * frame_width,
            y: (1.0 - ny - nh) * frame_height,
            width: nw * frame_width,
            height: nh * frame_height,
        }
    }

    fn center_y(&self) -> f64 {
        self.y + self.height / 2.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    fn contains(&self, (x, y): (f64, f64)) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

/// What the screen is.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenKind {
    /// The iPhone's Home Screen; `kalshi` is where the Kalshi icon is,
    /// when its label is on this page.
    HomeScreen { kalshi: Option<Rect> },
    /// A Kalshi market with its Up and Down buttons.
    KalshiMarket,
    /// Kalshi's sheet for placing a buy.
    KalshiOrderSheet {
        side: Option<Side>,
        /// The amount entered, when it can be read for certain.
        amount: Option<Cents>,
    },
    /// Some other screen of Kalshi.
    KalshiOther,
    /// Another app, or nothing recognisable.
    Unknown,
}

/// Lines Vision is less sure of than this are ignored when recognising a
/// screen. (Vision's confidences are 0.3, 0.5 or 1.)
const MIN_CONFIDENCE: f32 = 0.3;
/// The amount must be read with at least this confidence.
const AMOUNT_CONFIDENCE: f32 = 0.5;

/// Lowercase, straight quotes, single spaces.
fn norm(text: &str) -> String {
    let lowered: String = text
        .chars()
        .map(|c| match c {
            '’' | '‘' => '\'',
            '“' | '”' => '"',
            c => c,
        })
        .flat_map(char::to_lowercase)
        .collect();
    lowered.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Letters and digits only, lowercase: how names are compared when the
/// text recognition may have dropped a space or a dot.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn has_word(normed: &str, word: &str) -> bool {
    normed
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w == word)
}

/// Names under the icons of apps that come with an iPhone. A screen with
/// several of them is the Home Screen.
const HOME_LABELS: &[&str] = &[
    "facetime",
    "calendar",
    "photos",
    "camera",
    "mail",
    "notes",
    "reminders",
    "clock",
    "tv",
    "podcasts",
    "app store",
    "maps",
    "health",
    "wallet",
    "settings",
    "phone",
    "safari",
    "messages",
    "music",
    "weather",
    "files",
    "books",
    "fitness",
    "contacts",
    "calculator",
    "translate",
    "shortcuts",
    "find my",
    "freeform",
    "journal",
    "passwords",
    "search",
    "app library",
    "utilities",
];

/// Words that mean the screen belongs to something that is not Kalshi,
/// whatever else is on it.
const FOREIGN: &[&str] = &[
    "robinhood",
    "coinbase",
    "polymarket",
    "webull",
    "buying power",
    "app store",
    "in-app purchases",
    "ratings",
    "airplane mode",
    "wi-fi",
];

/// Controls that move money in or out of the account. Never touched.
const MONEY_MOVES: &[&str] = &[
    "deposit",
    "withdraw",
    "add funds",
    "add cash",
    "add money",
    "transfer",
    "payment method",
    "apple pay",
    "link bank",
];

/// Everything the rules need to know about a screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub kind: ScreenKind,
    /// Where a touch would (or might) commit a buy.
    buy_zones: Vec<Rect>,
    /// Where a touch sells or cashes out.
    sell_zones: Vec<Rect>,
    /// Where a touch would move money.
    money_zones: Vec<Rect>,
    /// The squashed text of every line, for finding the market's name.
    squashed: Vec<String>,
    /// The market's name as the sheet shows it.
    market_text: Option<String>,
    width: f64,
    height: f64,
}

impl Reading {
    fn is_kalshi(&self) -> bool {
        matches!(
            self.kind,
            ScreenKind::KalshiMarket
                | ScreenKind::KalshiOrderSheet { .. }
                | ScreenKind::KalshiOther
        )
    }

    /// Whether the screen names the market trading is limited to.
    fn names_market(&self, market: &str) -> bool {
        let needle = squash(market);
        !needle.is_empty() && self.squashed.iter().any(|line| line.contains(&needle))
    }
}

/// "up · 22%", "down 78%": one of a market's two buttons.
fn side_button(normed: &str) -> bool {
    (normed.starts_with("up") || normed.starts_with("down"))
        && normed.ends_with('%')
        && normed.chars().any(|c| c.is_ascii_digit())
        && normed.chars().count() <= 16
}

fn buy_button(normed: &str) -> bool {
    normed == "buy"
        || normed.starts_with("buy ")
        || normed.starts_with("place order")
        || normed.starts_with("submit")
        || normed.starts_with("confirm")
        || normed.starts_with("purchase")
        || normed.contains("swipe to buy")
        || normed.contains("swipe up to")
}

fn sell_button(normed: &str) -> bool {
    normed == "sell"
        || normed.starts_with("sell ")
        || normed.starts_with("cash out")
        || normed.starts_with("close position")
}

/// The row a "Slide to …" control sits in: the knob is to the left of
/// its text and the pill is taller than the text, so the whole width and
/// some way above and below.
fn slide_row(line: &OcrLine, frame: &OcrFrame) -> Rect {
    let reach = line.height * 1.75 + frame.height * 0.01;
    Rect {
        x0: 0.0,
        y0: line.y - reach,
        x1: frame.width,
        y1: line.y + line.height + reach,
    }
}

/// A button around its text.
fn button_area(line: &OcrLine, frame: &OcrFrame) -> Rect {
    let side = frame.width * 0.12 + line.height;
    let reach = line.height * 1.5;
    Rect {
        x0: line.x - side,
        y0: line.y - reach,
        x1: line.x + line.width + side,
        y1: line.y + line.height + reach,
    }
}

/// Where the Kalshi icon is, given its label: the icon is above the
/// name, about a sixth of the screen wide.
fn icon_area(label: &OcrLine, frame: &OcrFrame) -> Rect {
    let center = label.x + label.width / 2.0;
    let half = frame.width * 0.11;
    Rect {
        x0: center - half,
        y0: label.y - frame.width * 0.2,
        x1: center + half,
        y1: label.y + label.height + frame.height * 0.005,
    }
}

/// Rows of at least three short labels side by side: an icon grid.
fn label_rows(lines: &[(&OcrLine, String)], frame: &OcrFrame) -> usize {
    let mut labels: Vec<&OcrLine> = lines
        .iter()
        .filter(|(_, normed)| {
            let n = normed.chars().count();
            (2..=16).contains(&n) && !normed.contains(['$', '%', ':'])
        })
        .map(|(line, _)| *line)
        .collect();
    labels.sort_by(|a, b| a.center_y().total_cmp(&b.center_y()));
    let near = frame.height * 0.012;
    let mut rows = 0;
    let mut i = 0;
    while i < labels.len() {
        let mut j = i + 1;
        while j < labels.len() && labels[j].center_y() - labels[i].center_y() <= near {
            j += 1;
        }
        if j - i >= 3 {
            rows += 1;
        }
        i = j;
    }
    rows
}

/// Read a screen: what it is and where its risky controls are.
pub fn read(frame: &OcrFrame) -> Reading {
    let lines: Vec<(&OcrLine, String)> = frame
        .lines
        .iter()
        .filter(|l| l.confidence >= MIN_CONFIDENCE && !l.text.trim().is_empty())
        .map(|l| (l, norm(&l.text)))
        .collect();
    let any = |test: &dyn Fn(&str) -> bool| lines.iter().any(|(_, n)| test(n));

    let mut buy_zones = Vec::new();
    let mut sell_zones = Vec::new();
    let mut money_zones = Vec::new();
    let mut slide_buy: Option<&OcrLine> = None;
    for (line, normed) in &lines {
        if normed.contains("slide to") {
            // A slide that is not plainly a sell is treated as a buy.
            if has_word(normed, "sell") || normed.contains("cash out") {
                sell_zones.push(slide_row(line, frame));
            } else {
                buy_zones.push(slide_row(line, frame));
                slide_buy = Some(line);
            }
        } else if buy_button(normed) {
            buy_zones.push(button_area(line, frame));
        } else if sell_button(normed) {
            sell_zones.push(button_area(line, frame));
        }
        if MONEY_MOVES.iter().any(|m| normed.contains(m)) {
            money_zones.push(button_area(line, frame));
        }
    }

    // Marks that only Kalshi's screens carry, and weaker ones that count
    // only alongside those.
    let has_kalshi_word = any(&|n| has_word(n, "kalshi"));
    let target_price = any(&|n| n.contains("target price"));
    let read_rules = any(&|n| n.contains("read rules"));
    let available = any(&|n| has_word(n, "available"));
    let both_sides = lines
        .iter()
        .any(|(_, n)| side_button(n) && n.starts_with("up"))
        && lines
            .iter()
            .any(|(_, n)| side_button(n) && n.starts_with("down"));
    let strong_besides_name = [
        target_price,
        read_rules,
        any(&|n| n == "combo"),
        any(&|n| n.contains("slide to buy") || n.contains("slide to sell")),
        any(&|n| n.contains("outcome is")),
        any(&|n| n.contains("view open market")),
        any(&|n| has_word(n, "predictions")),
        both_sides,
    ]
    .iter()
    .filter(|hit| **hit)
    .count();
    let weak = [
        any(&|n| has_word(n, "markets")),
        any(&|n| has_word(n, "position") || has_word(n, "positions")),
        any(&|n| n.contains("order book")),
        any(&|n| has_word(n, "portfolio")),
        any(&|n| has_word(n, "chance")),
        available,
        any(&|n| has_word(n, "target")),
        any(&|n| n.contains("15 min")),
        any(&|n| has_word(n, "payout")),
        any(&|n| has_word(n, "contracts")),
    ]
    .iter()
    .filter(|hit| **hit)
    .count();
    let strong = strong_besides_name + usize::from(has_kalshi_word);

    let home_labels = lines
        .iter()
        .filter(|(_, n)| HOME_LABELS.contains(&n.as_str()))
        .count();
    let rows = label_rows(&lines, frame);
    // Prices and percentages are what Kalshi's own lists are made of; a
    // Home Screen has at most a widget or two with one.
    let priced = lines.iter().filter(|(_, n)| n.contains(['$', '%'])).count();
    let looks_like_home = strong_besides_name == 0
        && priced <= 2
        && ((home_labels >= 2 && rows >= 2) || home_labels >= 4);
    let foreign = any(&|n| FOREIGN.iter().any(|f| n.contains(f)));
    // iOS asking something on Kalshi's behalf ("Kalshi" Would Like to…).
    let system_dialog = any(&|n| has_word(n, "kalshi") && n.contains("would like"));

    let kind = if looks_like_home {
        ScreenKind::HomeScreen {
            kalshi: lines
                .iter()
                .find(|(_, n)| n == "kalshi")
                .map(|(line, _)| icon_area(line, frame)),
        }
    } else if foreign {
        ScreenKind::Unknown
    } else if strong >= 2 || (strong >= 1 && weak >= 2) || system_dialog {
        if slide_buy.is_some() || (target_price && (available || read_rules)) {
            let side = slide_buy.and_then(|line| {
                let normed = norm(&line.text);
                match (has_word(&normed, "up"), has_word(&normed, "down")) {
                    (true, false) => Some(Side::Up),
                    (false, true) => Some(Side::Down),
                    _ => None,
                }
            });
            if slide_buy.is_none() {
                // The sheet is up but its slider cannot be read: treat
                // everything below the sheet's last line as the slider.
                let below = lines
                    .iter()
                    .filter(|(_, n)| n.contains("read rules") || has_word(n, "available"))
                    .map(|(line, _)| line.y + line.height)
                    .fold(f64::NAN, f64::max);
                buy_zones.push(Rect {
                    x0: 0.0,
                    y0: if below.is_nan() {
                        frame.height * 0.75
                    } else {
                        below.min(frame.height * 0.75)
                    },
                    x1: frame.width,
                    y1: frame.height,
                });
            }
            ScreenKind::KalshiOrderSheet {
                side,
                amount: order_amount(frame, slide_buy),
            }
        } else if both_sides {
            ScreenKind::KalshiMarket
        } else {
            ScreenKind::KalshiOther
        }
    } else {
        ScreenKind::Unknown
    };

    let market_text = lines
        .iter()
        .find(|(_, n)| n.contains("15 min") || (n.contains('·') && has_word(n, "target")))
        .map(|(line, _)| {
            let name = line.text.split(['·', '•']).next().unwrap_or("").trim();
            name.chars().take(40).collect::<String>()
        })
        .filter(|name| !name.is_empty());

    Reading {
        kind,
        buy_zones,
        sell_zones,
        money_zones,
        squashed: lines.iter().map(|(l, _)| squash(&l.text)).collect(),
        market_text,
        width: frame.width,
        height: frame.height,
    }
}

/// What the screen is. See [`read`] for the controls on it.
pub fn classify(frame: &OcrFrame) -> ScreenKind {
    read(frame).kind
}

/// Text that is only an amount of money: "$0.10", "$", "0.10", ".10".
fn money_token(text: &str) -> Option<String> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let ok = !compact.is_empty()
        && compact
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '$' | '.' | ',' | 'O' | 'o' | 'S'));
    ok.then_some(compact)
}

/// "$0.10" → 10. Exactly: a dollar sign, digits, and at most two
/// decimals. Anything else is not an amount.
pub fn parse_amount(text: &str) -> Option<Cents> {
    let compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if matches!(c, 'O' | 'o') { '0' } else { c })
        .collect();
    // The recogniser sometimes reads "$" as "S".
    let rest = compact
        .strip_prefix('$')
        .or_else(|| compact.strip_prefix('S'))?;
    let (whole, cents) = match rest.split_once('.') {
        Some((whole, cents)) => (whole, cents),
        None => (rest, ""),
    };
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || !digits(cents) || whole.len() > 6 || cents.len() > 2 {
        return None;
    }
    if whole.is_empty() && cents.is_empty() {
        return None;
    }
    let whole: Cents = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let cents: Cents = match cents.len() {
        0 => 0,
        1 => cents.parse::<Cents>().ok()? * 10,
        _ => cents.parse().ok()?,
    };
    whole.checked_mul(100)?.checked_add(cents)
}

/// The amount on the order sheet, or `None` when it cannot be read for
/// certain.
///
/// The amount is the large figure between the "Target Price" line and
/// the "… chance" line (the keypad, whose keys are also digits, is below
/// that). The recogniser may split it ("$" and "0.10"), so the pieces on
/// its row are joined left to right. Anything else on that row, any
/// second figure in that part of the sheet, or a piece read with low
/// confidence, makes the amount unreadable.
fn order_amount(frame: &OcrFrame, slide: Option<&OcrLine>) -> Option<Cents> {
    let lines: Vec<(&OcrLine, String)> = frame
        .lines
        .iter()
        .filter(|l| !l.text.trim().is_empty())
        .map(|l| (l, norm(&l.text)))
        .collect();
    let top = lines
        .iter()
        .filter(|(_, n)| n.contains("target price"))
        .map(|(l, _)| l.y + l.height)
        .fold(0.0, f64::max);
    let chance = lines
        .iter()
        .filter(|(l, n)| has_word(n, "chance") && l.y > top)
        .map(|(l, _)| l.y)
        .fold(f64::NAN, f64::min);
    let anchored = !chance.is_nan();
    let bottom = if anchored {
        chance
    } else {
        // No "chance" line: everything above the sheet's footer, and
        // only figures that carry their own "$" count (keypad keys do
        // not).
        lines
            .iter()
            .filter(|(_, n)| has_word(n, "available") || n.contains("read rules"))
            .map(|(l, _)| l.y)
            .chain(slide.map(|l| l.y))
            .fold(frame.height, f64::min)
    };
    let between = |l: &OcrLine| l.center_y() > top && l.center_y() < bottom;

    let figures: Vec<(&OcrLine, String)> = lines
        .iter()
        .filter(|(l, _)| between(l))
        .filter_map(|(l, _)| money_token(&l.text).map(|token| (*l, token)))
        .collect();
    let main = figures
        .iter()
        .filter(|(_, token)| anchored || token.starts_with(['$', 'S']))
        .map(|(l, _)| *l)
        .max_by(|a, b| a.height.total_cmp(&b.height))?;
    let on_row = |l: &OcrLine| {
        l.center_y() >= main.y - main.height * 0.25 && l.center_y() <= main.y + main.height * 1.25
    };
    // Nothing but pieces of the amount may share its row…
    if lines
        .iter()
        .any(|(l, _)| on_row(l) && money_token(&l.text).is_none())
    {
        return None;
    }
    // …and there may be no other figure to confuse it with.
    if figures
        .iter()
        .any(|(l, token)| !on_row(l) && (anchored || token.contains('$')))
    {
        return None;
    }
    let mut row: Vec<&(&OcrLine, String)> = figures.iter().filter(|(l, _)| on_row(l)).collect();
    if row.iter().any(|(l, _)| l.confidence < AMOUNT_CONFIDENCE) {
        return None;
    }
    row.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
    let joined: String = row.iter().map(|(_, token)| token.as_str()).collect();
    parse_amount(&joined)
}

/// A buy the rails are being asked to let through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuyInfo {
    pub side: Option<Side>,
    pub stake_cents: Cents,
    pub market: String,
}

impl BuyInfo {
    /// "Up, $0.10, ZEC 15 min".
    pub fn summary(&self) -> String {
        format!(
            "{}, {}, {}",
            self.side.map_or("Side not read", Side::label),
            dollars(self.stake_cents),
            self.market
        )
    }
}

/// The user said yes to one buy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub buy: BuyInfo,
    /// When, seconds since 1970.
    pub at: i64,
}

/// An approval is good for this long.
pub const APPROVAL_SECS: i64 = 60;

/// What an allowed gesture is, for the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// Nothing worth writing down.
    Plain,
    Buy(BuyInfo),
    Sell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow(Note),
    Block {
        reason: String,
        /// The screen may only have been caught mid-change; look again
        /// before giving up.
        retry: bool,
    },
    /// Allowed once the user approves this buy.
    NeedsConfirm {
        buy: BuyInfo,
        summary: String,
    },
}

fn block(reason: impl Into<String>) -> Decision {
    Decision::Block {
        reason: reason.into(),
        retry: false,
    }
}

/// Reason given when a gesture is refused because the phone is not in
/// Kalshi.
pub const LEFT_KALSHI: &str = "Left Kalshi";

/// Judge one gesture against the screen it would land on.
///
/// `text` is the screen's text, or why it could not be read. `screen` is
/// the size the gesture's coordinates are measured in; they are scaled
/// to the frame the text was read from.
pub fn decide(
    limits: &TradingLimits,
    counters: &Counters,
    text: Result<&OcrFrame, &str>,
    gesture: &Gesture,
    screen: Screen,
    approval: Option<&Approval>,
    now: Now,
) -> Decision {
    // Home is always safe, and is the way back.
    if matches!(gesture, Gesture::PressHome) {
        return Decision::Allow(Note::Plain);
    }
    let frame = match text {
        Ok(frame) => frame,
        Err(why) => {
            return Decision::Block {
                reason: format!(
                    "Edart can’t read the iPhone’s screen to check this action ({why})"
                ),
                retry: true,
            }
        }
    };
    let reading = read(frame);
    let scale = |x: f64, y: f64| {
        (
            x / screen.width.max(1.0) * frame.width,
            y / screen.height.max(1.0) * frame.height,
        )
    };
    // Where the finger goes down, and for a swipe where it ends up.
    let (touch, swipe_to) = match gesture {
        Gesture::Tap { x, y } | Gesture::DoubleTap { x, y } | Gesture::LongPress { x, y, .. } => {
            (Some(scale(*x, *y)), None)
        }
        Gesture::Swipe {
            from_x,
            from_y,
            to_x,
            to_y,
            ..
        } => (Some(scale(*from_x, *from_y)), Some(scale(*to_x, *to_y))),
        Gesture::TypeText { .. } | Gesture::PressHome => (None, None),
    };
    if touch.is_some_and(|(x, y)| !(x.is_finite() && y.is_finite())) {
        return block("That gesture has no position");
    }

    match &reading.kind {
        ScreenKind::Unknown => Decision::Block {
            reason: format!(
                "{LEFT_KALSHI}: this screen isn’t Kalshi, so only Home is allowed"
            ),
            retry: true,
        },
        ScreenKind::HomeScreen { kalshi } => match (gesture, kalshi, touch) {
            (Gesture::Tap { .. }, Some(icon), Some(point)) if icon.contains(point) => {
                Decision::Allow(Note::Plain)
            }
            (_, None, _) => block(
                "On the Home Screen only the Kalshi icon may be tapped, and it isn’t on this page. Put Kalshi on the first Home Screen page",
            ),
            _ => block("On the Home Screen only the Kalshi icon may be tapped"),
        },
        ScreenKind::KalshiMarket | ScreenKind::KalshiOrderSheet { .. } | ScreenKind::KalshiOther => {
            debug_assert!(reading.is_kalshi());
            let Some(point) = touch else {
                // Typing.
                return if matches!(reading.kind, ScreenKind::KalshiOrderSheet { .. }) {
                    block(
                        "Typing is not allowed on the order sheet: the amount must be entered with the sheet’s own keypad",
                    )
                } else {
                    Decision::Allow(Note::Plain)
                };
            };
            if reading.money_zones.iter().any(|zone| zone.contains(point)) {
                return block("Moving money in or out of the account is not allowed");
            }
            let in_buy_zone = reading.buy_zones.iter().any(|zone| zone.contains(point));
            let in_sell_zone = reading.sell_zones.iter().any(|zone| zone.contains(point));
            // A long drag to the right along the bottom of the screen is
            // what a "slide to confirm" takes, whether or not its label
            // was read.
            let slide_like = swipe_to.is_some_and(|(to_x, to_y)| {
                to_x - point.0 >= reading.width * 0.3
                    && (to_y - point.1).abs() <= reading.height * 0.08
                    && point.1 >= reading.height * 0.7
            });
            if in_buy_zone || (slide_like && !in_sell_zone) {
                return decide_buy(limits, counters, &reading, approval, now);
            }
            if in_sell_zone {
                return Decision::Allow(Note::Sell);
            }
            Decision::Allow(Note::Plain)
        }
    }
}

/// The checks a buy has to pass, in the order the user would fix them.
fn decide_buy(
    limits: &TradingLimits,
    counters: &Counters,
    reading: &Reading,
    approval: Option<&Approval>,
    now: Now,
) -> Decision {
    let stake = limits.stake_cents;
    let ScreenKind::KalshiOrderSheet { side, amount } = &reading.kind else {
        return block(
            "This looks like it would place an order, and Edart can’t check the amount on this screen",
        );
    };
    if !reading.names_market(&limits.market) {
        return block(format!(
            "Buying is only allowed in the {} market, and this order isn’t for it",
            limits.market
        ));
    }
    match amount {
        None => {
            return block(format!(
                "Edart can’t read the order amount for certain, so the buy was refused (it must be exactly {})",
                dollars(stake)
            ))
        }
        Some(amount) if *amount != stake => {
            return block(format!(
                "The order amount is {}, and a buy must be exactly {}",
                dollars(*amount),
                dollars(stake)
            ))
        }
        Some(_) => {}
    }
    if counters.trades >= limits.max_trades_per_day {
        return block(format!(
            "Today’s limit of {} trades is used up",
            limits.max_trades_per_day
        ));
    }
    if counters.spent_cents.saturating_add(stake) > limits.max_loss_cents_per_day {
        return block(format!(
            "Today’s loss limit of {} is used up ({} already spent)",
            dollars(limits.max_loss_cents_per_day),
            dollars(counters.spent_cents)
        ));
    }
    let buy = BuyInfo {
        side: *side,
        stake_cents: stake,
        market: reading
            .market_text
            .clone()
            .unwrap_or_else(|| limits.market.clone()),
    };
    if limits.confirm_each_buy {
        let approved = approval
            .is_some_and(|a| a.buy == buy && now.unix >= a.at && now.unix - a.at <= APPROVAL_SECS);
        if !approved {
            let summary = buy.summary();
            return Decision::NeedsConfirm { buy, summary };
        }
    }
    Decision::Allow(Note::Buy(buy))
}

/// What the user answered when asked to confirm a buy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    Approved,
    Declined,
    /// Nobody answered in time.
    TimedOut,
    /// The run was stopped while waiting.
    Stopped,
}

/// Everything the rails need from the world. The app implements it with
/// the real phone; tests with a script.
pub trait Rails {
    fn now(&self) -> Now;
    /// The text on the iPhone's screen right now.
    fn read_screen(&mut self) -> Result<OcrFrame, String>;
    fn load_ledger(&mut self) -> Result<Ledger, String>;
    fn save_ledger(&mut self, ledger: &Ledger) -> Result<(), String>;
    fn wait_ms(&mut self, ms: u64);
    /// Ask the user to approve a buy, and wait for the answer.
    fn confirm(&mut self, buy: &BuyInfo, summary: &str) -> Confirmation;
    /// Send the gesture to the phone.
    fn perform(&mut self, gesture: &Gesture, screen: Screen) -> Result<(), String>;
}

/// Why a gesture did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateError {
    /// The rails refused it.
    Blocked(String),
    /// It was allowed, and sending it failed.
    Failed(String),
}

impl GateError {
    pub fn message(&self) -> String {
        match self {
            GateError::Blocked(why) => format!("Trading rails: {why}."),
            GateError::Failed(why) => why.clone(),
        }
    }
}

/// Looks at the screen again this many times when it may have been
/// caught mid-change.
const RETRIES: u32 = 2;
const RETRY_WAIT_MS: u64 = 700;
/// A buy may be put to the user this many times in one go (the screen
/// can change while they decide).
const MAX_CONFIRMS: u32 = 2;

/// Write a refusal in the ledger, when there is a ledger to write in.
fn note_refusal(rails: &mut impl Rails, ledger: Result<Ledger, String>, now: Now, reason: &str) {
    if let Ok(mut ledger) = ledger {
        let event = LedgerEvent {
            time: 0,
            day: String::new(),
            kind: EventKind::Blocked,
            side: None,
            stake_cents: 0,
            market: None,
            reason: Some(reason.to_string()),
        };
        if ledger.record(now, event) {
            // The refusal stands whether or not it could be written.
            let _ = rails.save_ledger(&ledger);
        }
    }
}

/// Perform a gesture if the rails allow it. With trading mode off this
/// is `perform` and nothing else.
pub fn gate(
    rails: &mut impl Rails,
    limits: &TradingLimits,
    gesture: &Gesture,
    screen: Screen,
) -> Result<(), GateError> {
    if !limits.enabled {
        return rails.perform(gesture, screen).map_err(GateError::Failed);
    }
    let mut approval: Option<Approval> = None;
    let mut retries = 0;
    let mut confirms = 0;
    loop {
        let now = rails.now();
        let text = rails.read_screen();
        let ledger = rails.load_ledger();
        // A ledger that cannot be read counts as a day already used up:
        // nothing can be bought, everything else goes on.
        let counters = match &ledger {
            Ok(ledger) => ledger.counters(&now.day()),
            Err(_) => Counters {
                day: now.day(),
                trades: u32::MAX,
                spent_cents: Cents::MAX,
            },
        };
        let decision = decide(
            limits,
            &counters,
            text.as_ref().map_err(String::as_str),
            gesture,
            screen,
            approval.as_ref(),
            now,
        );
        match decision {
            Decision::Allow(Note::Plain) => {
                return rails.perform(gesture, screen).map_err(GateError::Failed);
            }
            Decision::Allow(Note::Sell) => {
                rails.perform(gesture, screen).map_err(GateError::Failed)?;
                if let Ok(mut ledger) = ledger {
                    let event = LedgerEvent {
                        time: 0,
                        day: String::new(),
                        kind: EventKind::Sell,
                        side: None,
                        stake_cents: 0,
                        market: read_market(&text),
                        reason: None,
                    };
                    ledger.record(now, event);
                    // A sell that isn't written down costs nothing.
                    let _ = rails.save_ledger(&ledger);
                }
                return Ok(());
            }
            Decision::Allow(Note::Buy(buy)) => {
                // Written down before the finger moves: a buy missing
                // from the ledger would not count against the limits.
                let saved = ledger.and_then(|mut ledger| {
                    let event = LedgerEvent {
                        time: 0,
                        day: String::new(),
                        kind: EventKind::Buy,
                        side: buy.side,
                        stake_cents: buy.stake_cents,
                        market: Some(buy.market.clone()),
                        reason: None,
                    };
                    ledger.record(now, event);
                    rails.save_ledger(&ledger)
                });
                if let Err(why) = saved {
                    return Err(GateError::Blocked(format!("{why}, so the buy was refused")));
                }
                return rails.perform(gesture, screen).map_err(GateError::Failed);
            }
            Decision::Block { reason, retry } => {
                if retry && retries < RETRIES {
                    retries += 1;
                    rails.wait_ms(RETRY_WAIT_MS);
                    continue;
                }
                // When the ledger is what is wrong, say so rather than
                // "the limit is used up".
                let reason = match &ledger {
                    Err(why) if reason.starts_with("Today’s") => {
                        format!("{why}, so nothing can be bought")
                    }
                    _ => reason,
                };
                note_refusal(rails, ledger, now, &reason);
                return Err(GateError::Blocked(reason));
            }
            Decision::NeedsConfirm { buy, summary } => {
                let answer = if confirms < MAX_CONFIRMS {
                    confirms += 1;
                    rails.confirm(&buy, &summary)
                } else {
                    Confirmation::Declined
                };
                let reason = match answer {
                    Confirmation::Approved => {
                        approval = Some(Approval {
                            buy,
                            at: rails.now().unix,
                        });
                        // Look at the screen again: it may have changed
                        // while the user decided.
                        continue;
                    }
                    Confirmation::TimedOut => {
                        format!("The buy ({summary}) wasn’t approved within a minute")
                    }
                    Confirmation::Stopped => format!("The buy ({summary}) was cancelled"),
                    Confirmation::Declined => format!("The buy ({summary}) wasn’t approved"),
                };
                note_refusal(rails, ledger, now, &reason);
                return Err(GateError::Blocked(reason));
            }
        }
    }
}

/// The market's name on a screen, for a ledger line.
fn read_market(text: &Result<OcrFrame, String>) -> Option<String> {
    text.as_ref().ok().and_then(|frame| read(frame).market_text)
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
    /// 2026-11-19 20:00:00 UTC.
    const NOW: Now = Now {
        unix: 1_795_118_400,
        utc_offset: 0,
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

    /// A row of four icon labels.
    fn icon_row(y: f64, names: [&str; 4]) -> Vec<OcrLine> {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| line(name, 70.0 + 170.0 * i as f64, y, 90.0, 22.0))
            .collect()
    }

    fn home_screen() -> OcrFrame {
        let mut lines = vec![line("9:41", 60.0, 40.0, 70.0, 26.0)];
        lines.extend(icon_row(
            300.0,
            ["FaceTime", "Calendar", "Photos", "Camera"],
        ));
        lines.extend(icon_row(520.0, ["Mail", "Notes", "Reminders", "Clock"]));
        lines.extend(icon_row(740.0, ["TV", "App Store", "Maps", "Wallet"]));
        // Kalshi is the second icon of the last row: label at x 240..330.
        lines.extend(icon_row(
            960.0,
            ["Settings", "Kalshi", "Robinhood", "Podcasts"],
        ));
        lines.push(line("Search", 330.0, 1380.0, 80.0, 24.0));
        frame(lines)
    }

    fn home_screen_without_kalshi() -> OcrFrame {
        let mut screen = home_screen();
        screen.lines.retain(|l| l.text != "Kalshi");
        screen
    }

    fn market_page() -> OcrFrame {
        frame(vec![
            line("ZEC 15 min", 60.0, 150.0, 220.0, 40.0),
            line("8 PM target $916.9885", 60.0, 210.0, 300.0, 26.0),
            line("Now ▼ $0.9070 $916.0815", 60.0, 250.0, 330.0, 26.0),
            line("05:51", 600.0, 160.0, 80.0, 30.0),
            line("Up · 22%", 110.0, 1380.0, 150.0, 30.0),
            line("Down · 78%", 470.0, 1380.0, 170.0, 30.0),
            line("3.66x", 150.0, 1420.0, 60.0, 20.0),
            line("1.31x", 520.0, 1420.0, 60.0, 20.0),
            line("COMBO", 620.0, 1300.0, 80.0, 20.0),
        ])
    }

    /// The order sheet with `amount` as recognised pieces: (text, x).
    fn order_sheet_with(amount: &[(&str, f64)], side: &str) -> OcrFrame {
        let mut lines = vec![
            line("ZEC 15 min · $916.9885 target", 150.0, 140.0, 430.0, 30.0),
            line("Target Price: $916.9885", 210.0, 200.0, 310.0, 26.0),
            line("UP", 250.0, 290.0, 50.0, 26.0),
            line("DOWN", 420.0, 290.0, 90.0, 26.0),
            line("24% chance · 3.66x", 250.0, 540.0, 240.0, 26.0),
            line("Predictions · $15.86 available", 190.0, 1240.0, 350.0, 24.0),
            line("Read rules", 310.0, 1290.0, 120.0, 24.0),
            line(&format!("Slide to buy {side}"), 250.0, 1440.0, 260.0, 30.0),
        ];
        for (text, x) in amount {
            lines.push(line(text, *x, 420.0, 40.0 * text.len() as f64, 90.0));
        }
        // The keypad: digits are text too.
        for (row, keys) in [
            ["1", "2", "3"],
            ["4", "5", "6"],
            ["7", "8", "9"],
            [".", "0", "⌫"],
        ]
        .iter()
        .enumerate()
        {
            for (col, key) in keys.iter().enumerate() {
                lines.push(line(
                    key,
                    150.0 + 200.0 * col as f64,
                    680.0 + 140.0 * row as f64,
                    30.0,
                    44.0,
                ));
            }
        }
        frame(lines)
    }

    fn order_sheet(amount: &str) -> OcrFrame {
        order_sheet_with(&[(amount, 280.0)], "Up")
    }

    fn settled_page() -> OcrFrame {
        frame(vec![
            line("ZEC 15 min", 60.0, 150.0, 220.0, 40.0),
            line("Outcome is Up", 240.0, 700.0, 260.0, 36.0),
            line("View open market", 240.0, 1400.0, 260.0, 30.0),
        ])
    }

    fn position_page() -> OcrFrame {
        frame(vec![
            line("ZEC 15 min", 60.0, 150.0, 220.0, 40.0),
            line("Your position", 60.0, 400.0, 200.0, 28.0),
            line("Up · 10 contracts", 60.0, 450.0, 240.0, 24.0),
            line("Predictions", 60.0, 90.0, 140.0, 24.0),
            line("Slide to sell", 270.0, 1440.0, 220.0, 30.0),
        ])
    }

    fn portfolio_page() -> OcrFrame {
        frame(vec![
            line("Kalshi", 60.0, 90.0, 120.0, 34.0),
            line("Portfolio", 60.0, 180.0, 160.0, 30.0),
            line("Predictions · $15.86 available", 60.0, 240.0, 350.0, 24.0),
            line("Deposit", 120.0, 330.0, 110.0, 28.0),
            line("Withdraw", 480.0, 330.0, 130.0, 28.0),
            line("Positions", 60.0, 460.0, 130.0, 26.0),
            line("Sell", 600.0, 560.0, 60.0, 26.0),
            line("Markets", 150.0, 1530.0, 100.0, 20.0),
        ])
    }

    fn kalshi_dialog() -> OcrFrame {
        frame(vec![
            line("“Kalshi” Would Like to Send You", 150.0, 640.0, 440.0, 30.0),
            line("Notifications", 280.0, 680.0, 180.0, 30.0),
            line("Don’t Allow", 170.0, 860.0, 150.0, 28.0),
            line("Allow", 500.0, 860.0, 80.0, 28.0),
        ])
    }

    fn notes_app() -> OcrFrame {
        frame(vec![
            line("Notes", 60.0, 110.0, 90.0, 30.0),
            line("Shopping list", 60.0, 200.0, 260.0, 40.0),
            line("Milk, eggs, bread", 60.0, 270.0, 300.0, 28.0),
            line("Done", 640.0, 110.0, 60.0, 26.0),
        ])
    }

    fn settings_app() -> OcrFrame {
        frame(vec![
            line("Settings", 60.0, 150.0, 200.0, 50.0),
            line("Airplane Mode", 130.0, 420.0, 200.0, 28.0),
            line("Wi-Fi", 130.0, 500.0, 80.0, 28.0),
            line("Bluetooth", 130.0, 580.0, 140.0, 28.0),
            line("Cellular", 130.0, 660.0, 120.0, 28.0),
            line("General", 130.0, 900.0, 110.0, 28.0),
        ])
    }

    fn robinhood_app() -> OcrFrame {
        frame(vec![
            line("ZEC 15 min up or down?", 60.0, 150.0, 400.0, 36.0),
            line("Buying power $120.00", 60.0, 220.0, 300.0, 24.0),
            line("Yes 22¢", 110.0, 1300.0, 120.0, 30.0),
            line("No 78¢", 470.0, 1300.0, 120.0, 30.0),
            line("Review order", 280.0, 1440.0, 180.0, 30.0),
            line("Predictions", 60.0, 90.0, 140.0, 24.0),
            line("Markets", 250.0, 90.0, 110.0, 24.0),
        ])
    }

    fn app_store() -> OcrFrame {
        frame(vec![
            line("Kalshi", 250.0, 180.0, 130.0, 40.0),
            line("Kalshi: Prediction Markets", 250.0, 230.0, 330.0, 24.0),
            line("Get", 280.0, 300.0, 60.0, 28.0),
            line("12K Ratings", 60.0, 420.0, 130.0, 20.0),
            line("4.8", 90.0, 450.0, 50.0, 30.0),
            line("Today", 60.0, 1540.0, 60.0, 18.0),
            line("Games", 200.0, 1540.0, 60.0, 18.0),
            line("Apps", 340.0, 1540.0, 50.0, 18.0),
            line("Arcade", 470.0, 1540.0, 70.0, 18.0),
            line("Search", 610.0, 1540.0, 70.0, 18.0),
        ])
    }

    fn limits() -> TradingLimits {
        TradingLimits {
            enabled: true,
            ..TradingLimits::default()
        }
    }

    fn fresh() -> Counters {
        Counters {
            day: NOW.day(),
            trades: 0,
            spent_cents: 0,
        }
    }

    fn tap(x: f64, y: f64) -> Gesture {
        Gesture::Tap { x, y }
    }

    fn swipe(from: (f64, f64), to: (f64, f64)) -> Gesture {
        Gesture::Swipe {
            from_x: from.0,
            from_y: from.1,
            to_x: to.0,
            to_y: to.1,
            ms: None,
        }
    }

    /// The slide a model would make on the order sheet's slider.
    fn slide_to_buy() -> Gesture {
        swipe((80.0, 1455.0), (660.0, 1455.0))
    }

    fn judge(screen: &OcrFrame, gesture: &Gesture) -> Decision {
        decide(&limits(), &fresh(), Ok(screen), gesture, PHONE, None, NOW)
    }

    fn allowed(decision: &Decision) -> bool {
        matches!(decision, Decision::Allow(_))
    }

    fn reason(decision: &Decision) -> &str {
        match decision {
            Decision::Block { reason, .. } => reason,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn approve(buy: &BuyInfo, at: i64) -> Approval {
        Approval {
            buy: buy.clone(),
            at,
        }
    }

    fn zec_up() -> BuyInfo {
        BuyInfo {
            side: Some(Side::Up),
            stake_cents: 10,
            market: "ZEC 15 min".into(),
        }
    }

    // ---- Reading the screen -------------------------------------------

    #[test]
    fn vision_boxes_are_turned_the_right_way_up() {
        // A box a tenth of the way up from the bottom.
        let l = OcrLine::from_vision("Slide".into(), 1.0, (0.25, 0.1, 0.5, 0.05), W, H);
        assert_eq!((l.x, l.width), (184.0, 368.0));
        assert!((l.y - 0.85 * H).abs() < 1e-9);
        assert!((l.height - 0.05 * H).abs() < 1e-9);
        // The top-left corner.
        let l = OcrLine::from_vision("9:41".into(), 0.5, (0.0, 0.95, 0.1, 0.05), W, H);
        assert!(l.y.abs() < 1e-9 && l.x == 0.0);
    }

    #[test]
    fn every_screen_is_told_apart() {
        assert!(matches!(
            classify(&home_screen()),
            ScreenKind::HomeScreen { kalshi: Some(_) }
        ));
        assert_eq!(
            classify(&home_screen_without_kalshi()),
            ScreenKind::HomeScreen { kalshi: None }
        );
        assert_eq!(classify(&market_page()), ScreenKind::KalshiMarket);
        assert_eq!(
            classify(&order_sheet("$0.10")),
            ScreenKind::KalshiOrderSheet {
                side: Some(Side::Up),
                amount: Some(10)
            }
        );
        assert_eq!(
            classify(&order_sheet_with(&[("$0", 320.0)], "Down")),
            ScreenKind::KalshiOrderSheet {
                side: Some(Side::Down),
                amount: Some(0)
            }
        );
        assert_eq!(classify(&settled_page()), ScreenKind::KalshiOther);
        assert_eq!(classify(&position_page()), ScreenKind::KalshiOther);
        assert_eq!(classify(&portfolio_page()), ScreenKind::KalshiOther);
        assert_eq!(classify(&kalshi_dialog()), ScreenKind::KalshiOther);
        for other in [notes_app(), settings_app(), robinhood_app(), app_store()] {
            assert_eq!(classify(&other), ScreenKind::Unknown, "{other:?}");
        }
        assert_eq!(classify(&frame(vec![])), ScreenKind::Unknown);
    }

    #[test]
    fn a_market_page_is_recognised_without_its_combo_link() {
        let mut page = market_page();
        page.lines.retain(|l| l.text != "COMBO");
        assert_eq!(classify(&page), ScreenKind::KalshiMarket);
    }

    #[test]
    fn text_the_recogniser_is_unsure_of_is_ignored() {
        let mut page = settled_page();
        for l in &mut page.lines {
            l.confidence = 0.1;
        }
        assert_eq!(classify(&page), ScreenKind::Unknown);
    }

    #[test]
    fn amounts_parse_exactly() {
        for (text, cents) in [
            ("$0.10", Some(10)),
            ("$0.1", Some(10)),
            ("$.10", Some(10)),
            ("$ 0.10", Some(10)),
            ("$O.10", Some(10)),
            ("S0.10", Some(10)),
            ("$0", Some(0)),
            ("$0.", Some(0)),
            ("$1", Some(100)),
            ("$1.00", Some(100)),
            ("$10", Some(1000)),
            ("$10.00", Some(1000)),
            ("$0.01", Some(1)),
            ("$15.86", Some(1586)),
            // Not amounts.
            ("0.10", None),
            ("$", None),
            ("$.", None),
            ("$0.100", None),
            ("$0,10", None),
            ("$1,000", None),
            ("$0.1O0", None),
            ("$-0.10", None),
            ("$0.10x", None),
            ("$$0.10", None),
            ("0.10$", None),
            ("$1e1", None),
            ("", None),
            ("$99999999", None),
        ] {
            assert_eq!(parse_amount(text), cents, "{text:?}");
        }
    }

    fn amount_of(screen: &OcrFrame) -> Option<Cents> {
        match classify(screen) {
            ScreenKind::KalshiOrderSheet { amount, .. } => amount,
            other => panic!("not an order sheet: {other:?}"),
        }
    }

    #[test]
    fn the_order_amount_is_read_whole_or_not_at_all() {
        assert_eq!(amount_of(&order_sheet("$0.10")), Some(10));
        assert_eq!(amount_of(&order_sheet("$0.1")), Some(10));
        assert_eq!(amount_of(&order_sheet("$0")), Some(0));
        assert_eq!(amount_of(&order_sheet("$1")), Some(100));
        assert_eq!(amount_of(&order_sheet("$10.00")), Some(1000));
        // Split by the recogniser into "$" and "0.10", or "$0" and ".10".
        assert_eq!(
            amount_of(&order_sheet_with(&[("$", 260.0), ("0.10", 310.0)], "Up")),
            Some(10)
        );
        assert_eq!(
            amount_of(&order_sheet_with(&[("$0", 260.0), (".10", 350.0)], "Up")),
            Some(10)
        );
        // Pieces are joined left to right whatever order they came in.
        assert_eq!(
            amount_of(&order_sheet_with(&[("0.00", 330.0), ("$1", 250.0)], "Up")),
            Some(1000)
        );
        // The "$" was not read at all: not certain enough.
        assert_eq!(amount_of(&order_sheet("0.10")), None);
        // No amount on the sheet.
        assert_eq!(amount_of(&order_sheet_with(&[], "Up")), None);
        // Something that isn't a number shares the amount's row.
        assert_eq!(
            amount_of(&order_sheet_with(&[("$0.10", 280.0), ("USD", 520.0)], "Up")),
            None
        );
        assert_eq!(
            amount_of(&order_sheet_with(&[("1O", 180.0), ("$0.10", 280.0)], "Up")),
            None
        );
    }

    #[test]
    fn a_shaky_or_doubled_amount_is_unreadable() {
        let mut sheet = order_sheet_with(&[("$", 260.0), ("0.10", 310.0)], "Up");
        sheet
            .lines
            .iter_mut()
            .find(|l| l.text == "$")
            .unwrap()
            .confidence = 0.3;
        assert_eq!(amount_of(&sheet), None);

        // A second figure between the target price and the chance line.
        let mut sheet = order_sheet("$0.10");
        sheet.lines.push(line("$5", 560.0, 330.0, 60.0, 40.0));
        assert_eq!(amount_of(&sheet), None);
    }

    #[test]
    fn the_amount_is_found_without_the_chance_line_but_never_from_the_keypad() {
        let mut sheet = order_sheet("$0.10");
        sheet.lines.retain(|l| !l.text.contains("chance"));
        assert_eq!(amount_of(&sheet), Some(10));
        // With no "$" figure, a keypad key is not mistaken for one.
        let mut sheet = order_sheet_with(&[], "Up");
        sheet.lines.retain(|l| !l.text.contains("chance"));
        assert_eq!(amount_of(&sheet), None);
    }

    // ---- The Home Screen ----------------------------------------------

    #[test]
    fn on_the_home_screen_only_the_kalshi_icon_may_be_tapped() {
        let home = home_screen();
        // The label is at x 240..330, y 960..982; the icon is above it.
        for point in [
            (285.0, 890.0),
            (285.0, 970.0),
            (230.0, 840.0),
            (350.0, 930.0),
        ] {
            assert!(allowed(&judge(&home, &tap(point.0, point.1))), "{point:?}");
        }
        // Its neighbours, the row above, the dock and the search pill.
        for point in [
            (115.0, 890.0),
            (455.0, 890.0),
            (285.0, 700.0),
            (285.0, 1100.0),
            (370.0, 1390.0),
        ] {
            let decision = judge(&home, &tap(point.0, point.1));
            assert!(
                reason(&decision).contains("only the Kalshi icon"),
                "{point:?}"
            );
        }
        assert!(allowed(&judge(&home, &Gesture::PressHome)));
    }

    #[test]
    fn nothing_but_a_plain_tap_opens_kalshi() {
        let home = home_screen();
        let on_icon = (285.0, 890.0);
        for gesture in [
            Gesture::LongPress {
                x: on_icon.0,
                y: on_icon.1,
                ms: None,
            },
            Gesture::DoubleTap {
                x: on_icon.0,
                y: on_icon.1,
            },
            swipe(on_icon, (285.0, 300.0)),
            swipe((600.0, 800.0), (100.0, 800.0)),
            Gesture::TypeText {
                text: "robinhood".into(),
            },
        ] {
            assert!(!allowed(&judge(&home, &gesture)), "{gesture:?}");
        }
    }

    #[test]
    fn a_home_page_without_kalshi_allows_only_home() {
        let home = home_screen_without_kalshi();
        let decision = judge(&home, &tap(285.0, 890.0));
        assert!(reason(&decision).contains("isn’t on this page"));
        assert!(allowed(&judge(&home, &Gesture::PressHome)));
    }

    #[test]
    fn gestures_are_scaled_to_the_frame_the_text_came_from() {
        // The same tap, given in iPhone points (368 x 800) instead of
        // frame pixels.
        let points = Screen {
            width: 368.0,
            height: 800.0,
        };
        let home = home_screen();
        let judge_in_points =
            |gesture: &Gesture| decide(&limits(), &fresh(), Ok(&home), gesture, points, None, NOW);
        assert!(allowed(&judge_in_points(&tap(142.5, 445.0))));
        assert!(!allowed(&judge_in_points(&tap(285.0, 890.0))));
    }

    // ---- Other apps -----------------------------------------------------

    #[test]
    fn outside_kalshi_only_home_is_allowed() {
        for other in [
            notes_app(),
            settings_app(),
            robinhood_app(),
            app_store(),
            frame(vec![]),
        ] {
            for gesture in [
                tap(300.0, 310.0),
                swipe((80.0, 1455.0), (660.0, 1455.0)),
                Gesture::TypeText { text: "10".into() },
                Gesture::LongPress {
                    x: 300.0,
                    y: 300.0,
                    ms: None,
                },
            ] {
                let decision = judge(&other, &gesture);
                assert!(reason(&decision).starts_with(LEFT_KALSHI), "{gesture:?}");
                // The screen may have been caught mid-change.
                assert!(matches!(decision, Decision::Block { retry: true, .. }));
            }
            assert!(allowed(&judge(&other, &Gesture::PressHome)));
        }
    }

    #[test]
    fn a_screen_that_cannot_be_read_allows_only_home() {
        let blind = |gesture: &Gesture| {
            decide(
                &limits(),
                &fresh(),
                Err("no picture"),
                gesture,
                PHONE,
                None,
                NOW,
            )
        };
        let decision = blind(&tap(10.0, 10.0));
        assert!(reason(&decision).contains("can’t read"));
        assert!(matches!(decision, Decision::Block { retry: true, .. }));
        assert!(allowed(&blind(&Gesture::PressHome)));
    }

    // ---- Inside Kalshi --------------------------------------------------

    #[test]
    fn looking_around_kalshi_is_allowed() {
        let market = market_page();
        for gesture in [
            tap(185.0, 1395.0), // Up
            tap(555.0, 1395.0), // Down
            tap(660.0, 1310.0), // COMBO
            swipe((370.0, 1000.0), (370.0, 400.0)),
            swipe((100.0, 600.0), (600.0, 600.0)), // scrub the chart
            Gesture::TypeText { text: "zec".into() },
        ] {
            assert_eq!(judge(&market, &gesture), Decision::Allow(Note::Plain));
        }
        assert!(allowed(&judge(&settled_page(), &tap(370.0, 1415.0))));
        assert!(allowed(&judge(&kalshi_dialog(), &tap(540.0, 874.0))));
    }

    #[test]
    fn the_keypad_and_the_rest_of_the_sheet_can_be_tapped() {
        let sheet = order_sheet("$0");
        for point in [
            (165.0, 702.0),  // 1
            (365.0, 1122.0), // 0
            (165.0, 1122.0), // .
            (565.0, 1122.0), // delete
            (460.0, 303.0),  // DOWN
            (370.0, 1302.0), // Read rules
        ] {
            assert_eq!(
                judge(&sheet, &tap(point.0, point.1)),
                Decision::Allow(Note::Plain),
                "{point:?}"
            );
        }
    }

    #[test]
    fn typing_is_refused_on_the_order_sheet() {
        let decision = judge(&order_sheet("$0"), &Gesture::TypeText { text: "10".into() });
        assert!(reason(&decision).contains("keypad"));
    }

    #[test]
    fn a_buy_of_exactly_the_stake_asks_for_approval() {
        let decision = judge(&order_sheet("$0.10"), &slide_to_buy());
        assert_eq!(
            decision,
            Decision::NeedsConfirm {
                buy: zec_up(),
                summary: "Up, $0.10, ZEC 15 min".into()
            }
        );
        let down = judge(
            &order_sheet_with(&[("$", 260.0), ("0.10", 310.0)], "Down"),
            &slide_to_buy(),
        );
        assert!(matches!(
            down,
            Decision::NeedsConfirm { summary, .. } if summary == "Down, $0.10, ZEC 15 min"
        ));
    }

    #[test]
    fn a_buy_of_any_other_amount_is_refused() {
        for (amount, says) in [
            ("$0", "is $0.00"),
            ("$1", "is $1.00"),
            ("$10.00", "is $10.00"),
            ("$0.11", "is $0.11"),
            ("$0.09", "is $0.09"),
            ("$0.01", "is $0.01"),
            ("$100", "is $100.00"),
            ("0.10", "can’t read the order amount"),
            ("$0.1O0", "can’t read the order amount"),
            ("$??", "can’t read the order amount"),
        ] {
            let decision = judge(&order_sheet(amount), &slide_to_buy());
            assert!(reason(&decision).contains(says), "{amount}: {decision:?}");
        }
        let empty = judge(&order_sheet_with(&[], "Up"), &slide_to_buy());
        assert!(reason(&empty).contains("can’t read the order amount"));
    }

    #[test]
    fn every_way_of_touching_the_slider_counts_as_buying() {
        let sheet = order_sheet("$1");
        for gesture in [
            slide_to_buy(),
            // Starting on the label rather than the knob.
            swipe((300.0, 1455.0), (700.0, 1455.0)),
            // A short drag, a diagonal one, a drag to the left.
            swipe((80.0, 1455.0), (200.0, 1455.0)),
            swipe((80.0, 1500.0), (660.0, 1300.0)),
            swipe((660.0, 1455.0), (80.0, 1455.0)),
            // Starting just above or below the pill.
            swipe((80.0, 1390.0), (660.0, 1390.0)),
            swipe((80.0, 1530.0), (660.0, 1530.0)),
            tap(80.0, 1455.0),
            tap(380.0, 1455.0),
            Gesture::DoubleTap { x: 80.0, y: 1455.0 },
            Gesture::LongPress {
                x: 80.0,
                y: 1455.0,
                ms: Some(2000),
            },
        ] {
            let decision = judge(&sheet, &gesture);
            assert!(
                reason(&decision).contains("must be exactly $0.10"),
                "{gesture:?}"
            );
        }
    }

    #[test]
    fn a_slider_whose_label_was_not_read_is_still_a_buy() {
        // The sheet is recognised by its other lines.
        let mut sheet = order_sheet("$1");
        sheet.lines.retain(|l| !l.text.starts_with("Slide"));
        assert!(matches!(
            classify(&sheet),
            ScreenKind::KalshiOrderSheet {
                side: None,
                amount: Some(100)
            }
        ));
        assert!(!allowed(&judge(&sheet, &slide_to_buy())));
        assert!(!allowed(&judge(&sheet, &tap(80.0, 1455.0))));
        // The keypad above the footer still works.
        assert!(allowed(&judge(&sheet, &tap(365.0, 1122.0))));
        // With the right amount it is a buy like any other, side unread.
        let mut sheet = order_sheet("$0.10");
        sheet.lines.retain(|l| !l.text.starts_with("Slide"));
        assert!(matches!(
            judge(&sheet, &slide_to_buy()),
            Decision::NeedsConfirm { summary, .. } if summary == "Side not read, $0.10, ZEC 15 min"
        ));

        // A garbled label is treated as a buy slider too.
        let mut sheet = order_sheet("$1");
        for l in &mut sheet.lines {
            if l.text.starts_with("Slide") {
                l.text = "Slide to bvy Vp".into();
            }
        }
        assert!(!allowed(&judge(&sheet, &slide_to_buy())));
    }

    #[test]
    fn a_slide_along_the_bottom_of_an_unrecognised_kalshi_screen_is_refused() {
        // Kalshi, but not the order sheet, and no slider label read.
        let decision = judge(&market_page(), &swipe((80.0, 1500.0), (660.0, 1500.0)));
        assert!(reason(&decision).contains("can’t check the amount"));
        // A "Buy" button on some other Kalshi screen.
        let mut page = settled_page();
        page.lines.push(line("Buy Up", 300.0, 1200.0, 120.0, 30.0));
        let decision = judge(&page, &tap(360.0, 1215.0));
        assert!(reason(&decision).contains("can’t check the amount"));
    }

    #[test]
    fn buying_is_only_allowed_in_the_named_market() {
        let mut sheet = order_sheet("$0.10");
        for l in &mut sheet.lines {
            l.text = l.text.replace("ZEC", "BTC");
        }
        let decision = judge(&sheet, &slide_to_buy());
        assert!(reason(&decision).contains("only allowed in the ZEC 15 min market"));
        // Spacing and case the recogniser got wrong still match.
        let mut sheet = order_sheet("$0.10");
        for l in &mut sheet.lines {
            l.text = l.text.replace("ZEC 15 min", "zec 15min");
        }
        assert!(matches!(
            judge(&sheet, &slide_to_buy()),
            Decision::NeedsConfirm { .. }
        ));
    }

    #[test]
    fn the_daily_limits_stop_a_buy() {
        let sheet = order_sheet("$0.10");
        let with = |trades, spent_cents, limits: &TradingLimits| {
            let counters = Counters {
                day: NOW.day(),
                trades,
                spent_cents,
            };
            decide(
                limits,
                &counters,
                Ok(&sheet),
                &slide_to_buy(),
                PHONE,
                None,
                NOW,
            )
        };
        let l = limits();
        assert!(matches!(with(9, 90, &l), Decision::NeedsConfirm { .. }));
        assert!(reason(&with(10, 100, &l)).contains("limit of 10 trades"));
        assert!(reason(&with(10, 0, &l)).contains("limit of 10 trades"));
        // Ten cents left of the dollar is the last buy; less is not enough.
        assert!(matches!(with(3, 90, &l), Decision::NeedsConfirm { .. }));
        assert!(reason(&with(3, 91, &l)).contains("loss limit of $1.00"));
        assert!(reason(&with(3, 100, &l)).contains("loss limit of $1.00"));
        // The limits are the user's, not constants.
        let tight = TradingLimits {
            max_trades_per_day: 2,
            max_loss_cents_per_day: 15,
            ..limits()
        };
        assert!(matches!(with(0, 0, &tight), Decision::NeedsConfirm { .. }));
        assert!(reason(&with(1, 10, &tight)).contains("loss limit of $0.15"));
        assert!(reason(&with(2, 0, &tight)).contains("limit of 2 trades"));
        assert!(reason(&with(u32::MAX, Cents::MAX, &l)).contains("trades"));
    }

    #[test]
    fn an_approval_is_for_one_buy_and_one_minute() {
        let sheet = order_sheet("$0.10");
        let with = |approval: &Approval, now: i64| {
            decide(
                &limits(),
                &fresh(),
                Ok(&sheet),
                &slide_to_buy(),
                PHONE,
                Some(approval),
                Now {
                    unix: now,
                    utc_offset: 0,
                },
            )
        };
        let t = NOW.unix;
        assert_eq!(
            with(&approve(&zec_up(), t), t + 1),
            Decision::Allow(Note::Buy(zec_up()))
        );
        assert!(allowed(&with(&approve(&zec_up(), t), t + 60)));
        // Too old, or from the future.
        assert!(matches!(
            with(&approve(&zec_up(), t), t + 61),
            Decision::NeedsConfirm { .. }
        ));
        assert!(matches!(
            with(&approve(&zec_up(), t + 5), t),
            Decision::NeedsConfirm { .. }
        ));
        // For the other side, another market or another amount.
        for other in [
            BuyInfo {
                side: Some(Side::Down),
                ..zec_up()
            },
            BuyInfo {
                market: "BTC 15 min".into(),
                ..zec_up()
            },
            BuyInfo {
                stake_cents: 100,
                ..zec_up()
            },
        ] {
            assert!(matches!(
                with(&approve(&other, t), t + 1),
                Decision::NeedsConfirm { .. }
            ));
        }
        // An approval never excuses a wrong amount.
        let wrong = decide(
            &limits(),
            &fresh(),
            Ok(&order_sheet("$1")),
            &slide_to_buy(),
            PHONE,
            Some(&approve(&zec_up(), t)),
            NOW,
        );
        assert!(reason(&wrong).contains("is $1.00"));
    }

    #[test]
    fn without_confirm_each_buy_a_good_buy_goes_straight_through() {
        let l = TradingLimits {
            confirm_each_buy: false,
            ..limits()
        };
        let decision = decide(
            &l,
            &fresh(),
            Ok(&order_sheet("$0.10")),
            &slide_to_buy(),
            PHONE,
            None,
            NOW,
        );
        assert_eq!(decision, Decision::Allow(Note::Buy(zec_up())));
    }

    #[test]
    fn selling_is_allowed_and_noted() {
        let position = position_page();
        assert_eq!(
            judge(&position, &swipe((80.0, 1455.0), (660.0, 1455.0))),
            Decision::Allow(Note::Sell)
        );
        assert_eq!(
            judge(&portfolio_page(), &tap(630.0, 573.0)),
            Decision::Allow(Note::Sell)
        );
        let mut page = settled_page();
        page.lines
            .push(line("Cash out", 300.0, 1200.0, 140.0, 30.0));
        assert_eq!(
            judge(&page, &tap(370.0, 1215.0)),
            Decision::Allow(Note::Sell)
        );
    }

    #[test]
    fn moving_money_is_refused() {
        let portfolio = portfolio_page();
        for point in [(175.0, 344.0), (545.0, 344.0)] {
            let decision = judge(&portfolio, &tap(point.0, point.1));
            assert!(reason(&decision).contains("Moving money"), "{point:?}");
        }
        assert!(allowed(&judge(&portfolio, &tap(120.0, 473.0))));
    }

    // ---- The whole gate -------------------------------------------------

    /// A phone that shows a script of screens and remembers what was
    /// done to it.
    struct Script {
        screens: Vec<Result<OcrFrame, String>>,
        reads: usize,
        ledger: Result<Ledger, String>,
        save_fails: bool,
        answers: Vec<Confirmation>,
        asked: Vec<String>,
        performed: Vec<Gesture>,
        perform_fails: bool,
        waited_ms: u64,
        clock: i64,
    }

    impl Script {
        fn showing(screens: Vec<OcrFrame>) -> Script {
            Script {
                screens: screens.into_iter().map(Ok).collect(),
                reads: 0,
                ledger: Ok(Ledger::default()),
                save_fails: false,
                answers: Vec::new(),
                asked: Vec::new(),
                performed: Vec::new(),
                perform_fails: false,
                waited_ms: 0,
                clock: NOW.unix,
            }
        }

        fn answering(mut self, answers: &[Confirmation]) -> Script {
            self.answers = answers.to_vec();
            self
        }

        fn events(&self) -> &[LedgerEvent] {
            &self.ledger.as_ref().unwrap().events
        }

        fn kinds(&self) -> Vec<EventKind> {
            self.events().iter().map(|e| e.kind).collect()
        }
    }

    impl Rails for Script {
        fn now(&self) -> Now {
            Now {
                unix: self.clock,
                utc_offset: 0,
            }
        }

        fn read_screen(&mut self) -> Result<OcrFrame, String> {
            // The last screen stays up.
            let at = self.reads.min(self.screens.len() - 1);
            self.reads += 1;
            self.screens[at].clone()
        }

        fn load_ledger(&mut self) -> Result<Ledger, String> {
            self.ledger.clone()
        }

        fn save_ledger(&mut self, ledger: &Ledger) -> Result<(), String> {
            if self.save_fails {
                return Err("Couldn’t save the trading ledger: disk full".into());
            }
            self.ledger = Ok(ledger.clone());
            Ok(())
        }

        fn wait_ms(&mut self, ms: u64) {
            self.waited_ms += ms;
        }

        fn confirm(&mut self, _buy: &BuyInfo, summary: &str) -> Confirmation {
            self.asked.push(summary.to_string());
            // The user takes ten seconds.
            self.clock += 10;
            if self.answers.is_empty() {
                Confirmation::TimedOut
            } else {
                self.answers.remove(0)
            }
        }

        fn perform(&mut self, gesture: &Gesture, _screen: Screen) -> Result<(), String> {
            if self.perform_fails {
                return Err("the iPhone is not connected".into());
            }
            self.performed.push(gesture.clone());
            Ok(())
        }
    }

    fn pass(
        script: &mut Script,
        limits: &TradingLimits,
        gesture: &Gesture,
    ) -> Result<(), GateError> {
        gate(script, limits, gesture, PHONE)
    }

    fn blocked(result: Result<(), GateError>) -> String {
        match result {
            Err(GateError::Blocked(why)) => why,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn with_trading_mode_off_nothing_is_checked() {
        let mut phone = Script::showing(vec![robinhood_app()]);
        let off = TradingLimits::default();
        assert_eq!(pass(&mut phone, &off, &tap(370.0, 1455.0)), Ok(()));
        assert_eq!(phone.reads, 0);
        assert_eq!(phone.performed.len(), 1);
        assert!(phone.events().is_empty());
        // A failure to send is passed on as it was.
        phone.perform_fails = true;
        assert_eq!(
            pass(&mut phone, &off, &tap(1.0, 1.0)),
            Err(GateError::Failed("the iPhone is not connected".into()))
        );
    }

    #[test]
    fn an_approved_buy_is_written_down_then_made() {
        let mut phone =
            Script::showing(vec![order_sheet("$0.10")]).answering(&[Confirmation::Approved]);
        assert_eq!(pass(&mut phone, &limits(), &slide_to_buy()), Ok(()));
        assert_eq!(phone.asked, ["Up, $0.10, ZEC 15 min"]);
        assert_eq!(phone.performed, [slide_to_buy()]);
        // The screen was read again after the approval.
        assert_eq!(phone.reads, 2);
        let events = phone.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::Buy);
        assert_eq!(events[0].side, Some(Side::Up));
        assert_eq!(events[0].stake_cents, 10);
        assert_eq!(events[0].market.as_deref(), Some("ZEC 15 min"));
        assert_eq!(events[0].day, "2026-11-19");
    }

    #[test]
    fn a_declined_unanswered_or_stopped_buy_is_not_made() {
        for (answer, says) in [
            (Confirmation::Declined, "wasn’t approved"),
            (Confirmation::TimedOut, "within a minute"),
            (Confirmation::Stopped, "cancelled"),
        ] {
            let mut phone = Script::showing(vec![order_sheet("$0.10")]).answering(&[answer]);
            let why = blocked(pass(&mut phone, &limits(), &slide_to_buy()));
            assert!(why.contains(says), "{why}");
            assert!(phone.performed.is_empty());
            assert_eq!(phone.kinds(), [EventKind::Blocked]);
        }
    }

    #[test]
    fn a_screen_that_changed_during_approval_is_judged_again() {
        // Approved as $0.10, but by then the sheet reads $10.
        let mut phone = Script::showing(vec![order_sheet("$0.10"), order_sheet("$10")])
            .answering(&[Confirmation::Approved]);
        let why = blocked(pass(&mut phone, &limits(), &slide_to_buy()));
        assert!(why.contains("is $10.00"), "{why}");
        assert!(phone.performed.is_empty());

        // Approved Up, but the sheet is now for Down: asked again, for Down.
        let mut phone = Script::showing(vec![
            order_sheet("$0.10"),
            order_sheet_with(&[("$0.10", 280.0)], "Down"),
        ])
        .answering(&[Confirmation::Approved, Confirmation::Declined]);
        let why = blocked(pass(&mut phone, &limits(), &slide_to_buy()));
        assert!(why.contains("Down, $0.10"), "{why}");
        assert_eq!(phone.asked.len(), 2);
        assert!(phone.performed.is_empty());

        // A sheet that keeps changing is given up on, not asked forever.
        let mut phone = Script::showing(vec![
            order_sheet("$0.10"),
            order_sheet_with(&[("$0.10", 280.0)], "Down"),
            order_sheet("$0.10"),
            order_sheet_with(&[("$0.10", 280.0)], "Down"),
        ])
        .answering(&[Confirmation::Approved; 5]);
        blocked(pass(&mut phone, &limits(), &slide_to_buy()));
        assert_eq!(phone.asked.len(), 2);
        assert!(phone.performed.is_empty());
    }

    #[test]
    fn ten_buys_a_day_and_not_one_more() {
        let l = TradingLimits {
            confirm_each_buy: false,
            ..limits()
        };
        let mut phone = Script::showing(vec![order_sheet("$0.10")]);
        for _ in 0..10 {
            assert_eq!(pass(&mut phone, &l, &slide_to_buy()), Ok(()));
        }
        let why = blocked(pass(&mut phone, &l, &slide_to_buy()));
        assert!(why.contains("limit of 10 trades"), "{why}");
        assert_eq!(phone.performed.len(), 10);
        let counters = phone.ledger.as_ref().unwrap().counters("2026-11-19");
        assert_eq!((counters.trades, counters.spent_cents), (10, 100));
        // Looking around and selling still work.
        assert_eq!(pass(&mut phone, &l, &tap(365.0, 1122.0)), Ok(()));
        // Tomorrow the count starts again.
        phone.clock += 86_400;
        assert_eq!(pass(&mut phone, &l, &slide_to_buy()), Ok(()));
        assert_eq!(phone.performed.len(), 12);
    }

    #[test]
    fn the_loss_limit_counts_money_not_trades() {
        let l = TradingLimits {
            confirm_each_buy: false,
            max_loss_cents_per_day: 30,
            ..limits()
        };
        let mut phone = Script::showing(vec![order_sheet("$0.10")]);
        for _ in 0..3 {
            assert_eq!(pass(&mut phone, &l, &slide_to_buy()), Ok(()));
        }
        let why = blocked(pass(&mut phone, &l, &slide_to_buy()));
        assert!(why.contains("loss limit of $0.30"), "{why}");
        assert_eq!(phone.performed.len(), 3);
    }

    #[test]
    fn a_buy_that_cannot_be_written_down_is_not_made() {
        let l = TradingLimits {
            confirm_each_buy: false,
            ..limits()
        };
        let mut phone = Script::showing(vec![order_sheet("$0.10")]);
        phone.save_fails = true;
        let why = blocked(pass(&mut phone, &l, &slide_to_buy()));
        assert!(why.contains("Couldn’t save the trading ledger"), "{why}");
        assert!(phone.performed.is_empty());

        let mut phone = Script::showing(vec![order_sheet("$0.10")]);
        phone.ledger = Err("The trading ledger is damaged (line 1)".into());
        let why = blocked(pass(&mut phone, &l, &slide_to_buy()));
        assert!(
            why.contains("damaged") && why.contains("nothing can be bought"),
            "{why}"
        );
        assert!(phone.performed.is_empty());
        // Everything that isn't a buy carries on.
        assert_eq!(pass(&mut phone, &l, &tap(365.0, 1122.0)), Ok(()));
    }

    #[test]
    fn a_buy_counts_even_when_sending_it_failed() {
        let l = TradingLimits {
            confirm_each_buy: false,
            ..limits()
        };
        let mut phone = Script::showing(vec![order_sheet("$0.10")]);
        phone.perform_fails = true;
        assert!(matches!(
            pass(&mut phone, &l, &slide_to_buy()),
            Err(GateError::Failed(_))
        ));
        assert_eq!(phone.kinds(), [EventKind::Buy]);
    }

    #[test]
    fn a_screen_caught_mid_change_gets_a_second_look() {
        // Blank while Kalshi opens, then the market.
        let mut phone = Script::showing(vec![frame(vec![]), market_page()]);
        assert_eq!(pass(&mut phone, &limits(), &tap(185.0, 1395.0)), Ok(()));
        assert_eq!(phone.reads, 2);
        assert_eq!(phone.waited_ms, RETRY_WAIT_MS);
        assert!(phone.events().is_empty());

        // An unreadable screen that clears up.
        let mut phone = Script::showing(vec![market_page()]);
        phone.screens.insert(0, Err("no picture".into()));
        assert_eq!(pass(&mut phone, &limits(), &tap(185.0, 1395.0)), Ok(()));
    }

    #[test]
    fn leaving_kalshi_is_refused_and_written_down_once() {
        let mut phone = Script::showing(vec![robinhood_app()]);
        let why = blocked(pass(&mut phone, &limits(), &tap(370.0, 1455.0)));
        assert!(why.starts_with(LEFT_KALSHI), "{why}");
        assert_eq!(phone.reads, 1 + RETRIES as usize);
        assert!(phone.performed.is_empty());
        blocked(pass(&mut phone, &limits(), &tap(370.0, 1455.0)));
        assert_eq!(phone.kinds(), [EventKind::Blocked]);
        // Home is the way back, and needs no look at the screen.
        let reads = phone.reads;
        assert_eq!(pass(&mut phone, &limits(), &Gesture::PressHome), Ok(()));
        assert_eq!(phone.performed, [Gesture::PressHome]);
        assert!(phone.reads <= reads + 1);
    }

    #[test]
    fn a_sale_is_made_then_noted_and_costs_nothing() {
        let mut phone = Script::showing(vec![position_page()]);
        let slide = swipe((80.0, 1455.0), (660.0, 1455.0));
        assert_eq!(pass(&mut phone, &limits(), &slide), Ok(()));
        assert_eq!(phone.performed, [slide]);
        assert_eq!(phone.kinds(), [EventKind::Sell]);
        assert_eq!(phone.events()[0].market.as_deref(), Some("ZEC 15 min"));
        let counters = phone.ledger.as_ref().unwrap().counters("2026-11-19");
        assert_eq!((counters.trades, counters.spent_cents), (0, 0));
    }

    #[test]
    fn a_refusal_reads_as_coming_from_the_rails() {
        assert_eq!(
            GateError::Blocked("Left Kalshi".into()).message(),
            "Trading rails: Left Kalshi."
        );
        assert_eq!(GateError::Failed("No link.".into()).message(), "No link.");
    }
}
