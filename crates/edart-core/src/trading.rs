//! Trading mode: the limits the user sets, and the playbook the agent is
//! given.
//!
//! The limits are enforced by `crate::guard` before any gesture reaches
//! the iPhone; the playbook only tells the model what the limits are so
//! it does not waste steps running into them. Nothing here is a promise
//! about how well the model trades.

use serde::{Deserialize, Serialize};

use crate::guard::BuyInfo;
use crate::ledger::{Counters, Ledger, Now};

/// Money, in cents.
pub type Cents = u32;

/// What a buy costs unless the user says otherwise.
pub const STAKE_CENTS: Cents = 10;
/// The amount per buy can be set from one cent to five dollars.
pub const STAKE_RANGE: (Cents, Cents) = (1, 500);
pub const MAX_TRADES_RANGE: (u32, u32) = (1, 100);
/// The most spent in a day can be set from one cent to a hundred dollars.
pub const MAX_SPENT_RANGE: (Cents, Cents) = (1, 10_000);
pub const MAX_STEPS_RANGE: (u32, u32) = (10, 300);
const MAX_NAME_CHARS: usize = 40;
/// The playbook and the request share the API's goal limit.
pub const MAX_PLAYBOOK_CHARS: usize = 3000;
/// In trading mode the agent may wait this long in one step, to watch a
/// market move.
pub const TRADING_WAIT_MS: u64 = 30_000;

/// What the agent is told when trading mode is on, unless the user wrote
/// their own.
pub const DEFAULT_PLAYBOOK: &str = "\
You are trading on this iPhone in the Kalshi app only. Edart enforces hard limits and stops you if you break one, so follow them exactly:
- Use only the Kalshi app. From the Home Screen, tap the Kalshi icon. Never open another app, a link, a notification or Settings. If you end up outside Kalshi, press Home and tap Kalshi again.
- Trade only the \"ZEC 15 min\" up/down market.
- Every buy is exactly $0.10. On the order sheet tap the keypad keys 0 . 1 0 (do not type text), check that the amount reads $0.10, and only then slide \"Slide to buy\". If it shows anything else, fix it with the delete key first.
- At most 10 buys a day and at most $1.00 spent a day. Never deposit, withdraw or change account settings.
How to trade:
1. Open the ZEC 15 min market and read the target price, the current price, the time left and the Up and Down prices.
2. Consider buying a side only when its price is under 30% and the chart and recent momentum suggest it is likely to rise. If that is not clearly the case, wait and look again; not trading is a fine outcome.
3. After a buy, watch the position: wait up to 30 seconds at a time and check again.
4. When the position is up 50% to 100%, sell it to cash out. If it falls, let it expire; do not buy more to make up for a loss.
5. When the market settles, go to the next open ZEC 15 min market.
If you are unsure what a screen is or what a control does, wait or go back; do not guess. Finish with a short summary of what you bought and sold.";

/// The trading settings, as stored in the settings file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TradingLimits {
    /// Off unless the user turns it on.
    pub enabled: bool,
    /// A buy must be for exactly this much.
    pub stake_cents: Cents,
    pub max_trades_per_day: u32,
    /// Most that may be spent on buys in a day. A buy cannot lose more
    /// than its stake, so this bounds the day's loss.
    pub max_loss_cents_per_day: Cents,
    /// Ask the user before every buy. Off unless the user wants it: the
    /// limits hold either way.
    pub confirm_each_buy: bool,
    /// Most agent steps in one run while trading mode is on.
    pub max_steps: u32,
    /// The one app that may be used.
    pub app: String,
    /// A buy is allowed only in a market whose name contains this.
    /// Empty allows any market.
    pub market: String,
    /// The user's playbook; `None` is [`DEFAULT_PLAYBOOK`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playbook: Option<String>,
}

impl Default for TradingLimits {
    fn default() -> Self {
        TradingLimits {
            enabled: false,
            stake_cents: STAKE_CENTS,
            max_trades_per_day: 10,
            max_loss_cents_per_day: 100,
            confirm_each_buy: false,
            max_steps: 120,
            app: "Kalshi".into(),
            market: "ZEC 15 min".into(),
            playbook: None,
        }
    }
}

