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
mod mirror;
#[cfg(not(target_os = "macos"))]
#[path = "mirror_stub.rs"]
mod mirror;

#[cfg(target_os = "macos")]
mod http;
#[cfg(not(target_os = "macos"))]
#[path = "http_stub.rs"]
mod http;

mod agent;

#[cfg(target_os = "macos")]
mod usb;
#[cfg(not(target_os = "macos"))]
#[path = "usb_stub.rs"]
mod usb;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use edart_core::agent::{AgentStatus, ConnectionTest};
use edart_core::gesture::{self, Gesture, Screen, Step, Tuning};
use edart_core::mirror::MirrorStatus;
use edart_core::settings::{self, Settings, SettingsChange, SettingsView};
use edart_core::status::StatusView;
use edart_core::usb::plan as usb_plan;
use edart_core::usb::readiness::{route, DeveloperModeOutcome, Route, UsbStatus};
use tauri::{AppHandle, Manager};

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
    tauri::async_runtime::spawn_blocking(move || perform_blocking(&app, &gesture, screen))
        .await
        .map_err(|e| e.to_string())?
}

/// True when a gesture has somewhere to go.
pub(crate) fn control_ready() -> bool {
    route(usb::is_ready(), ble::status().subscribed).is_ok()
}

/// [`perform`] for callers already off the main thread (the agent).
pub(crate) fn perform_blocking(
    app: &AppHandle,
    gesture: &Gesture,
    screen: Screen,
) -> Result<(), String> {
    match route(usb::is_ready(), ble::status().subscribed)? {
        Route::Usb => usb::perform(usb_plan::plan(gesture, screen).map_err(|e| e.to_string())?),
        Route::Bluetooth => run_steps(
            app,
            gesture::plan(gesture, screen, Tuning::default()).map_err(|e| e.to_string())?,
        ),
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

#[tauri::command]
fn mirror_status() -> MirrorStatus {
    mirror::status()
}

/// Start showing the iPhone's screen. Asks for Camera access the first
/// time: macOS lists a USB iPhone as a camera.
#[tauri::command]
fn mirror_start() {
    mirror::start();
}

/// Open System Settings › Privacy & Security › Camera, the only place a
/// refused Camera permission can be changed.
#[tauri::command]
fn open_camera_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Camera")
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Camera settings are a macOS feature.".into())
    }
}

/// Where the settings file lives: the app's own config folder.
fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join("settings.json"))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn settings_get(app: AppHandle) -> Result<SettingsView, String> {
    let path = settings_path(&app)?;
    Ok(settings::load(&path)?.view(&path))
}

/// Save a change to the settings. The token is never sent back.
#[tauri::command]
fn settings_set(app: AppHandle, change: SettingsChange) -> Result<SettingsView, String> {
    let path = settings_path(&app)?;
    // A damaged file is replaced rather than blocking every save.
    let mut current = settings::load(&path).unwrap_or_default();
    current.apply(change)?;
    settings::save(&path, &current).map_err(|e| format!("Couldn’t save the settings: {e}"))?;
    Ok(current.view(&path))
}

/// Check that the AI service answers and accepts the saved token.
#[tauri::command]
async fn settings_test(app: AppHandle) -> Result<ConnectionTest, String> {
    let current = settings::load(&settings_path(&app)?)?;
    tauri::async_runtime::spawn_blocking(move || test_connection(&current))
        .await
        .map_err(|e| e.to_string())
}

fn test_connection(current: &Settings) -> ConnectionTest {
    const TIMEOUT: Duration = Duration::from_secs(20);
    let health =
        http::request("GET", &current.health_url(), &[], None, TIMEOUT, None).map(|r| r.status);
    // An empty step request: the service checks the token before the body,
    // so this tells a good token from a bad one without running the model.
    let probe = match (&health, &current.token) {
        (Ok(200), Some(token)) => Some(
            http::request(
                "POST",
                &current.step_url(),
                &[
                    ("content-type", "application/json"),
                    ("x-edart-token", token),
                ],
                Some(b"{}"),
                TIMEOUT,
                None,
            )
            .map(|r| r.status),
        ),
        _ => None,
    };
    edart_core::agent::connection_test(health, probe)
}

#[tauri::command]
fn agent_status() -> AgentStatus {
    agent::status()
}

/// Start the agent on a goal. Fails with a sentence when the token, the
/// control link or the screen mirror is missing.
#[tauri::command]
fn agent_start(app: AppHandle, goal: String) -> Result<(), String> {
    let current = settings::load(&settings_path(&app)?)?;
    agent::start(app, &goal, current)
}

#[tauri::command]
fn agent_stop(app: AppHandle) {
    agent::stop(&app);
}

/// Forget the conversation (the "new chat" button).
#[tauri::command]
fn agent_clear(app: AppHandle) -> Result<(), String> {
    agent::clear(&app)
}

/// Answer `edart://localhost/frame.jpg` with the newest frame of the
/// mirror.
///
/// The live preview is a picture about ten times a second. Through IPC it
/// would be base64 inside JSON, copied and parsed on the page's main
/// thread; through a URL the webview loads the JPEG bytes as it loads any
/// image.
fn serve_frame(path: &str) -> tauri::http::Response<Vec<u8>> {
    let frame = if path == "/frame.jpg" {
        mirror::preview_jpeg()
    } else {
        None
    };
    let (status, kind, body) = match frame {
        Some(jpeg) => (200, "image/jpeg", jpeg),
        None => (404, "text/plain", b"no frame".to_vec()),
    };
    tauri::http::Response::builder()
        .status(status)
        .header("content-type", kind)
        .header("cache-control", "no-store")
        .body(body)
        .unwrap_or_default()
}

/// MCP screenshots come from the live screen mirror; tool coordinates are
/// in the pixels of the JPEG it returns.
struct MirrorScreenshots;

impl mcp::ScreenshotProvider for MirrorScreenshots {
    fn available(&self) -> bool {
        mirror::status().screen().is_some()
    }

    fn capture(&self) -> Result<mcp::Frame, String> {
        let (jpeg, screen) = mirror::screenshot().ok_or_else(|| {
            "The iPhone's screen isn't showing in Edart yet: connect it with a USB cable, \
             unlock it and allow Camera access in Edart's Permissions screen."
                .to_string()
        })?;
        Ok(mcp::Frame {
            jpeg,
            width: screen.width.round() as u32,
            height: screen.height.round() as u32,
        })
    }
}

/// Build and run the application.
pub fn run() {
    tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("edart", |_context, request, responder| {
            let path = request.uri().path().to_string();
            // Encoding a frame takes a few milliseconds; keep it off the
            // thread the webview calls on.
            std::thread::spawn(move || responder.respond(serve_frame(&path)));
        })
        .setup(|app| {
            // MCP connector, with screenshots from the screen mirror.
            mcp::set_screenshot_provider(MirrorScreenshots);
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
            // Same for the screen mirror and its Camera prompt.
            if mirror::already_allowed() {
                mirror::start();
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
            mirror_status,
            mirror_start,
            open_camera_settings,
            settings_get,
            settings_set,
            settings_test,
            agent_status,
            agent_start,
            agent_stop,
            agent_clear,
            // MCP connector.
            mcp::mcp_status,
            mcp::mcp_set_enabled
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the application");
}
