//! The trading ledger: every buy, sell and refused action, by local day.
//!
//! The day's counters (trades made, money spent) are worked out from it,
//! so there is one record and nothing to drift. It is a small JSON file
//! in the app's config folder, readable by the user only.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::settings::write_private;
use crate::trading::Cents;

/// The time, with enough to know the local date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Now {
    /// Seconds since 1970, UTC.
    pub unix: i64,
    /// Local time minus UTC, in seconds.
    pub utc_offset: i32,
}

impl Now {
    /// The local date, "2026-11-19".
    pub fn day(&self) -> String {
        let days = (self.unix + i64::from(self.utc_offset)).div_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        format!("{year:04}-{month:02}-{day:02}")
    }
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's
/// algorithm, valid for any date this app will see.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Which side of an up/down market.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Up,
    Down,
}

impl Side {
    pub fn label(self) -> &'static str {
        match self {
            Side::Up => "Up",
            Side::Down => "Down",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    /// A buy the rails let through. Counted against the day's limits.
    Buy,
    /// A sell or cash-out the rails let through. Not counted.
    Sell,
    /// An action the rails refused.
    Blocked,
}

/// One line of the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEvent {
    /// Seconds since 1970, UTC.
    pub time: i64,
    /// The local date it happened on.
    pub day: String,
    pub kind: EventKind,
    /// The side, when the screen said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    /// What a buy cost; zero for the rest.
    #[serde(default)]
    pub stake_cents: Cents,
    /// The market's name, when the screen said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
    /// Why it was refused, or a note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A day's totals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counters {
    pub day: String,
    /// Buys made today.
    pub trades: u32,
    /// The sum of their stakes.
    pub spent_cents: Cents,
}

/// Buys and sells older than this many days are dropped.
const KEEP_DAYS: i64 = 30;
/// Most refused actions kept.
const KEEP_BLOCKED: usize = 200;
/// The same refusal again within this long is not written twice.
const SAME_BLOCK_SECS: i64 = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub events: Vec<LedgerEvent>,
}

impl Ledger {
    /// What has been bought on `day`.
    pub fn counters(&self, day: &str) -> Counters {
        let buys = self
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Buy && e.day == day);
        let (trades, spent) = buys.fold((0u32, 0u32), |(n, sum), e| {
            (n + 1, sum.saturating_add(e.stake_cents))
        });
        Counters {
            day: day.to_string(),
            trades,
            spent_cents: spent,
        }
    }

    /// Add a line. Returns false when it repeats the refusal just before
    /// it and was left out.
    pub fn record(&mut self, now: Now, mut event: LedgerEvent) -> bool {
        event.time = now.unix;
        event.day = now.day();
        if event.kind == EventKind::Blocked {
            if let Some(last) = self.events.last() {
                if last.kind == EventKind::Blocked
                    && last.reason == event.reason
                    && (now.unix - last.time).abs() < SAME_BLOCK_SECS
                {
                    return false;
                }
            }
        }
        self.events.push(event);
        self.prune(now);
        true
    }

    /// Drop what is too old to matter. Today's buys are never dropped:
    /// the limits are counted from them.
    fn prune(&mut self, now: Now) {
        let oldest = now.unix - KEEP_DAYS * 86_400;
        let today = now.day();
        self.events.retain(|e| e.day == today || e.time >= oldest);
        let blocked = self
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Blocked)
            .count();
        let mut extra = blocked.saturating_sub(KEEP_BLOCKED);
        self.events.retain(|e| {
            if extra > 0 && e.kind == EventKind::Blocked {
                extra -= 1;
                false
            } else {
                true
            }
        });
    }

    /// The newest `limit` lines, newest first.
    pub fn recent(&self, limit: usize) -> Vec<LedgerEvent> {
        self.events.iter().rev().take(limit).cloned().collect()
    }
}

/// Read the ledger. A missing file is an empty ledger; a damaged one is
/// an error, so the day's count is never silently reset to zero.
pub fn load(path: &Path) -> Result<Ledger, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|e| format!("The trading ledger is damaged ({e})"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
        Err(e) => Err(format!("Couldn’t read the trading ledger: {e}")),
    }
}

