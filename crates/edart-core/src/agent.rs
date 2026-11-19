//! The agent loop's decisions: what to send, what an answer means, when to
//! stop.
//!
//! One step is: take a screenshot, ask the API for the next action, do it,
//! let the screen settle. The native side does the screenshot, the HTTPS
//! call and the gesture; everything it has to decide is decided here, so
//! it is tested without a phone or a network.

use serde::{Deserialize, Serialize};

use crate::gesture::{Gesture, Screen};

/// Who said a line of the history the API is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// The body of `POST /api/agent/step`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepRequest<'a> {
    pub goal: &'a str,
    pub history: &'a [Message],
    /// Base64 JPEG, no `data:` prefix.
    pub screenshot: &'a str,
    pub screen: Screen,
}

/// What the API wants done next. Coordinates are screenshot pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
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
        ms: Option<f64>,
    },
    Swipe {
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        #[serde(default)]
        ms: Option<f64>,
    },
    TypeText {
        text: String,
    },
    PressHome,
    Wait {
        #[serde(default)]
        ms: Option<f64>,
    },
    Done {
        #[serde(default)]
        success: bool,
        #[serde(default)]
        summary: String,
    },
}

/// The API's answer to one step.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StepResponse {
    pub action: Action,
    #[serde(default)]
    pub thought: String,
}

/// What an action comes to on this side.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Perform(Gesture),
    WaitMs(u64),
    Finish { success: bool, summary: String },
}

/// Bounds for durations the model picks, so a wild number cannot hold a
/// finger down for a minute or stall the loop.
const WAIT_MS: (f64, f64) = (100.0, 5000.0);
/// The longest wait in one step, unless the run's limits allow more.
pub const MAX_WAIT_MS: u64 = WAIT_MS.1 as u64;
const LONG_PRESS_MS: (f64, f64) = (300.0, 5000.0);
const SWIPE_MS: (f64, f64) = (100.0, 3000.0);
const DEFAULT_WAIT_MS: f64 = 1000.0;

fn duration(ms: Option<f64>, (low, high): (f64, f64)) -> Option<u64> {
    ms.filter(|ms| ms.is_finite())
        .map(|ms| ms.clamp(low, high) as u64)
}

/// Pull a point onto the screen. Models aim a few pixels past an edge often
/// enough that refusing would waste a step; a point that is not a number
/// is refused.
fn on_screen(x: f64, y: f64, screen: Screen) -> Result<(f64, f64), String> {
    if !(x.is_finite() && y.is_finite()) {
        return Err("the coordinates are not numbers".into());
    }
    Ok((x.clamp(0.0, screen.width), y.clamp(0.0, screen.height)))
}

impl Action {
    /// Turn the action into something to do.
    pub fn effect(&self, screen: Screen) -> Result<Effect, String> {
        self.effect_within(screen, MAX_WAIT_MS)
    }

    /// How long a wait action waits when the longest allowed is
    /// `max_wait_ms`.
    fn wait_ms(ms: Option<f64>, max_wait_ms: u64) -> u64 {
        let range = (WAIT_MS.0, (max_wait_ms as f64).max(WAIT_MS.0));
        duration(Some(ms.unwrap_or(DEFAULT_WAIT_MS)), range).unwrap_or(DEFAULT_WAIT_MS as u64)
    }