/// A change from the Trading screen. A field left out is left alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingChange {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub stake_cents: Option<Cents>,
    #[serde(default)]
    pub max_trades_per_day: Option<u32>,
    #[serde(default)]
    pub max_loss_cents_per_day: Option<Cents>,
    #[serde(default)]
    pub confirm_each_buy: Option<bool>,
    #[serde(default)]
    pub max_steps: Option<u32>,
    #[serde(default)]
    pub app: Option<String>,
    /// Empty allows any market.
    #[serde(default)]
    pub market: Option<String>,
    /// Empty goes back to the built-in playbook.
    #[serde(default)]
    pub playbook: Option<String>,
}

impl TradingLimits {
    /// The limits with every value pulled inside its bounds, so a
    /// hand-edited settings file cannot go past what the screen allows.
    pub fn sanitized(mut self) -> Self {
        self.stake_cents = self.stake_cents.clamp(STAKE_RANGE.0, STAKE_RANGE.1);
        self.max_trades_per_day = self
            .max_trades_per_day
            .clamp(MAX_TRADES_RANGE.0, MAX_TRADES_RANGE.1);
        self.max_loss_cents_per_day = self
            .max_loss_cents_per_day
            .clamp(MAX_SPENT_RANGE.0, MAX_SPENT_RANGE.1);
        self.max_steps = self.max_steps.clamp(MAX_STEPS_RANGE.0, MAX_STEPS_RANGE.1);
        self.app = clean_name(&self.app)
            .ok()
            .filter(|app| !app.is_empty())
            .unwrap_or_else(|| TradingLimits::default().app);
        self.market = clean_name(&self.market).unwrap_or_default();
        self
    }

    pub fn playbook(&self) -> &str {
        self.playbook.as_deref().unwrap_or(DEFAULT_PLAYBOOK)
    }

    /// Apply a change, or say why it is not acceptable. Nothing changes
    /// on an error.
    pub fn apply(&mut self, change: TradingChange) -> Result<(), String> {
        let mut next = self.clone();
        if let Some(enabled) = change.enabled {
            next.enabled = enabled;
        }
        if let Some(stake) = change.stake_cents {
            if !(STAKE_RANGE.0..=STAKE_RANGE.1).contains(&stake) {
                return Err(format!(
                    "The amount per buy must be between {} and {}.",
                    dollars(STAKE_RANGE.0),
                    dollars(STAKE_RANGE.1)
                ));
            }
            next.stake_cents = stake;
        }
        if let Some(trades) = change.max_trades_per_day {
            if !(MAX_TRADES_RANGE.0..=MAX_TRADES_RANGE.1).contains(&trades) {
                return Err(format!(
                    "Buys per day must be between {} and {}.",
                    MAX_TRADES_RANGE.0, MAX_TRADES_RANGE.1
                ));
            }
            next.max_trades_per_day = trades;
        }
        if let Some(spent) = change.max_loss_cents_per_day {
            if !(MAX_SPENT_RANGE.0..=MAX_SPENT_RANGE.1).contains(&spent) {
                return Err(format!(
                    "The most spent per day must be between {} and {}.",
                    dollars(MAX_SPENT_RANGE.0),
                    dollars(MAX_SPENT_RANGE.1)
                ));
            }
            next.max_loss_cents_per_day = spent;
        }
        if let Some(confirm) = change.confirm_each_buy {
            next.confirm_each_buy = confirm;
        }
        if let Some(steps) = change.max_steps {
            if !(MAX_STEPS_RANGE.0..=MAX_STEPS_RANGE.1).contains(&steps) {
                return Err(format!(
                    "Steps per run must be between {} and {}.",
                    MAX_STEPS_RANGE.0, MAX_STEPS_RANGE.1
                ));
            }
            next.max_steps = steps;
        }
        if let Some(app) = change.app {
            let app = clean_name(&app)?;
            if app.is_empty() {
                return Err("Name the one app the agent may use.".into());
            }
            next.app = app;
        }
        if let Some(market) = change.market {
            next.market = clean_name(&market)?;
        }
        if let Some(text) = change.playbook {
            let text = text.trim();
            if text.chars().count() > MAX_PLAYBOOK_CHARS {
                return Err(format!(
                    "The playbook is too long. Keep it under {MAX_PLAYBOOK_CHARS} characters."
                ));
            }
            next.playbook = if text.is_empty() || text == DEFAULT_PLAYBOOK {
                None
            } else {
                Some(text.to_string())
            };
        }
        *self = next;
        Ok(())
    }