/// Write the ledger: private to the user, and whole or not at all.
pub fn save(path: &Path, ledger: &Ledger) -> Result<(), String> {
    let text = serde_json::to_string_pretty(ledger).map_err(|e| e.to_string())?;
    write_private(path, &text).map_err(|e| format!("Couldn’t save the trading ledger: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-19 20:00:00 UTC.
    const EVENING_UTC: i64 = 1_795_118_400;

    fn at(unix: i64, utc_offset: i32) -> Now {
        Now { unix, utc_offset }
    }

    fn buy(stake: Cents) -> LedgerEvent {
        LedgerEvent {
            time: 0,
            day: String::new(),
            kind: EventKind::Buy,
            side: Some(Side::Up),
            stake_cents: stake,
            market: Some("ZEC 15 min".into()),
            reason: None,
        }
    }

    fn blocked(reason: &str) -> LedgerEvent {
        LedgerEvent {
            kind: EventKind::Blocked,
            side: None,
            stake_cents: 0,
            market: None,
            reason: Some(reason.into()),
            ..buy(0)
        }
    }

    #[test]
    fn the_day_is_the_local_date() {
        assert_eq!(at(0, 0).day(), "1970-01-01");
        assert_eq!(at(-1, 0).day(), "1969-12-31");
        assert_eq!(at(EVENING_UTC, 0).day(), "2026-11-19");
        // 8 pm UTC is already tomorrow in Tokyo and still today in LA.
        assert_eq!(at(EVENING_UTC, 9 * 3600).day(), "2026-11-20");
        assert_eq!(at(EVENING_UTC, -8 * 3600).day(), "2026-11-19");
        // Leap day and a year end.
        assert_eq!(at(1_709_208_000, 0).day(), "2024-02-29");
        assert_eq!(at(1_735_689_599, 0).day(), "2024-12-31");
        assert_eq!(at(1_735_689_600, 0).day(), "2025-01-01");
    }

    #[test]
    fn counters_add_up_todays_buys_only() {
        let mut ledger = Ledger::default();
        let now = at(EVENING_UTC, 0);
        assert_eq!(ledger.counters(&now.day()).trades, 0);
        ledger.record(now, buy(10));
        ledger.record(now, buy(10));
        ledger.record(now, blocked("no"));
        ledger.record(
            now,
            LedgerEvent {
                kind: EventKind::Sell,
                ..buy(0)
            },
        );
        let counters = ledger.counters(&now.day());
        assert_eq!((counters.trades, counters.spent_cents), (2, 20));
        assert_eq!(counters.day, "2026-11-19");
    }

    #[test]
    fn the_count_starts_again_at_local_midnight() {
        let mut ledger = Ledger::default();
        // 23:59:50 local, then twenty seconds later.
        let offset = -5 * 3600;
        let late = at(EVENING_UTC + 9 * 3600 - 10, offset);
        assert_eq!(late.day(), "2026-11-19");
        ledger.record(late, buy(10));
        let after = at(late.unix + 20, offset);
        assert_eq!(after.day(), "2026-11-20");
        assert_eq!(ledger.counters(&late.day()).trades, 1);
        assert_eq!(ledger.counters(&after.day()).trades, 0);
        ledger.record(after, buy(10));
        assert_eq!(ledger.counters(&after.day()).spent_cents, 10);
        assert_eq!(ledger.counters(&late.day()).spent_cents, 10);
    }

    #[test]
    fn a_repeated_refusal_is_written_once() {
        let mut ledger = Ledger::default();
        let now = at(EVENING_UTC, 0);
        assert!(ledger.record(now, blocked("Left Kalshi")));
        assert!(!ledger.record(at(now.unix + 3, 0), blocked("Left Kalshi")));
        assert!(ledger.record(at(now.unix + 4, 0), blocked("Another reason")));
        assert!(ledger.record(at(now.unix + 60, 0), blocked("Another reason")));
        assert_eq!(ledger.events.len(), 3);
        // Buys are never folded together.
        assert!(ledger.record(now, buy(10)));
        assert!(ledger.record(now, buy(10)));
    }

    #[test]
    fn old_lines_are_dropped_but_never_todays_buys() {
        let mut ledger = Ledger::default();
        let long_ago = at(EVENING_UTC - 40 * 86_400, 0);
        ledger.record(long_ago, buy(10));
        let now = at(EVENING_UTC, 0);
        for i in 0..(KEEP_BLOCKED as i64 + 50) {
            ledger.record(at(now.unix + i * 20, 0), blocked("no"));
        }
        ledger.record(at(now.unix + 60, 0), buy(10));
        assert!(ledger.events.iter().all(|e| e.day != long_ago.day()));
        let blocked_kept = ledger
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Blocked)
            .count();
        assert_eq!(blocked_kept, KEEP_BLOCKED);
        assert_eq!(ledger.counters("2026-11-19").trades, 1);
        assert_eq!(ledger.recent(1)[0].kind, EventKind::Buy);
    }

    #[test]
    fn the_ledger_round_trips_through_a_private_file() {
        let dir = std::env::temp_dir().join(format!("edart-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("trading-ledger.json");
        assert_eq!(load(&path).unwrap(), Ledger::default());
        let mut ledger = Ledger::default();
        ledger.record(at(EVENING_UTC, 0), buy(10));
        save(&path, &ledger).unwrap();
        assert_eq!(load(&path).unwrap(), ledger);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(&path, "{broken").unwrap();
        assert!(load(&path).unwrap_err().contains("damaged"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
