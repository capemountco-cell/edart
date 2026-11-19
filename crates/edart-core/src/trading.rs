//! Trading mode: the limits the user sets, and the playbook the agent is
//! given.
//!
//! The limits are enforced by `crate::guard` before any gesture reaches
//! the iPhone; the playbook only tells the model what the limits are so
//! it does not waste steps running into them. Nothing here is a promise
//! about how well the model trades.

use serde::{Deserialize, Serialize};

use crate::agent::MAX_GOAL_CHARS;
use crate::guard::BuyInfo;
use crate::ledger::{Counters, Ledger, Now};

/// Money, in cents.
pub type Cents = u32;

/// What a trade costs unless the settings file says otherwise.
pub const STAKE_CENTS: Cents = 10;
const MAX_STAKE_CENTS: Cents = 100;
pub const MAX_TRADES_RANGE: (u32, u32) = (1, 50);
/// The daily loss limit can be set between one stake and ten dollars.
pub const MAX_LOSS_TOP_CENTS: Cents = 1000;
const MAX_STEPS_RANGE: (u32, u32) = (10, 300);
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
3. After a buy, the user may be asked to approve it. Then watch the position: wait up to 30 seconds at a time and check again.
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
    /// Ask the user before every buy.
    pub confirm_each_buy: bool,
    /// Most agent steps in one run while trading mode is on.
    pub max_steps: u32,
    /// A buy is allowed only in a market whose name contains this.
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
            confirm_each_buy: true,
            max_steps: 120,
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
    pub max_trades_per_day: Option<u32>,
    #[serde(default)]
    pub max_loss_cents_per_day: Option<Cents>,
    #[serde(default)]
    pub confirm_each_buy: Option<bool>,
    /// Empty goes back to the built-in playbook.
    #[serde(default)]
    pub playbook: Option<String>,
}

impl TradingLimits {
    /// The limits with every number pulled inside its bounds, so a
    /// hand-edited settings file cannot widen them past what the screen
    /// allows.
    pub fn sanitized(mut self) -> Self {
        self.stake_cents = self.stake_cents.clamp(1, MAX_STAKE_CENTS);
        self.max_trades_per_day = self
            .max_trades_per_day
            .clamp(MAX_TRADES_RANGE.0, MAX_TRADES_RANGE.1);
        self.max_loss_cents_per_day = self
            .max_loss_cents_per_day
            .clamp(self.stake_cents, MAX_LOSS_TOP_CENTS);
        self.max_steps = self.max_steps.clamp(MAX_STEPS_RANGE.0, MAX_STEPS_RANGE.1);
        if self.market.trim().is_empty() {
            self.market = TradingLimits::default().market;
        }
        self
    }

    pub fn playbook(&self) -> &str {
        self.playbook.as_deref().unwrap_or(DEFAULT_PLAYBOOK)
    }

    /// Apply a change, or say why it is not acceptable. Nothing changes
    /// on an error.
    pub fn apply(&mut self, change: TradingChange) -> Result<(), String> {
        if let Some(trades) = change.max_trades_per_day {
            if !(MAX_TRADES_RANGE.0..=MAX_TRADES_RANGE.1).contains(&trades) {
                return Err(format!(
                    "Trades per day must be between {} and {}.",
                    MAX_TRADES_RANGE.0, MAX_TRADES_RANGE.1
                ));
            }
        }
        if let Some(loss) = change.max_loss_cents_per_day {
            if !(self.stake_cents..=MAX_LOSS_TOP_CENTS).contains(&loss) {
                return Err(format!(
                    "The daily loss limit must be between {} and {}.",
                    dollars(self.stake_cents),
                    dollars(MAX_LOSS_TOP_CENTS)
                ));
            }
        }
        let playbook = match change.playbook {
            None => None,
            Some(text) => {
                let text = text.trim();
                if text.chars().count() > MAX_PLAYBOOK_CHARS {
                    return Err(format!(
                        "The playbook is too long. Keep it under {MAX_PLAYBOOK_CHARS} characters."
                    ));
                }
                Some(if text.is_empty() || text == DEFAULT_PLAYBOOK {
                    None
                } else {
                    Some(text.to_string())
                })
            }
        };
        if let Some(enabled) = change.enabled {
            self.enabled = enabled;
        }
        if let Some(trades) = change.max_trades_per_day {
            self.max_trades_per_day = trades;
        }
        if let Some(loss) = change.max_loss_cents_per_day {
            self.max_loss_cents_per_day = loss;
        }
        if let Some(confirm) = change.confirm_each_buy {
            self.confirm_each_buy = confirm;
        }
        if let Some(playbook) = playbook {
            self.playbook = playbook;
        }
        Ok(())
    }

