//! The agent loop: screenshot, ask the API, act, repeat.
//!
//! It runs on its own thread in Rust, not in the webview, so it keeps
//! going when the window is hidden and its token never reaches the page.
//! Every decision (what to send, what an answer means, when to stop) is
//! `edart_core::agent`; this file does the waiting and the I/O.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use edart_core::agent::{
    self, AgentStatus, Context, Effect, Entry, Limits, Outcome, Run, StepFailure, RETRY_MS,
    SETTLE_MS,
};
use edart_core::base64;
use edart_core::gesture::{Gesture, Screen};
use edart_core::ground::{self, Element, Hands};
use edart_core::guard::{self, GateError, OcrFrame};
use edart_core::jpeg;
use edart_core::settings::Settings;
use tauri::{AppHandle, Emitter};

use crate::trading::{self, Source};
use crate::{http, mirror, ocr};

/// The UI listens for this and reads [`status`] again.
pub const CHANGED_EVENT: &str = "agent-changed";
/// The model can take a while on a large screenshot.
const STEP_TIMEOUT: Duration = Duration::from_secs(90);

static STATUS: Mutex<Option<AgentStatus>> = Mutex::new(None);
static STOP: AtomicBool = AtomicBool::new(false);

fn update(app: &AppHandle, f: impl FnOnce(&mut AgentStatus)) {
    {
        let mut guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
        f(guard.get_or_insert_with(AgentStatus::default));
    }
    let _ = app.emit(CHANGED_EVENT, ());
}

pub fn status() -> AgentStatus {
    STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// Start working towards `goal`. Fails with a sentence for the user when
/// something the agent needs is missing.
pub fn start(app: AppHandle, goal: &str, settings: Settings) -> Result<(), String> {
    // In trading mode the run is longer and may wait longer. The
    // playbook and the limits go with every step as instructions.
    let trading = settings.trading.clone().sanitized();
    let (limits, sent_goal, shown_goal) = if trading.enabled {
        (
            Limits::for_trading(trading.max_steps),
            trading.goal(goal),
            trading.goal_label(goal),
        )
    } else {
        (Limits::default(), goal.to_string(), goal.trim().to_string())
    };
    let run = {
        // Checked and marked running under one lock, so two starts cannot
        // both get through.
        let mut guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
        let status = guard.get_or_insert_with(AgentStatus::default);
        agent::ready_to_start(
            settings.token.is_some(),
            crate::control_ready(),
            mirror::status().screen(),
            status.running,
        )?;
        // What was asked earlier in this chat, and how it went, is
        // carried into the new run.
        let run = Run::new(&sent_goal, limits)?.with_context(Context {
            instructions: trading.instructions(settings.instructions.as_deref()),
            earlier: agent::exchanges(&status.entries),
            allow_typing: settings.allow_typing,
        });
        STOP.store(false, Ordering::SeqCst);
        status.running = true;
        status.stopping = false;
        status.step = 0;
        status.max_steps = limits.max_steps;
        status.push(Entry::Goal { text: shown_goal });
        run
    };
    let _ = app.emit(CHANGED_EVENT, ());

    let worker = app.clone();
    let spawned = std::thread::Builder::new()
        .name("edart-agent".into())
        .spawn(move || {
            let outcome = work(&worker, run, &settings);
            update(&worker, |s| {
                s.running = false;
                s.stopping = false;
                s.push(Entry::Finished {
                    success: outcome.success(),
                    text: outcome.summary(),
                });
            });
        });
    if let Err(e) = spawned {
        update(&app, |s| {
            s.running = false;
            s.push(Entry::Finished {
                success: false,
                text: format!("Couldn’t start the agent: {e}"),
            });
        });
    }
    Ok(())
}

/// Ask the agent to stop. It finishes the gesture in hand first.
pub fn stop(app: &AppHandle) {
    STOP.store(true, Ordering::SeqCst);
    update(app, |s| s.stopping = s.running);
}

/// True once Stop has been pressed and until the next run starts.
pub fn stop_requested() -> bool {
    STOP.load(Ordering::SeqCst)
}

/// Forget the conversation. Refused while the agent is working.
pub fn clear(app: &AppHandle) -> Result<(), String> {
    if status().running {
        return Err("Stop the agent first.".into());
    }
    update(app, |s| s.entries.clear());
    Ok(())
}

/// Sleep, waking early when Stop is pressed.
fn pause(ms: u64) {
    let mut left = ms;
    while left > 0 && !STOP.load(Ordering::Relaxed) {
        let nap = left.min(50);
        std::thread::sleep(Duration::from_millis(nap));
        left -= nap;
    }
}

/// The quality of the screenshot the model is sent.
const SCREENSHOT_QUALITY: u8 = 82;

/// What a step was asked about: the screen's size and the text found on
/// it, numbered.
struct Seen {
    screen: Screen,
    elements: Vec<Element>,
}

fn ask(run: &Run, settings: &Settings) -> Result<(agent::StepResponse, Seen), StepFailure> {
    let (frame, seq, _age) = mirror::frame().ok_or_else(|| {
        StepFailure::Network(
            "Edart can’t see the iPhone’s screen. Check the cable and unlock the iPhone".into(),
        )
    })?;
    let screen = Screen {
        width: frame.width as f64,
        height: frame.height as f64,
    };
    let jpeg = jpeg::encode(&frame, SCREENSHOT_QUALITY)
        .ok_or_else(|| StepFailure::Rejected("the screenshot couldn’t be encoded".into()))?;
    // The text of the very frame that is sent, so "element 12" means
    // the same to the model and to Edart. Without it the model still
    // has the picture.
    let elements = ocr::read_frame(&frame, seq)
        .map(|text| ground::elements(&text, screen))
        .unwrap_or_default();
    let screenshot = base64::encode(&jpeg);
    let request = run.request(
        &screenshot,
        screen,
        (!elements.is_empty()).then_some(elements.as_slice()),
    );
    let body = serde_json::to_vec(&request).map_err(|e| StepFailure::Rejected(e.to_string()))?;
    let token = settings.token.as_deref().unwrap_or_default();
    let response = http::request(
        "POST",
        &settings.step_url(),
        &[
            ("content-type", "application/json"),
            ("x-edart-token", token),
        ],
        Some(&body),
        STEP_TIMEOUT,
        Some(&STOP),
    )
    .map_err(StepFailure::Network)?;
    agent::parse_step(response.status, &response.body).map(|step| (step, Seen { screen, elements }))
}

/// The phone, for `open_app`: every gesture it makes goes through the
/// trading rails like any other.
struct AgentHands<'a> {
    app: &'a AppHandle,
    /// Set when the rails refused one of its gestures.
    refused: Option<String>,
}