    /// [`Action::effect`], with a run's own limit on waiting.
    pub fn effect_within(&self, screen: Screen, max_wait_ms: u64) -> Result<Effect, String> {
        Ok(match self {
            Action::Tap { x, y } => {
                let (x, y) = on_screen(*x, *y, screen)?;
                Effect::Perform(Gesture::Tap { x, y })
            }
            Action::DoubleTap { x, y } => {
                let (x, y) = on_screen(*x, *y, screen)?;
                Effect::Perform(Gesture::DoubleTap { x, y })
            }
            Action::LongPress { x, y, ms } => {
                let (x, y) = on_screen(*x, *y, screen)?;
                Effect::Perform(Gesture::LongPress {
                    x,
                    y,
                    ms: duration(*ms, LONG_PRESS_MS),
                })
            }
            Action::Swipe {
                from_x,
                from_y,
                to_x,
                to_y,
                ms,
            } => {
                let (from_x, from_y) = on_screen(*from_x, *from_y, screen)?;
                let (to_x, to_y) = on_screen(*to_x, *to_y, screen)?;
                Effect::Perform(Gesture::Swipe {
                    from_x,
                    from_y,
                    to_x,
                    to_y,
                    ms: duration(*ms, SWIPE_MS),
                })
            }
            Action::TypeText { text } => {
                if text.is_empty() {
                    return Err("there is no text to type".into());
                }
                Effect::Perform(Gesture::TypeText { text: text.clone() })
            }
            Action::PressHome => Effect::Perform(Gesture::PressHome),
            Action::Wait { ms } => Effect::WaitMs(Self::wait_ms(*ms, max_wait_ms)),
            Action::Done { success, summary } => Effect::Finish {
                success: *success,
                summary: summary.clone(),
            },
        })
    }

    /// One short line for the chat and for the history.
    pub fn describe(&self) -> String {
        self.describe_within(MAX_WAIT_MS)
    }

    /// [`Action::describe`], with a run's own limit on waiting.
    pub fn describe_within(&self, max_wait_ms: u64) -> String {
        let at = |x: &f64, y: &f64| format!("({}, {})", x.round(), y.round());
        match self {
            Action::Tap { x, y } => format!("Tap {}", at(x, y)),
            Action::DoubleTap { x, y } => format!("Double tap {}", at(x, y)),
            Action::LongPress { x, y, .. } => format!("Tap and hold {}", at(x, y)),
            Action::Swipe {
                from_x,
                from_y,
                to_x,
                to_y,
                ..
            } => format!("Swipe {} → {}", at(from_x, from_y), at(to_x, to_y)),
            Action::TypeText { text } => format!("Type “{}”", shorten(text, 60)),
            Action::PressHome => "Go Home".into(),
            Action::Wait { ms } => format!(
                "Wait {:.1}s",
                Self::wait_ms(*ms, max_wait_ms) as f64 / 1000.0
            ),
            Action::Done { .. } => "Finish".into(),
        }
    }
}

fn shorten(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut short: String = text.chars().take(max_chars).collect();
    short.push('…');
    short
}

/// Why asking the API for a step failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepFailure {
    /// The token is missing or wrong. Trying again cannot help.
    Unauthorized,
    /// The API refused the request itself. Trying again cannot help.
    Rejected(String),
    /// The API or the model behind it had a problem; the next try may work.
    Upstream(String),
    /// The request never got an answer.
    Network(String),
}

impl StepFailure {
    /// Whether the loop should give up at once.
    pub fn is_fatal(&self) -> bool {
        matches!(self, StepFailure::Unauthorized | StepFailure::Rejected(_))
    }

    /// The line shown to the user.
    pub fn message(&self) -> String {
        match self {
            StepFailure::Unauthorized => {
                "The AI token was refused. Check it in Preferences.".into()
            }
            StepFailure::Rejected(why) => format!("The AI service refused the request: {why}"),
            StepFailure::Upstream(why) => format!("The AI service had a problem: {why}"),
            StepFailure::Network(why) => format!("Couldn’t reach the AI service: {why}"),
        }
    }
}

/// The `error` field of an API error body, or a fallback.
fn error_text(status: u16, body: &str) -> String {
    #[derive(Deserialize)]
    struct ErrorBody {
        error: String,
    }
    match serde_json::from_str::<ErrorBody>(body) {
        Ok(parsed) if !parsed.error.is_empty() => shorten(&parsed.error, 200),
        _ => format!("HTTP {status}"),
    }
}

/// Read the API's answer to a step.
pub fn parse_step(status: u16, body: &str) -> Result<StepResponse, StepFailure> {
    match status {
        200 => serde_json::from_str(body)
            .map_err(|e| StepFailure::Upstream(format!("an answer Edart can’t read ({e})"))),
        401 | 403 => Err(StepFailure::Unauthorized),
        400..=499 => Err(StepFailure::Rejected(error_text(status, body))),
        _ => Err(StepFailure::Upstream(error_text(status, body))),
    }
}

