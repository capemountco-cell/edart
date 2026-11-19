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
    self, AgentStatus, Effect, Entry, Limits, Outcome, Run, StepFailure, RETRY_MS, SETTLE_MS,
};
use edart_core::base64;
use edart_core::settings::Settings;
use tauri::{AppHandle, Emitter};

use crate::{http, mirror};

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
    let limits = Limits::default();
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
        let run = Run::new(goal, limits)?;
        STOP.store(false, Ordering::SeqCst);
        status.running = true;
        status.stopping = false;
        status.step = 0;
        status.max_steps = limits.max_steps;
        status.push(Entry::Goal {
            text: run.goal().to_string(),
        });
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

fn ask(
    run: &Run,
    settings: &Settings,
) -> Result<(agent::StepResponse, edart_core::gesture::Screen), StepFailure> {
    let (jpeg, screen) = mirror::screenshot().ok_or_else(|| {
        StepFailure::Network(
            "Edart can’t see the iPhone’s screen. Check the cable and unlock the iPhone".into(),
        )
    })?;
    let screenshot = base64::encode(&jpeg);
    let body = serde_json::to_vec(&run.request(&screenshot, screen))
        .map_err(|e| StepFailure::Rejected(e.to_string()))?;
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
    agent::parse_step(response.status, &response.body).map(|step| (step, screen))
}

fn work(app: &AppHandle, mut run: Run, settings: &Settings) -> Outcome {
    loop {
        if let Some(outcome) = run.check(STOP.load(Ordering::SeqCst)) {
            return outcome;
        }
        let (response, screen) = match ask(&run, settings) {
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

        let effect = run.answered(&response, screen);
        let step = run.steps();
        if !matches!(effect, Ok(Effect::Finish { .. })) {
            update(app, |s| {
                s.step = step;
                s.push(Entry::Step {
                    n: step,
                    thought: response.thought.trim().to_string(),
                    action: response.action.describe(),
                });
            });
        }
        let result = match effect {
            Ok(Effect::Finish { success, summary }) => {
                return Outcome::Done { success, summary };
            }
            Ok(Effect::WaitMs(ms)) => {
                pause(ms);
                Ok(())
            }
            Ok(Effect::Perform(gesture)) => {
                let done = crate::perform_blocking(app, &gesture, screen);
                if done.is_ok() {
                    pause(SETTLE_MS);
                }
                done
            }
            Err(why) => Err(why),
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
