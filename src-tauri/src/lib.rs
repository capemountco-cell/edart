//! The Tauri shell.
//!
//! No logic lives here. A command deserialises its arguments, calls one
//! function in the core crate, and returns the result. A decision made in a
//! command handler is a decision that cannot be tested on Linux.

#[cfg(target_os = "macos")]
mod ble;
#[cfg(not(target_os = "macos"))]
#[path = "ble_stub.rs"]
mod ble;

use std::sync::Mutex;

use edart_core::gesture::{self, Gesture, Screen, Step, Tuning};
use edart_core::status::StatusView;
use tauri::AppHandle;

/// One gesture at a time: two interleaved taps would be nonsense.
static GESTURE_LOCK: Mutex<()> = Mutex::new(());

#[tauri::command]
fn bt_status() -> StatusView {
    ble::status().into()
}

/// Start the Bluetooth keyboard role. Asks for Bluetooth permission the
/// first time.
#[tauri::command]
fn bt_start(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        app.run_on_main_thread(|| {
            if let Some(mtm) = objc2::MainThreadMarker::new() {
                ble::start(mtm);
            }
        })
        .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Bluetooth control needs macOS.".into())
    }
}

/// Perform a gesture on the iPhone. Returns when the last report is sent.
#[tauri::command]
async fn perform(app: AppHandle, gesture: Gesture, screen: Screen) -> Result<(), String> {
    if !ble::status().subscribed {
        return Err("No iPhone is connected over Bluetooth yet.".into());
    }
    let steps = gesture::plan(&gesture, screen, Tuning::default()).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || run_steps(&app, steps))
        .await
        .map_err(|e| e.to_string())?
}

fn run_steps(app: &AppHandle, steps: Vec<Step>) -> Result<(), String> {
    let _guard = GESTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for step in steps {
        match step {
            Step::Send(report) => app
                .run_on_main_thread(move || {
                    ble::send(report.id, &report.payload);
                })
                .map_err(|e| e.to_string())?,
            Step::WaitMs(ms) => std::thread::sleep(std::time::Duration::from_millis(ms)),
        }
    }
    Ok(())
}

/// Build and run the application.
pub fn run() {
    tauri::Builder::default()
        .setup(|_app| {
            // Skip the prompt on first launch; the Setup screen asks with
            // context. Once allowed, start straight away.
            #[cfg(target_os = "macos")]
            if ble::already_allowed() {
                if let Some(mtm) = objc2::MainThreadMarker::new() {
                    ble::start(mtm);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![bt_status, bt_start, perform])
        .run(tauri::generate_context!())
        .expect("failed to start the application");
}