/// How the token check in Preferences went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TokenCheck {
    /// The service answered and accepted the token.
    Accepted,
    /// The service answered and refused the token.
    Refused,
    /// The service is there but is missing its own configuration.
    ServiceBroken,
    /// The answer made no sense for this API: probably the wrong address.
    Unexpected,
}

/// Judge the token from the answer to a deliberately empty step request.
/// The API checks the token before it reads the body, so a wrong token is
/// 401 and a right one gets as far as "invalid input" (400).
pub fn judge_token_probe(status: u16) -> TokenCheck {
    match status {
        400 | 200 => TokenCheck::Accepted,
        401 | 403 => TokenCheck::Refused,
        500..=599 => TokenCheck::ServiceBroken,
        _ => TokenCheck::Unexpected,
    }
}

/// What the Test button in Preferences reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTest {
    /// The service answered its health check.
    pub reachable: bool,
    /// How the token fared; `None` when it was not tried.
    pub token: Option<TokenCheck>,
    /// One sentence for the user.
    pub message: String,
}

/// Put the two probes together. `health` is the status of
/// `GET /api/health` (or why it failed); `probe` is the status of the
/// empty step request, when a token is saved and the service is up.
pub fn connection_test(
    health: Result<u16, String>,
    probe: Option<Result<u16, String>>,
) -> ConnectionTest {
    let unreachable = |why: String| ConnectionTest {
        reachable: false,
        token: None,
        message: format!("Couldn’t reach the AI service: {why}."),
    };
    match health {
        Err(why) => return unreachable(why),
        Ok(200) => {}
        Ok(status) => return unreachable(format!("it answered HTTP {status}")),
    }
    let (token, message) = match probe {
        None => (
            None,
            "The AI service is up. Add your token to use it.".to_string(),
        ),
        Some(Err(why)) => (
            None,
            format!("The AI service is up, but the token check failed: {why}."),
        ),
        Some(Ok(status)) => {
            let check = judge_token_probe(status);
            let message = match check {
                TokenCheck::Accepted => "Connected. The token works.".to_string(),
                TokenCheck::Refused => {
                    "The AI service is up, but it refused this token.".to_string()
                }
                TokenCheck::ServiceBroken => {
                    "The AI service is up but isn’t set up correctly on its side.".to_string()
                }
                TokenCheck::Unexpected => format!(
                    "That address answered, but not like the Edart service (HTTP {status})."
                ),
            };
            (Some(check), message)
        }
    };
    ConnectionTest {
        reachable: true,
        token,
        message,
    }
}

/// When to give up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Most actions in one run.
    pub max_steps: u32,
    /// This many failures in a row end the run.
    pub max_errors_in_a_row: u32,
    /// Most history messages sent with a request (the API allows 200).
    pub max_history: usize,
    /// The longest one wait action may take.
    pub max_wait_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_steps: 25,
            max_errors_in_a_row: 3,
            max_history: 40,
            max_wait_ms: MAX_WAIT_MS,
        }
    }
}

impl Limits {
    /// The limits of a run in trading mode: long enough to watch a
    /// fifteen-minute market, with waits of up to half a minute.
    pub fn for_trading(max_steps: u32) -> Limits {
        Limits {
            max_steps,
            max_wait_ms: crate::trading::TRADING_WAIT_MS,
            ..Limits::default()
        }
    }
}

/// Wait this long after an action for the screen to settle before the next
/// screenshot.
pub const SETTLE_MS: u64 = 750;
/// Wait this long before asking again after a failed request.
pub const RETRY_MS: u64 = 1500;
/// The API accepts goals up to this many characters.
pub const MAX_GOAL_CHARS: usize = 4000;

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The model said it was finished.
    Done { success: bool, summary: String },
    /// The step limit ran out first.
    OutOfSteps(u32),
    /// The user pressed Stop.
    Stopped,
    /// Something kept failing.
    Failed(String),
}

impl Outcome {
    pub fn success(&self) -> bool {
        matches!(self, Outcome::Done { success: true, .. })
    }

