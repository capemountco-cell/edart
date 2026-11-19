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
#[cfg(target_os = "macos")]
mod ble_central;

// MCP connector: the local server AI tools use to reach the iPhone.
mod mcp;

#[cfg(target_os = "macos")]
mod usb;
#[cfg(not(target_os = "macos"))]
#[path = "usb_stub.rs"]
mod usb;

use std::sync::Mutex;

use edart_core::gesture::{self, Gesture, Screen, Step, Tuning};
use edart_core::status::StatusView;
use edart_core::usb::plan as usb_plan;
use edart_core::usb::readiness::{route, DeveloperModeOutcome, Route, UsbStatus};
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
                ble_central::start(mtm);
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

/// Publish the Bluetooth services again, so a phone with a stale copy
/// rediscovers them.
#[tauri::command]
fn bt_restart(app: AppHandle) -> Result<(), String> {
    app.run_on_main_thread(ble::restart)
        .map_err(|e| e.to_string())
}

/// Connect to iPhones already linked to this Mac, from the Mac side.
#[tauri::command]
fn bt_reach_out(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        app.run_on_main_thread(ble_central::reach_out)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        ble::reach_out();
        Err("Bluetooth control needs macOS.".into())
    }
}

/// Open System Settings › Bluetooth. macOS refuses a phone's pairing
/// request when nothing is there to handle it; with this pane open, it asks.
#[tauri::command]
fn open_bluetooth_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.BluetoothSettings")
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Bluetooth settings are a macOS feature.".into())
    }
}

#[tauri::command]
fn usb_status() -> UsbStatus {
    usb::status()
}

/// Ask the plugged-in iPhone to turn Developer Mode on.
#[tauri::command]
async fn usb_enable_developer_mode() -> Result<DeveloperModeOutcome, String> {
    tauri::async_runtime::spawn_blocking(usb::enable_developer_mode)
        .await
        .map_err(|e| e.to_string())?
}

/// Connect to the iPhone's developer services (touch and keyboard).
#[tauri::command]
async fn usb_prepare() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(usb::prepare)
        .await
        .map_err(|e| e.to_string())?
}

/// Perform a gesture on the iPhone: over USB when that is ready, else over
/// Bluetooth. Returns when the last report is sent.
#[tauri::command]
async fn perform(app: AppHandle, gesture: Gesture, screen: Screen) -> Result<(), String> {
    match route(usb::is_ready(), ble::status().subscribed)? {
        Route::Usb => {
            let steps = usb_plan::plan(&gesture, screen).map_err(|e| e.to_string())?;
            tauri::async_runtime::spawn_blocking(move || usb::perform(steps))
                .await
                .map_err(|e| e.to_string())?
        }
        Route::Bluetooth => {
            let steps =
                gesture::plan(&gesture, screen, Tuning::default()).map_err(|e| e.to_string())?;
            tauri::async_runtime::spawn_blocking(move || run_steps(&app, steps))
                .await
                .map_err(|e| e.to_string())?
        }
    }
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
        .setup(|app| {
            // MCP connector.
            mcp::start(app.handle());
            // Watching the cable asks nothing of the user, so it starts
            // with the app.
            usb::start();
            // Skip the prompt on first launch; the Setup screen asks with
            // context. Once allowed, start straight away.
            #[cfg(target_os = "macos")]
            if ble::already_allowed() {
                if let Some(mtm) = objc2::MainThreadMarker::new() {
                    ble::start(mtm);
                    ble_central::start(mtm);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bt_status,
            bt_start,
            bt_restart,
            bt_reach_out,
            open_bluetooth_settings,
            usb_status,
            usb_enable_developer_mode,
            usb_prepare,
            perform,
            // MCP connector.
            mcp::mcp_status,
            mcp::mcp_set_enabled
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the application");
}