    /// The goal the model is sent: the playbook, then what was asked.
    pub fn compose_goal(&self, request: &str) -> Result<String, String> {
        let request = request.trim();
        let goal = if request.is_empty() {
            self.playbook().to_string()
        } else {
            format!("{}\n\nRequest: {request}", self.playbook())
        };
        if goal.chars().count() > MAX_GOAL_CHARS {
            return Err(format!(
                "That’s too long with the trading playbook in front. Keep the two under {MAX_GOAL_CHARS} characters together."
            ));
        }
        Ok(goal)
    }
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
    pub market: String,
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
            market: self.market.clone(),
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
        assert!(limits.confirm_each_buy);
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
    fn a_hand_edited_file_cannot_widen_the_limits() {
        let limits: TradingLimits = serde_json::from_str(
            r#"{"stake_cents":100000,"max_trades_per_day":9999,"max_loss_cents_per_day":5000000,"max_steps":100000,"market":" "}"#,
        )
        .unwrap();
        let limits = limits.sanitized();
        assert_eq!(limits.stake_cents, 100);
        assert_eq!(limits.max_trades_per_day, 50);
        assert_eq!(limits.max_loss_cents_per_day, 1000);
        assert_eq!(limits.max_steps, 300);
        assert_eq!(limits.market, "ZEC 15 min");
        let zero: TradingLimits = serde_json::from_str(
            r#"{"stake_cents":0,"max_trades_per_day":0,"max_loss_cents_per_day":0}"#,
        )
        .unwrap();
        let zero = zero.sanitized();
        assert_eq!((zero.stake_cents, zero.max_trades_per_day), (1, 1));
        assert_eq!(zero.max_loss_cents_per_day, 1);
    }

    #[test]
    fn changes_are_checked_and_all_or_nothing() {
        let mut limits = TradingLimits::default();
        limits
            .apply(TradingChange {
                enabled: Some(true),
                max_trades_per_day: Some(5),
                max_loss_cents_per_day: Some(50),
                confirm_each_buy: Some(false),
                playbook: Some("  Only watch.  ".into()),
            })
            .unwrap();
        assert!(limits.enabled && !limits.confirm_each_buy);
        assert_eq!(limits.max_trades_per_day, 5);
        assert_eq!(limits.max_loss_cents_per_day, 50);
        assert_eq!(limits.playbook(), "Only watch.");

        let before = limits.clone();
        for bad in [
            TradingChange {
                enabled: Some(false),
                max_trades_per_day: Some(0),
                ..Default::default()
            },
            TradingChange {
                max_trades_per_day: Some(51),
                ..Default::default()
            },
            TradingChange {
                max_loss_cents_per_day: Some(9),
                ..Default::default()
            },
            TradingChange {
                max_loss_cents_per_day: Some(1001),
                ..Default::default()
            },
            TradingChange {
                enabled: Some(false),
                playbook: Some("x".repeat(MAX_PLAYBOOK_CHARS + 1)),
                ..Default::default()
            },
        ] {
            assert!(limits.apply(bad).is_err());
            assert_eq!(limits, before);
        }
        // An empty playbook goes back to the built-in one.
        limits
            .apply(TradingChange {
                playbook: Some(String::new()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(limits.playbook, None);
    }

    #[test]
    fn the_goal_is_the_playbook_then_the_request() {
        let limits = TradingLimits::default();
        assert_eq!(limits.compose_goal("  ").unwrap(), DEFAULT_PLAYBOOK);
        let goal = limits.compose_goal("Check my position").unwrap();
        assert!(goal.starts_with(DEFAULT_PLAYBOOK));
        assert!(goal.ends_with("Request: Check my position"));
        assert!(limits.compose_goal(&"x".repeat(MAX_GOAL_CHARS)).is_err());
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