    /// The closing line in the chat.
    pub fn summary(&self) -> String {
        match self {
            Outcome::Done { summary, success } if summary.trim().is_empty() => if *success {
                "Done."
            } else {
                "Couldn’t finish."
            }
            .into(),
            Outcome::Done { summary, .. } => summary.trim().to_string(),
            Outcome::OutOfSteps(steps) => {
                format!("Stopped after {steps} steps without finishing. Ask again to continue.")
            }
            Outcome::Stopped => "Stopped.".into(),
            Outcome::Failed(why) => why.clone(),
        }
    }
}

/// One run of the agent towards a goal.
#[derive(Debug, Clone)]
pub struct Run {
    goal: String,
    history: Vec<Message>,
    steps: u32,
    errors_in_a_row: u32,
    limits: Limits,
}

impl Run {
    pub fn new(goal: &str, limits: Limits) -> Result<Run, String> {
        let goal = goal.trim();
        if goal.is_empty() {
            return Err("Say what the agent should do.".into());
        }
        if goal.chars().count() > MAX_GOAL_CHARS {
            return Err(format!(
                "That’s too long. Keep it under {MAX_GOAL_CHARS} characters."
            ));
        }
        Ok(Run {
            goal: goal.to_string(),
            history: Vec::new(),
            steps: 0,
            errors_in_a_row: 0,
            limits,
        })
    }

    pub fn goal(&self) -> &str {
        &self.goal
    }

    /// Actions taken so far.
    pub fn steps(&self) -> u32 {
        self.steps
    }

    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// Call before each step. `Some` means the run is over.
    pub fn check(&self, stop_requested: bool) -> Option<Outcome> {
        if stop_requested {
            Some(Outcome::Stopped)
        } else if self.steps >= self.limits.max_steps {
            Some(Outcome::OutOfSteps(self.steps))
        } else {
            None
        }
    }

    pub fn request<'a>(&'a self, screenshot: &'a str, screen: Screen) -> StepRequest<'a> {
        StepRequest {
            goal: &self.goal,
            history: &self.history,
            screenshot,
            screen,
        }
    }

    /// The API answered. Counts the step and says what to do.
    pub fn answered(&mut self, response: &StepResponse, screen: Screen) -> Result<Effect, String> {
        self.steps += 1;
        let thought = response.thought.trim();
        let action = response.action.describe_within(self.limits.max_wait_ms);
        self.push(
            Role::Assistant,
            if thought.is_empty() {
                action
            } else {
                format!("{thought}\nAction: {action}")
            },
        );
        response
            .action
            .effect_within(screen, self.limits.max_wait_ms)
    }

    /// The longest one wait may take in this run.
    pub fn max_wait_ms(&self) -> u64 {
        self.limits.max_wait_ms
    }

    /// The trading rails refused an action. That ends the run: the agent
    /// is not given a second try at something it was told not to do.
    pub fn refused(&self, reason: &str) -> Outcome {
        Outcome::Failed(format!("Stopped by the trading rails: {reason}."))
    }

    /// The action was carried out, or could not be. Tells the model, and
    /// ends the run after too many failures in a row.
    pub fn acted(&mut self, result: Result<(), String>) -> Option<Outcome> {
        match result {
            Ok(()) => {
                self.errors_in_a_row = 0;
                self.push(
                    Role::User,
                    "Done. The next screenshot shows the result.".into(),
                );
                None
            }
            Err(why) => {
                self.push(
                    Role::User,
                    format!("That action failed: {why}. Try something else."),
                );
                self.failed(format!("The iPhone didn’t take that action: {why}"))
            }
        }
    }

    /// Asking the API failed. `Some` means the run is over.
    pub fn request_failed(&mut self, failure: &StepFailure) -> Option<Outcome> {
        if failure.is_fatal() {
            return Some(Outcome::Failed(failure.message()));
        }
        self.failed(failure.message())
    }

    fn failed(&mut self, message: String) -> Option<Outcome> {
        self.errors_in_a_row += 1;
        (self.errors_in_a_row >= self.limits.max_errors_in_a_row)
            .then_some(Outcome::Failed(message))
    }