    /// The most that can be lost today under these limits.
    pub fn worst_case_cents(&self) -> Cents {
        self.max_loss_cents_per_day
            .min(self.stake_cents.saturating_mul(self.max_trades_per_day))
    }

    /// The rules in one line: "Kalshi only · $0.10 · 3/10 buys ·
    /// $0.30/$1.00".
    pub fn summary(&self, today: &Counters) -> String {
        format!(
            "{} only · {} · {}/{} buys · {}/{}",
            self.app,
            dollars(self.stake_cents),
            today.trades,
            self.max_trades_per_day,
            dollars(today.spent_cents),
            dollars(self.max_loss_cents_per_day)
        )
    }

    /// Whether `name` is the allowed app, however it is capitalised.
    pub fn is_allowed_app(&self, name: &str) -> bool {
        name.trim().to_lowercase() == self.app.trim().to_lowercase()
    }

    /// What the chat shows as the goal of a trading run.
    pub fn goal_label(&self, request: &str) -> String {
        let request = request.trim();
        if request.is_empty() {
            "Trading run: follow the playbook".to_string()
        } else {
            request.to_string()
        }
    }

    /// The goal the model is sent for a trading run. The playbook goes
    /// with every step as instructions, so the goal is only the request.
    pub fn goal(&self, request: &str) -> String {
        let request = request.trim();
        if request.is_empty() {
            "Follow the trading playbook in your instructions.".to_string()
        } else {
            request.to_string()
        }
    }

    /// The limits in words, as the model is told them. They are enforced
    /// whatever the model does; this only saves it from walking into them.
    pub fn hard_limits(&self) -> String {
        format!(
            "Hard limits, enforced by Edart on every action (breaking one ends the run):\n\
             - Only the {app} app. On the Home Screen only the {app} icon can be tapped; open it with open_app \"{app}\".\n\
             - Buying only in {}.\n\
             - Every buy exactly {}: the order sheet must read that amount before the slide.\n\
             - At most {} buys a day and at most {} spent a day.\n\
             - No typing on the order sheet, and no deposits, withdrawals or transfers.{}",
            if self.market.is_empty() {
                "that app".to_string()
            } else {
                format!("the {} market", self.market)
            },
            dollars(self.stake_cents),
            self.max_trades_per_day,
            dollars(self.max_loss_cents_per_day),
            if self.confirm_each_buy {
                "\n- The user is asked to approve each buy before it is made."
            } else {
                ""
            },
            app = self.app
        )
    }

    /// What is sent as instructions with every step: the user's standing
    /// instructions and, in trading mode, the playbook and the limits.
    pub fn instructions(&self, standing: Option<&str>) -> Option<String> {
        let standing = standing.map(str::trim).filter(|s| !s.is_empty());
        if !self.enabled {
            return standing.map(str::to_string);
        }
        let mut parts: Vec<String> = standing.map(str::to_string).into_iter().collect();
        parts.push(self.playbook().to_string());
        parts.push(self.hard_limits());
        Some(parts.join("\n\n"))
    }
}

/// An app or market name: one short line.
fn clean_name(name: &str) -> Result<String, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(format!(
            "Keep the name under {MAX_NAME_CHARS} characters, on one line."
        ));
    }
    Ok(name)
}

/// A buy waiting for the user's yes or no.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingBuy {
    /// "Up, $0.10, ZEC 15 min".
    pub summary: String,
    pub buy: BuyInfo,
    /// How long is left to answer.
    pub seconds_left: u32,
}

/// Trading mode, as the Trading screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradingView {
    pub enabled: bool,
    pub stake_cents: Cents,
    pub max_trades_per_day: u32,
    pub max_loss_cents_per_day: Cents,
    pub confirm_each_buy: bool,
    pub max_steps: u32,
    pub app: String,
    pub market: String,
    /// The most that can be lost in a day under these limits.
    pub worst_case_cents: Cents,
    /// The rules and today's count in one line.
    pub summary: String,
    pub playbook: String,
    pub playbook_is_default: bool,
    /// What has been bought today.
    pub today: Counters,
    /// How many more buys today's limits leave room for.
    pub trades_left: u32,
    /// Why the ledger cannot be read. Nothing can be bought until it can.
    pub ledger_error: Option<String>,
    pub pending: Option<PendingBuy>,
}