impl Hands for AgentHands<'_> {
    fn perform(&mut self, gesture: &Gesture, screen: Screen) -> Result<(), String> {
        trading::perform(self.app, gesture, screen, Source::Agent).map_err(|e| {
            if let GateError::Blocked(reason) = &e {
                self.refused = Some(reason.clone());
            }
            e.message()
        })
    }

    fn read_screen(&mut self) -> Result<OcrFrame, String> {
        ocr::read()
    }

    fn wait_ms(&mut self, ms: u64) {
        pause(ms);
    }
}

/// How a step's action went.
enum Acted {
    Done(Result<(), String>),
    /// The trading rails said no.
    Refused(String),
}

fn perform(app: &AppHandle, gesture: &Gesture, screen: Screen) -> Acted {
    match trading::perform(app, gesture, screen, Source::Agent) {
        Ok(()) => {
            pause(SETTLE_MS);
            Acted::Done(Ok(()))
        }
        Err(GateError::Blocked(reason)) => Acted::Refused(reason),
        Err(GateError::Failed(why)) => Acted::Done(Err(why)),
    }
}

fn open_app(app: &AppHandle, name: &str, screen: Screen) -> Acted {
    // With trading mode on only the allowed app may be opened, and only
    // from the Home Screen page it is on: searching would mean swiping
    // and typing outside it.
    let limits = match trading::limits(app) {
        Ok(limits) => limits,
        Err(why) => return Acted::Refused(why),
    };
    if let Err(why) = guard::may_open(&limits, name) {
        return Acted::Refused(why);
    }
    let mut hands = AgentHands { app, refused: None };
    let opened = ground::open_app(&mut hands, name, screen, !limits.enabled);
    match (hands.refused, opened) {
        (Some(reason), _) => Acted::Refused(reason),
        (None, Ok(())) => {
            pause(SETTLE_MS);
            Acted::Done(Ok(()))
        }
        (None, Err(why)) => Acted::Done(Err(why)),
    }
}

fn work(app: &AppHandle, mut run: Run, settings: &Settings) -> Outcome {
    loop {
        if let Some(outcome) = run.check(STOP.load(Ordering::SeqCst)) {
            return outcome;
        }
        // The instructions follow the settings as they are now, so a
        // rule changed during a run is the rule from the next step on.
        if let Ok(current) =
            crate::settings_path(app).and_then(|path| edart_core::settings::load(&path))
        {
            let trading = current.trading.clone().sanitized();
            run.set_instructions(trading.instructions(current.instructions.as_deref()));
        }
        let (response, seen) = match ask(&run, settings) {
            Ok(answer) => answer,
            Err(failure) => {
                if STOP.load(Ordering::SeqCst) {
                    return Outcome::Stopped;
                }
                if let Some(outcome) = run.request_failed(&failure) {
                    return outcome;
                }
                update(app, |s| {
                    s.push(Entry::Problem {
                        text: format!("{} Trying again.", failure.message()),
                    })
                });
                pause(RETRY_MS);
                continue;
            }
        };

        let screen = seen.screen;
        let effect = run.answered_with(&response, screen, &seen.elements);
        let step = run.steps();
        if !matches!(effect, Ok(Effect::Finish { .. })) {
            update(app, |s| {
                s.step = step;
                s.push(Entry::Step {
                    n: step,
                    thought: response.thought.trim().to_string(),
                    action: response
                        .action
                        .describe_within(run.max_wait_ms(), &seen.elements),
                });
            });
        }
        let result = match effect {
            Ok(Effect::Finish { success, summary }) => {
                return Outcome::Done { success, summary };
            }
            Ok(Effect::WaitMs(ms)) => {
                pause(ms);
                Acted::Done(Ok(()))
            }
            Ok(Effect::Perform(gesture)) => perform(app, &gesture, screen),
            Ok(Effect::OpenApp(name)) => open_app(app, &name, screen),
            Err(why) => Acted::Done(Err(why)),
        };
        let result = match result {
            Acted::Done(result) => result,
            // The rails said no: the run is over.
            Acted::Refused(reason) => {
                if STOP.load(Ordering::SeqCst) {
                    return Outcome::Stopped;
                }
                return run.refused(&reason);
            }
        };
        if let Err(why) = &result {
            let text = format!("That didn’t work: {why}.");
            update(app, |s| s.push(Entry::Problem { text }));
        }
        if let Some(outcome) = run.acted(result) {
            return outcome;
        }
    }
}