    /// Add to the history, dropping the oldest exchanges past the limit.
    /// The API puts the goal first as a user message, so what is kept
    /// always starts with an assistant message.
    fn push(&mut self, role: Role, content: String) {
        self.history.push(Message {
            role,
            content: shorten(&content, 4000),
        });
        let max = self.limits.max_history.max(2);
        if self.history.len() > max {
            let mut cut = self.history.len() - max;
            while cut < self.history.len() && self.history[cut].role != Role::Assistant {
                cut += 1;
            }
            self.history.drain(..cut);
        }
    }
}

/// What the agent needs before it can start, checked in the order the user
/// would fix them.
pub fn ready_to_start(
    token_set: bool,
    control_ready: bool,
    screen: Option<Screen>,
    already_running: bool,
) -> Result<Screen, String> {
    if already_running {
        return Err("The agent is already working. Stop it first.".into());
    }
    if !token_set {
        return Err("Add your AI token in Preferences first.".into());
    }
    if !control_ready {
        return Err(
            "No iPhone is connected for control. Pair it over Bluetooth in Permissions first."
                .into(),
        );
    }
    screen.ok_or_else(|| {
        "The agent needs to see the screen. Plug the iPhone in with a cable, unlock it, and allow Camera access."
            .to_string()
    })
}

/// A line of the chat, as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Entry {
    /// What the user asked for.
    Goal { text: String },
    /// One step: what the agent thought and what it did.
    Step {
        n: u32,
        thought: String,
        action: String,
    },
    /// Something went wrong; the run may go on.
    Problem { text: String },
    /// The run ended.
    Finished { success: bool, text: String },
}

/// The agent, as the UI sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub running: bool,
    /// The user pressed Stop and the current step is finishing.
    pub stopping: bool,
    pub step: u32,
    pub max_steps: u32,
    /// The whole conversation, oldest first.
    pub entries: Vec<Entry>,
}

/// Most chat lines kept. Older ones are dropped.
pub const MAX_ENTRIES: usize = 400;

impl AgentStatus {
    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
        if self.entries.len() > MAX_ENTRIES {
            let extra = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..extra);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Screen = Screen {
        width: 739.0,
        height: 1600.0,
    };

    fn answer(json: &str) -> StepResponse {
        parse_step(200, json).unwrap()
    }