impl TradingLimits {
    /// How many more buys fit in the day's limits.
    pub fn trades_left(&self, counters: &Counters) -> u32 {
        let by_count = self.max_trades_per_day.saturating_sub(counters.trades);
        let by_money = self
            .max_loss_cents_per_day
            .saturating_sub(counters.spent_cents)
            / self.stake_cents.max(1);
        by_count.min(by_money)
    }

    pub fn view(
        &self,
        ledger: Result<&Ledger, &str>,
        now: Now,
        pending: Option<PendingBuy>,
    ) -> TradingView {
        let day = now.day();
        let (today, trades_left, ledger_error) = match ledger {
            Ok(ledger) => {
                let today = ledger.counters(&day);
                let left = self.trades_left(&today);
                (today, left, None)
            }
            Err(why) => (
                Counters {
                    day,
                    trades: 0,
                    spent_cents: 0,
                },
                0,
                Some(why.to_string()),
            ),
        };
        TradingView {
            enabled: self.enabled,
            stake_cents: self.stake_cents,
            max_trades_per_day: self.max_trades_per_day,
            max_loss_cents_per_day: self.max_loss_cents_per_day,
            confirm_each_buy: self.confirm_each_buy,
            max_steps: self.max_steps,
            app: self.app.clone(),
            market: self.market.clone(),
            worst_case_cents: self.worst_case_cents(),
            summary: self.summary(&today),
            playbook: self.playbook().to_string(),
            playbook_is_default: self.playbook.is_none(),
            today,
            trades_left,
            ledger_error,
            pending,
        }
    }
}

