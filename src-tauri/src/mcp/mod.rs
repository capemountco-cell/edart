//! The MCP connector's place in the app: start and stop the local server,
//! hand it the real iPhone, and report its state to the window.
//!
//! The protocol, the tools and the HTTP transport are in
//! `edart_core::mcp`, where they are tested. What is here cannot be: it
//! is the wiring to Bluetooth, USB and (once it exists) screen mirroring.
//!
//! To plug in screen mirroring, call [`set_screenshot_provider`] once the
//! mirror can produce a JPEG of the iPhone's screen.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use edart_core::gesture::{self, Gesture, Screen, Tuning};
use edart_core::mcp::http::{HttpServer, DEFAULT_PORT, LAST_PORT};
use edart_core::mcp::server::Server;
use edart_core::mcp::{CallLog, ControlLink, McpStatus, Phone, PhoneStatus};
use edart_core::usb::plan as usb_plan;
use edart_core::usb::readiness::{route, Route};
use tauri::{AppHandle, Manager};

pub use edart_core::mcp::Frame;

/// Where `screenshot` gets its picture. Screen mirroring implements this
/// (a closure returning a [`Frame`] is enough) and registers it with
/// [`set_screenshot_provider`].
pub trait ScreenshotProvider: Send + Sync {
    /// Whether a picture can be taken right now. Shown by `get_status`.
    fn available(&self) -> bool {
        true
    }
    /// The iPhone's screen, now, as a JPEG with its size in pixels. Tool
    /// coordinates are in those pixels.
    fn capture(&self) -> Result<Frame, String>;
}

impl<F> ScreenshotProvider for F
where
    F: Fn() -> Result<Frame, String> + Send + Sync,
{
    fn capture(&self) -> Result<Frame, String> {
        self()
    }
}

static PROVIDER: RwLock<Option<Box<dyn ScreenshotProvider>>> = RwLock::new(None);

const NO_MIRROR: &str = "Screen mirroring isn't available yet: connect the iPhone with a USB \
                         cable and open Edart.";

/// Register what takes screenshots. Replaces any earlier provider.
// Nothing calls this until screen mirroring lands.
#[allow(dead_code)]
pub fn set_screenshot_provider(provider: impl ScreenshotProvider + 'static) {
    *PROVIDER.write().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(provider));
}

/// Perform a gesture on the iPhone and return when it is done: over USB
/// when that is ready, else over Bluetooth. The same route, plans and
/// gesture lock as the `perform` command, for callers that are not async.
pub fn perform_blocking(app: &AppHandle, gesture: &Gesture, screen: Screen) -> Result<(), String> {
    match route(crate::usb::is_ready(), crate::ble::status().subscribed)? {
        Route::Usb => {
            let steps = usb_plan::plan(gesture, screen).map_err(|e| e.to_string())?;
            crate::usb::perform(steps)
        }
        Route::Bluetooth => {
            let steps =
                gesture::plan(gesture, screen, Tuning::default()).map_err(|e| e.to_string())?;
            crate::run_steps(app, steps)
        }
    }
}

/// The real iPhone, as the connector sees it.
struct AppPhone {
    app: AppHandle,
}

impl Phone for AppPhone {
    fn status(&self) -> PhoneStatus {
        let control = match route(crate::usb::is_ready(), crate::ble::status().subscribed) {
            Ok(Route::Usb) => ControlLink::Usb,
            Ok(Route::Bluetooth) => ControlLink::Bluetooth,
            Err(_) => ControlLink::None,
        };
        let provider = PROVIDER.read().unwrap_or_else(|e| e.into_inner());
        PhoneStatus {
            phone_name: crate::usb::status().device_name,
            control,
            mirror_available: provider.as_ref().is_some_and(|p| p.available()),
        }
    }

    fn screenshot(&self) -> Result<Frame, String> {
        let provider = PROVIDER.read().unwrap_or_else(|e| e.into_inner());
        match provider.as_ref() {
            Some(provider) => provider.capture(),
            None => Err(NO_MIRROR.into()),
        }
    }

    fn perform(&self, gesture: &Gesture, screen: Screen) -> Result<(), String> {
        perform_blocking(&self.app, gesture, screen)
    }
}

struct State {
    app: Option<AppHandle>,
    enabled: bool,
    server: Option<HttpServer>,
    error: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    app: None,
    enabled: true,
    server: None,
    error: None,
});

fn log() -> Arc<CallLog> {
    static LOG: OnceLock<Arc<CallLog>> = OnceLock::new();
    LOG.get_or_init(Arc::default).clone()
}

/// The file whose presence means "the user turned the connector off".
/// On by default, so there is nothing to write until they do.
fn off_marker(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("mcp-off"))
}

/// Make the server match `enabled`.
fn apply(state: &mut State, enabled: bool) {
    state.enabled = enabled;
    state.error = None;
    if !enabled {
        // Dropping the server stops it.
        state.server = None;
        return;
    }
    if state.server.is_some() {
        return;
    }
    let Some(app) = state.app.clone() else {
        return;
    };
    let handler = Arc::new(Server::new(AppPhone { app }, log()));
    match HttpServer::start(handler, DEFAULT_PORT..=LAST_PORT) {
        Ok(server) => state.server = Some(server),
        Err(e) => {
            state.error = Some(format!(
                "Could not listen on ports {DEFAULT_PORT} to {LAST_PORT}: {e}"
            ));
        }
    }
}

fn view(state: &State) -> McpStatus {
    McpStatus::new(
        state.enabled,
        state.server.as_ref().map(HttpServer::port),
        state.error.clone(),
        std::env::current_exe()
            .ok()
            .map(|path| path.to_string_lossy().into_owned()),
        log().snapshot(),
    )
}

/// Start the connector with the app, unless the user turned it off.
pub fn start(app: &AppHandle) {
    let enabled = !off_marker(app).is_some_and(|marker| marker.exists());
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.app = Some(app.clone());
    apply(&mut state, enabled);
}

#[tauri::command]
pub fn mcp_status() -> McpStatus {
    view(&STATE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Turn the connector on or off, and remember the choice.
#[tauri::command]
pub fn mcp_set_enabled(app: AppHandle, enabled: bool) -> McpStatus {
    if let Some(marker) = off_marker(&app) {
        // If the choice cannot be saved it still holds until the app quits.
        if enabled {
            let _ = std::fs::remove_file(&marker);
        } else if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&marker, b""));
        }
    }
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.app = Some(app);
    apply(&mut state, enabled);
    view(&state)
}
