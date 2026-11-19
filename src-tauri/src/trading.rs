//! The trading rails' place in the app: every gesture goes through
//! [`perform`], which asks `edart_core::guard` whether it may happen.
//!
//! The rules and the order of events (read the screen, judge, ask the
//! user, write the ledger, send) are `edart_core::guard::gate`, where
//! they are tested. This file gives it the real screen, clock, ledger
//! file, confirmation sheet and phone.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use edart_core::gesture::{Gesture, Screen};
use edart_core::guard::{self, BuyInfo, Confirmation, GateError, OcrFrame, Rails};
use edart_core::ledger::{self, Ledger, LedgerEvent, Now};
use edart_core::settings;
use edart_core::trading::{PendingBuy, TradingChange, TradingLimits, TradingView};
use tauri::{AppHandle, Emitter, Manager};

use crate::{agent, clock, ocr};

/// The UI listens for this and reads [`trading_get`] again.
pub const CHANGED_EVENT: &str = "trading-changed";
/// How long the user has to answer a confirmation.
const CONFIRM_FOR: Duration = Duration::from_secs(guard::APPROVAL_SECS as u64);
/// Most ledger lines sent to the Trading screen.
const LEDGER_LINES: usize = 100;

/// One guarded gesture at a time, from the look at the screen to the
/// last report sent, so two callers cannot both take the last trade of
/// the day.
static GATE: Mutex<()> = Mutex::new(());

struct Asking {
    id: u64,
    buy: BuyInfo,
    summary: String,
    since: Instant,
}

#[derive(Default)]
struct Confirm {
    last_id: u64,
    asking: Option<Asking>,
    /// The user's answer to the question with this id.
    answer: Option<(u64, bool)>,
}

static CONFIRM: Mutex<Option<Confirm>> = Mutex::new(None);

fn with_confirm<T>(f: impl FnOnce(&mut Confirm) -> T) -> T {
    let mut guard = CONFIRM.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Confirm::default))
}

/// Who is asking for a gesture.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The agent loop; pressing Stop cancels a confirmation it waits on.
    Agent,
    /// The user's own clicks on the mirror, or an MCP client.
    Other,
}

fn ledger_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join("trading-ledger.json"))
        .map_err(|e| e.to_string())
}

/// The trading limits as saved, within their bounds.
pub fn limits(app: &AppHandle) -> Result<TradingLimits, String> {
    Ok(settings::load(&crate::settings_path(app)?)?
        .trading
        .sanitized())
}

struct AppRails<'a> {
    app: &'a AppHandle,
    source: Source,
}

impl Rails for AppRails<'_> {
    fn now(&self) -> Now {
        clock::now()
    }

    fn read_screen(&mut self) -> Result<OcrFrame, String> {
        ocr::read()
    }

    fn load_ledger(&mut self) -> Result<Ledger, String> {
        ledger::load(&ledger_path(self.app)?)
    }

    fn save_ledger(&mut self, ledger: &Ledger) -> Result<(), String> {
        let saved = ledger::save(&ledger_path(self.app)?, ledger);
        let _ = self.app.emit(CHANGED_EVENT, ());
        saved
    }

    fn wait_ms(&mut self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    fn confirm(&mut self, buy: &BuyInfo, summary: &str) -> Confirmation {
        let id = with_confirm(|c| {
            c.last_id += 1;
            c.answer = None;
            c.asking = Some(Asking {
                id: c.last_id,
                buy: buy.clone(),
                summary: summary.to_string(),
                since: Instant::now(),
            });
            c.last_id
        });
        let _ = self.app.emit(CHANGED_EVENT, ());
        let started = Instant::now();
        let answer = loop {
            if let Some(approved) =
                with_confirm(|c| c.answer.filter(|(for_id, _)| *for_id == id).map(|a| a.1))
            {
                break if approved {
                    Confirmation::Approved
                } else {
                    Confirmation::Declined
                };
            }
            if self.source == Source::Agent && agent::stop_requested() {
                break Confirmation::Stopped;
            }
            if started.elapsed() >= CONFIRM_FOR {
                break Confirmation::TimedOut;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        with_confirm(|c| {
            c.asking = None;
            c.answer = None;
        });
        let _ = self.app.emit(CHANGED_EVENT, ());
        answer
    }

    fn perform(&mut self, gesture: &Gesture, screen: Screen) -> Result<(), String> {
        crate::perform_unchecked(self.app, gesture, screen)
    }
}

/// Perform a gesture on the iPhone, if the trading rails allow it. With
/// trading mode off it is sent as it always was.
pub fn perform(
    app: &AppHandle,
    gesture: &Gesture,
    screen: Screen,
    source: Source,
) -> Result<(), GateError> {
    // If the settings cannot be read there is no knowing whether trading
    // mode is on, so nothing is sent.
    let limits = limits(app).map_err(|why| {
        GateError::Blocked(format!(
            "{why} Until then Edart can’t tell whether trading mode is on"
        ))
    })?;
    if !limits.enabled {
        return crate::perform_unchecked(app, gesture, screen).map_err(GateError::Failed);
    }
    let _one_at_a_time = GATE.lock().unwrap_or_else(|e| e.into_inner());
    guard::gate(&mut AppRails { app, source }, &limits, gesture, screen)
}

fn view(app: &AppHandle, limits: &TradingLimits) -> TradingView {
    let ledger = ledger_path(app).and_then(|path| ledger::load(&path));
    let pending = with_confirm(|c| {
        c.asking.as_ref().map(|asking| PendingBuy {
            summary: asking.summary.clone(),
            buy: asking.buy.clone(),
            seconds_left: CONFIRM_FOR.saturating_sub(asking.since.elapsed()).as_secs() as u32,
        })
    });
    limits.view(
        ledger.as_ref().map_err(String::as_str),
        clock::now(),
        pending,
    )
}

#[tauri::command]
pub fn trading_get(app: AppHandle) -> Result<TradingView, String> {
    Ok(view(&app, &limits(&app)?))
}

/// Change the trading rules. They are read again before every gesture,
/// so a change holds at once, even for a run that is under way.
#[tauri::command]
pub fn trading_set(app: AppHandle, change: TradingChange) -> Result<TradingView, String> {
    let path = crate::settings_path(&app)?;
    let mut current = settings::load(&path)?;
    let mut trading = current.trading.clone().sanitized();
    trading.apply(change)?;
    current.trading = trading;
    settings::save(&path, &current).map_err(|e| format!("Couldn’t save the settings: {e}"))?;
    let _ = app.emit(CHANGED_EVENT, ());
    Ok(view(&app, &current.trading))
}

/// The newest lines of the ledger, newest first.
#[tauri::command]
pub fn trading_ledger(app: AppHandle) -> Result<Vec<LedgerEvent>, String> {
    Ok(ledger::load(&ledger_path(&app)?)?.recent(LEDGER_LINES))
}

/// The user's answer to the buy being asked about.
#[tauri::command]
pub fn trading_confirm(app: AppHandle, approve: bool) {
    with_confirm(|c| {
        if let Some(asking) = &c.asking {
            c.answer = Some((asking.id, approve));
        }
    });
    let _ = app.emit(CHANGED_EVENT, ());
}