/// "$0.10".
pub fn dollars(cents: Cents) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_users_limits_and_off() {
        let limits = TradingLimits::default();
        assert!(!limits.enabled);
        assert_eq!(limits.stake_cents, 10);
        assert_eq!(limits.max_trades_per_day, 10);
        assert_eq!(limits.max_loss_cents_per_day, 100);
        assert!(!limits.confirm_each_buy);
        assert_eq!(limits.max_steps, 120);
        assert_eq!(limits.playbook(), DEFAULT_PLAYBOOK);
        assert!(DEFAULT_PLAYBOOK.chars().count() <= MAX_PLAYBOOK_CHARS);
    }

    #[test]
    fn an_old_settings_file_reads_as_the_defaults() {
        let limits: TradingLimits = serde_json::from_str("{}").unwrap();
        assert_eq!(limits, TradingLimits::default());
        let limits: TradingLimits = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(limits.enabled);
        assert_eq!(limits.stake_cents, 10);
    }

    #[test]
    fn a_hand_edited_file_cannot_go_past_the_bounds() {
        let limits: TradingLimits = serde_json::from_str(
            r#"{"stake_cents":100000,"max_trades_per_day":9999,"max_loss_cents_per_day":5000000,"max_steps":100000,"market":" ","app":"  "}"#,
        )
        .unwrap();
        let limits = limits.sanitized();
        assert_eq!(limits.stake_cents, 500);
        assert_eq!(limits.max_trades_per_day, 100);
        assert_eq!(limits.max_loss_cents_per_day, 10_000);
        assert_eq!(limits.max_steps, 300);
        assert_eq!(limits.market, "");
        assert_eq!(limits.app, "Kalshi");
        let zero: TradingLimits = serde_json::from_str(
            r#"{"stake_cents":0,"max_trades_per_day":0,"max_loss_cents_per_day":0}"#,
        )
        .unwrap();
        let zero = zero.sanitized();
        assert_eq!((zero.stake_cents, zero.max_trades_per_day), (1, 1));
        assert_eq!(zero.max_loss_cents_per_day, 1);
    }

    fn change() -> TradingChange {
        TradingChange::default()
    }

    #[test]
    fn every_rule_can_be_changed_within_its_range() {
        let mut limits = TradingLimits::default();
        limits
            .apply(TradingChange {
                enabled: Some(true),
                stake_cents: Some(25),
                max_trades_per_day: Some(5),
                max_loss_cents_per_day: Some(50),
                confirm_each_buy: Some(true),
                max_steps: Some(60),
                app: Some("  Robinhood ".into()),
                market: Some(" BTC  15 min ".into()),
                playbook: Some("  Only watch.  ".into()),
            })
            .unwrap();
        assert!(limits.enabled && limits.confirm_each_buy);
        assert_eq!(limits.stake_cents, 25);
        assert_eq!(limits.max_trades_per_day, 5);
        assert_eq!(limits.max_loss_cents_per_day, 50);
        assert_eq!(limits.max_steps, 60);
        assert_eq!(limits.app, "Robinhood");
        assert_eq!(limits.market, "BTC 15 min");
        assert_eq!(limits.playbook(), "Only watch.");
        // The ends of each range are inside it.
        for ok in [
            TradingChange {
                stake_cents: Some(1),
                ..change()
            },
            TradingChange {
                stake_cents: Some(500),
                ..change()
            },
            TradingChange {
                max_trades_per_day: Some(1),
                ..change()
            },
            TradingChange {
                max_trades_per_day: Some(100),
                ..change()
            },
            TradingChange {
                max_loss_cents_per_day: Some(1),
                ..change()
            },
            TradingChange {
                max_loss_cents_per_day: Some(10_000),
                ..change()
            },
            TradingChange {
                market: Some(String::new()),
                ..change()
            },
        ] {
            limits.apply(ok).unwrap();
        }
        assert_eq!(limits.market, "");
        // An empty playbook goes back to the built-in one.
        limits
            .apply(TradingChange {
                playbook: Some(String::new()),
                ..change()
            })
            .unwrap();
        assert_eq!(limits.playbook, None);
    }

    #[test]
    fn out_of_range_values_are_refused_with_a_reason_and_change_nothing() {
        let mut limits = TradingLimits::default();
        let before = limits.clone();
        for (bad, says) in [
            (
                TradingChange {
                    stake_cents: Some(0),
                    ..change()
                },
                "between $0.01 and $5.00",
            ),
            (
                TradingChange {
                    stake_cents: Some(501),
                    ..change()
                },
                "between $0.01 and $5.00",
            ),
            (
                TradingChange {
                    max_trades_per_day: Some(0),
                    ..change()
                },
                "between 1 and 100",
            ),
            (
                TradingChange {
                    max_trades_per_day: Some(101),
                    ..change()
                },
                "between 1 and 100",
            ),
            (
                TradingChange {
                    max_loss_cents_per_day: Some(0),
                    ..change()
                },
                "between $0.01 and $100.00",
            ),
            (
                TradingChange {
                    max_loss_cents_per_day: Some(10_001),
                    ..change()
                },
                "between $0.01 and $100.00",
            ),
            (
                TradingChange {
                    max_steps: Some(5),
                    ..change()
                },
                "between 10 and 300",
            ),
            (
                TradingChange {
                    app: Some("  ".into()),
                    ..change()
                },
                "Name the one app",
            ),
            (
                TradingChange {
                    app: Some("x".repeat(41)),
                    ..change()
                },
                "under 40 characters",
            ),
            (
                TradingChange {
                    market: Some("x".repeat(41)),
                    ..change()
                },
                "under 40 characters",
            ),
            (
                TradingChange {
                    playbook: Some("x".repeat(MAX_PLAYBOOK_CHARS + 1)),
                    ..change()
                },
                "too long",
            ),
            // One bad field spoils the whole change.
            (
                TradingChange {
                    enabled: Some(true),
                    stake_cents: Some(9999),
                    ..change()
                },
                "amount per buy",
            ),
        ] {
            let why = limits.apply(bad).unwrap_err();
            assert!(why.contains(says), "{why}");
            assert_eq!(limits, before);
        }
    }

    #[test]
    fn the_worst_case_is_the_nearer_of_the_two_caps() {
        let limits = TradingLimits::default();
        assert_eq!(limits.worst_case_cents(), 100);
        let few = TradingLimits {
            max_trades_per_day: 3,
            ..limits.clone()
        };
        assert_eq!(few.worst_case_cents(), 30);
        let big = TradingLimits {
            stake_cents: 500,
            max_trades_per_day: 100,
            ..limits.clone()
        };
        assert_eq!(big.worst_case_cents(), 100);
        let today = Counters {
            day: "2026-11-19".into(),
            trades: 3,
            spent_cents: 30,
        };
        assert_eq!(
            limits.summary(&today),
            "Kalshi only · $0.10 · 3/10 buys · $0.30/$1.00"
        );
        assert!(limits.is_allowed_app(" kalshi "));
        assert!(!limits.is_allowed_app("Kalshi Pro") && !limits.is_allowed_app("Robinhood"));
    }

    #[test]
    fn the_playbook_and_limits_travel_as_instructions_not_as_the_goal() {
        let off = TradingLimits::default();
        assert_eq!(off.instructions(None), None);
        assert_eq!(off.instructions(Some("  ")), None);
        assert_eq!(
            off.instructions(Some(" Be brief. ")).as_deref(),
            Some("Be brief.")
        );

        let on = TradingLimits {
            enabled: true,
            ..TradingLimits::default()
        };
        let text = on.instructions(Some("Be brief.")).unwrap();
        assert!(text.starts_with("Be brief.\n\n"));
        assert!(text.contains(DEFAULT_PLAYBOOK));
        for needle in [
            "exactly $0.10",
            "At most 10 buys a day and at most $1.00 spent a day",
            "ZEC 15 min market",
            "Only the Kalshi app",
        ] {
            assert!(text.contains(needle), "{needle}");
        }
        assert!(!text.contains("approve each buy"));
        assert!(on.instructions(None).unwrap().starts_with(DEFAULT_PLAYBOOK));
        // The limits in the text are the ones in force.
        let custom = TradingLimits {
            max_trades_per_day: 4,
            max_loss_cents_per_day: 40,
            confirm_each_buy: true,
            playbook: Some("Only watch.".into()),
            ..on.clone()
        };
        let text = custom.instructions(None).unwrap();
        assert!(text.starts_with("Only watch.\n\n"));
        assert!(text.contains("At most 4 buys a day and at most $0.40 spent a day"));
        assert!(text.contains("approve each buy"));

        assert_eq!(
            on.goal(" "),
            "Follow the trading playbook in your instructions."
        );
        assert_eq!(on.goal(" Sell it "), "Sell it");
        assert_eq!(on.goal_label(" "), "Trading run: follow the playbook");
        assert_eq!(on.goal_label(" Sell it "), "Sell it");
    }

    #[test]
    fn the_view_counts_what_is_left_today() {
        use crate::ledger::{EventKind, LedgerEvent};
        let now = Now {
            unix: 1_795_118_400,
            utc_offset: 0,
        };
        let mut ledger = Ledger::default();
        for _ in 0..3 {
            ledger.record(
                now,
                LedgerEvent {
                    time: 0,
                    day: String::new(),
                    kind: EventKind::Buy,
                    side: None,
                    stake_cents: 10,
                    market: None,
                    reason: None,
                },
            );
        }
        let limits = TradingLimits::default();
        let view = limits.view(Ok(&ledger), now, None);
        assert_eq!((view.today.trades, view.today.spent_cents), (3, 30));
        assert_eq!(view.trades_left, 7);
        assert!(view.playbook_is_default && view.ledger_error.is_none());
        // Whichever limit is nearer decides.
        let tight = TradingLimits {
            max_loss_cents_per_day: 45,
            ..TradingLimits::default()
        };
        assert_eq!(tight.view(Ok(&ledger), now, None).trades_left, 1);
        // The next day starts from nothing; a broken ledger leaves nothing.
        let tomorrow = Now {
            unix: now.unix + 86_400,
            utc_offset: 0,
        };
        assert_eq!(limits.view(Ok(&ledger), tomorrow, None).trades_left, 10);
        let broken = limits.view(Err("damaged"), now, None);
        assert_eq!(broken.trades_left, 0);
        assert_eq!(broken.ledger_error.as_deref(), Some("damaged"));
        let json = serde_json::to_string(&view).unwrap();
        assert!(json.contains("\"maxTradesPerDay\":10") && json.contains("\"spentCents\":30"));
    }

    #[test]
    fn money_is_written_with_two_decimals() {
        assert_eq!(dollars(10), "$0.10");
        assert_eq!(dollars(100), "$1.00");
        assert_eq!(dollars(1586), "$15.86");
        assert_eq!(dollars(0), "$0.00");
    }
}