    #[test]
    fn every_action_the_api_can_send_is_understood() {
        let cases = [
            (r#"{"type":"tap","x":10,"y":20}"#, "Tap (10, 20)"),
            (
                r#"{"type":"double_tap","x":10.4,"y":20.6}"#,
                "Double tap (10, 21)",
            ),
            (
                r#"{"type":"long_press","x":1,"y":2}"#,
                "Tap and hold (1, 2)",
            ),
            (
                r#"{"type":"swipe","from_x":1,"from_y":2,"to_x":3,"to_y":4,"ms":250}"#,
                "Swipe (1, 2) → (3, 4)",
            ),
            (r#"{"type":"type_text","text":"hi"}"#, "Type “hi”"),
            (r#"{"type":"press_home"}"#, "Go Home"),
            (r#"{"type":"wait","ms":1500}"#, "Wait 1.5s"),
            (r#"{"type":"done","success":true,"summary":"ok"}"#, "Finish"),
        ];
        for (action, described) in cases {
            let response = answer(&format!(r#"{{"action":{action},"thought":"t"}}"#));
            assert_eq!(response.action.describe(), described);
            assert!(response.action.effect(SCREEN).is_ok());
        }
    }

    #[test]
    fn a_missing_thought_and_extra_fields_are_fine() {
        let response = answer(r#"{"action":{"type":"tap","x":1,"y":2,"why":"x"}}"#);
        assert_eq!(response.thought, "");
        assert_eq!(response.action, Action::Tap { x: 1.0, y: 2.0 });
    }

    #[test]
    fn actions_become_gestures_in_screenshot_units() {
        let effect = Action::Swipe {
            from_x: 370.0,
            from_y: 1200.0,
            to_x: 370.0,
            to_y: 400.0,
            ms: Some(250.4),
        }
        .effect(SCREEN)
        .unwrap();
        assert_eq!(
            effect,
            Effect::Perform(Gesture::Swipe {
                from_x: 370.0,
                from_y: 1200.0,
                to_x: 370.0,
                to_y: 400.0,
                ms: Some(250),
            })
        );
        assert_eq!(
            Action::LongPress {
                x: 5.0,
                y: 6.0,
                ms: None
            }
            .effect(SCREEN)
            .unwrap(),
            Effect::Perform(Gesture::LongPress {
                x: 5.0,
                y: 6.0,
                ms: None
            })
        );
    }

    #[test]
    fn points_past_an_edge_are_pulled_in_and_nonsense_is_refused() {
        assert_eq!(
            Action::Tap { x: 760.0, y: -4.0 }.effect(SCREEN).unwrap(),
            Effect::Perform(Gesture::Tap { x: 739.0, y: 0.0 })
        );
        assert!(Action::Tap {
            x: f64::NAN,
            y: 1.0
        }
        .effect(SCREEN)
        .is_err());
        assert!(Action::TypeText { text: "".into() }.effect(SCREEN).is_err());
    }

    #[test]
    fn durations_are_kept_within_bounds() {
        let wait = |ms| Action::Wait { ms }.effect(SCREEN).unwrap();
        assert_eq!(wait(Some(1.0)), Effect::WaitMs(100));
        assert_eq!(wait(Some(600_000.0)), Effect::WaitMs(5000));
        assert_eq!(wait(None), Effect::WaitMs(1000));
        assert_eq!(wait(Some(f64::INFINITY)), Effect::WaitMs(1000));
        let press = Action::LongPress {
            x: 1.0,
            y: 1.0,
            ms: Some(99_999.0),
        };
        assert_eq!(
            press.effect(SCREEN).unwrap(),
            Effect::Perform(Gesture::LongPress {
                x: 1.0,
                y: 1.0,
                ms: Some(5000)
            })
        );
    }

    #[test]
    fn done_finishes_with_the_models_summary() {
        let response = answer(
            r#"{"action":{"type":"done","success":false,"summary":"No such app."},"thought":""}"#,
        );
        assert_eq!(
            response.action.effect(SCREEN).unwrap(),
            Effect::Finish {
                success: false,
                summary: "No such app.".into()
            }
        );
    }

    #[test]
    fn api_errors_are_sorted_into_fatal_and_retryable() {
        assert_eq!(
            parse_step(401, r#"{"error":"unauthorized"}"#),
            Err(StepFailure::Unauthorized)
        );
        let rejected = parse_step(400, r#"{"error":"screenshot too large"}"#).unwrap_err();
        assert_eq!(
            rejected,
            StepFailure::Rejected("screenshot too large".into())
        );
        assert!(rejected.is_fatal());
        let upstream = parse_step(502, r#"{"error":"openai error 429"}"#).unwrap_err();
        assert_eq!(upstream, StepFailure::Upstream("openai error 429".into()));
        assert!(!upstream.is_fatal());
        assert_eq!(
            parse_step(503, "<html>"),
            Err(StepFailure::Upstream("HTTP 503".into()))
        );
        assert!(matches!(
            parse_step(200, r#"{"action":{"type":"fly"}}"#),
            Err(StepFailure::Upstream(_))
        ));
        assert!(!StepFailure::Network("timed out".into()).is_fatal());
    }

    #[test]
    fn the_token_probe_reads_the_status() {
        assert_eq!(judge_token_probe(400), TokenCheck::Accepted);
        assert_eq!(judge_token_probe(401), TokenCheck::Refused);
        assert_eq!(judge_token_probe(500), TokenCheck::ServiceBroken);
        assert_eq!(judge_token_probe(404), TokenCheck::Unexpected);
    }

    #[test]
    fn the_connection_test_explains_each_result() {
        let down = connection_test(Err("offline".into()), None);
        assert!(!down.reachable);
        assert!(down.message.contains("offline"));
        assert!(!connection_test(Ok(503), Some(Ok(400))).reachable);

        let no_token = connection_test(Ok(200), None);
        assert!(no_token.reachable);
        assert_eq!(no_token.token, None);

        let good = connection_test(Ok(200), Some(Ok(400)));
        assert_eq!(good.token, Some(TokenCheck::Accepted));
        let bad = connection_test(Ok(200), Some(Ok(401)));
        assert_eq!(bad.token, Some(TokenCheck::Refused));
        assert!(bad.message.contains("refused"));
        let odd = connection_test(Ok(200), Some(Ok(404)));
        assert_eq!(odd.token, Some(TokenCheck::Unexpected));
        let json = serde_json::to_value(&good).unwrap();
        assert_eq!(json["token"], "accepted");
    }

    #[test]
    fn the_request_has_the_shape_the_api_expects() {
        let mut run = Run::new("  Open Settings  ", Limits::default()).unwrap();
        let response = answer(r#"{"action":{"type":"press_home"},"thought":"Start at Home."}"#);
        run.answered(&response, SCREEN).unwrap();
        run.acted(Ok(()));
        let json = serde_json::to_value(run.request("QUJD", SCREEN)).unwrap();
        assert_eq!(json["goal"], "Open Settings");
        assert_eq!(json["screenshot"], "QUJD");
        assert_eq!(json["screen"]["width"], 739.0);
        assert_eq!(json["screen"]["height"], 1600.0);
        assert_eq!(json["history"][0]["role"], "assistant");
        assert_eq!(
            json["history"][0]["content"],
            "Start at Home.\nAction: Go Home"
        );
        assert_eq!(json["history"][1]["role"], "user");
    }

    #[test]
    fn goals_must_say_something_and_fit() {
        assert!(Run::new("   ", Limits::default()).is_err());
        assert!(Run::new(&"x".repeat(MAX_GOAL_CHARS + 1), Limits::default()).is_err());
        assert!(Run::new(&"x".repeat(MAX_GOAL_CHARS), Limits::default()).is_ok());
    }

    fn tap() -> StepResponse {
        answer(r#"{"action":{"type":"tap","x":1,"y":2},"thought":""}"#)
    }

    #[test]
    fn the_run_stops_at_the_step_limit() {
        let limits = Limits {
            max_steps: 3,
            ..Limits::default()
        };
        let mut run = Run::new("go", limits).unwrap();
        for _ in 0..3 {
            assert_eq!(run.check(false), None);
            run.answered(&tap(), SCREEN).unwrap();
            assert_eq!(run.acted(Ok(())), None);
        }
        assert_eq!(run.check(false), Some(Outcome::OutOfSteps(3)));
    }

    #[test]
    fn stop_wins_over_everything() {
        let run = Run::new("go", Limits::default()).unwrap();
        assert_eq!(run.check(true), Some(Outcome::Stopped));
    }

    #[test]
    fn three_failures_in_a_row_end_the_run_but_a_success_resets_the_count() {
        let mut run = Run::new("go", Limits::default()).unwrap();
        let flaky = StepFailure::Upstream("busy".into());
        assert_eq!(run.request_failed(&flaky), None);
        assert_eq!(run.request_failed(&flaky), None);
        run.answered(&tap(), SCREEN).unwrap();
        assert_eq!(run.acted(Ok(())), None);
        assert_eq!(run.request_failed(&flaky), None);
        assert_eq!(run.acted(Err("off screen".into())), None);
        assert_eq!(
            run.request_failed(&StepFailure::Network("offline".into())),
            Some(Outcome::Failed(
                "Couldn’t reach the AI service: offline".into()
            ))
        );
    }

    #[test]
    fn a_refused_token_ends_the_run_at_once() {
        let mut run = Run::new("go", Limits::default()).unwrap();
        let outcome = run.request_failed(&StepFailure::Unauthorized).unwrap();
        assert!(outcome.summary().contains("token"));
        assert!(!outcome.success());
    }

    #[test]
    fn a_failed_action_is_reported_to_the_model() {
        let mut run = Run::new("go", Limits::default()).unwrap();
        run.answered(&tap(), SCREEN).unwrap();
        run.acted(Err("cannot type 'é' with a keyboard".into()));
        let last = run.history().last().unwrap();
        assert_eq!(last.role, Role::User);
        assert!(last.content.contains("cannot type"));
    }

    #[test]
    fn history_is_bounded_and_starts_with_the_assistant() {
        let limits = Limits {
            max_steps: 1000,
            max_history: 6,
            ..Limits::default()
        };
        let mut run = Run::new("go", limits).unwrap();
        for _ in 0..50 {
            run.answered(&tap(), SCREEN).unwrap();
            run.acted(Ok(()));
            assert!(run.history().len() <= 6);
            assert_eq!(run.history()[0].role, Role::Assistant);
        }
        assert_eq!(run.history().len(), 6);
        // Roles still alternate.
        for pair in run.history().windows(2) {
            assert_ne!(pair[0].role, pair[1].role);
        }
    }

    #[test]
    fn outcomes_read_well() {
        let done = Outcome::Done {
            success: true,
            summary: " Dark Mode is on. ".into(),
        };
        assert!(done.success());
        assert_eq!(done.summary(), "Dark Mode is on.");
        let silent = Outcome::Done {
            success: true,
            summary: String::new(),
        };
        assert_eq!(silent.summary(), "Done.");
        assert!(Outcome::OutOfSteps(25).summary().contains("25 steps"));
        assert_eq!(Outcome::Stopped.summary(), "Stopped.");
    }

    #[test]
    fn starting_needs_a_token_control_and_a_picture_in_that_order() {
        let ok = ready_to_start(true, true, Some(SCREEN), false);
        assert_eq!(ok, Ok(SCREEN));
        assert!(ready_to_start(true, true, Some(SCREEN), true)
            .unwrap_err()
            .contains("already"));
        assert!(ready_to_start(false, false, None, false)
            .unwrap_err()
            .contains("token"));
        assert!(ready_to_start(true, false, None, false)
            .unwrap_err()
            .contains("control"));
        assert!(ready_to_start(true, true, None, false)
            .unwrap_err()
            .contains("see the screen"));
    }

    #[test]
    fn chat_entries_serialise_for_the_ui_and_are_bounded() {
        let mut status = AgentStatus::default();
        status.push(Entry::Step {
            n: 1,
            thought: "t".into(),
            action: "Tap (1, 2)".into(),
        });
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["entries"][0]["kind"], "step");
        assert_eq!(json["maxSteps"], 0);
        for i in 0..MAX_ENTRIES + 10 {
            status.push(Entry::Problem {
                text: i.to_string(),
            });
        }
        assert_eq!(status.entries.len(), MAX_ENTRIES);
    }

    #[test]
    fn a_trading_run_is_longer_and_may_wait_half_a_minute() {
        let limits = Limits::for_trading(120);
        assert_eq!(limits.max_steps, 120);
        assert_eq!(limits.max_errors_in_a_row, 3);
        let mut run = Run::new("trade", limits).unwrap();
        let wait = |ms: f64| StepResponse {
            action: Action::Wait { ms: Some(ms) },
            thought: String::new(),
        };
        assert_eq!(
            run.answered(&wait(20_000.0), SCREEN),
            Ok(Effect::WaitMs(20_000))
        );
        assert_eq!(
            run.answered(&wait(600_000.0), SCREEN),
            Ok(Effect::WaitMs(30_000))
        );
        assert_eq!(run.history()[0].content, "Wait 20.0s");
        assert_eq!(run.history()[1].content, "Wait 30.0s");
        // An ordinary run still stops a wait at five seconds.
        let mut run = Run::new("go", Limits::default()).unwrap();
        assert_eq!(
            run.answered(&wait(20_000.0), SCREEN),
            Ok(Effect::WaitMs(5000))
        );
        assert_eq!(run.history()[0].content, "Wait 5.0s");
    }

    #[test]
    fn a_refusal_by_the_rails_ends_the_run_with_its_reason() {
        let run = Run::new("trade", Limits::for_trading(120)).unwrap();
        let outcome = run.refused("Left Kalshi: this screen isn’t Kalshi");
        assert!(!outcome.success());
        assert_eq!(
            outcome.summary(),
            "Stopped by the trading rails: Left Kalshi: this screen isn’t Kalshi."
        );
    }

    #[test]
    fn long_text_is_shortened_on_a_character_boundary() {
        assert_eq!(shorten("héllo", 10), "héllo");
        assert_eq!(shorten("ééééé", 3), "ééé…");
    }
}
