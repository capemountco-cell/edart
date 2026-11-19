//! The MCP connector: lets Claude, Cursor and other MCP clients see and
//! control the iPhone through Edart.
//!
//! - [`server`]: the Model Context Protocol itself (JSON-RPC 2.0 in, JSON-RPC
//!   out), generic over a [`Phone`].
//! - [`tools`]: the tools a client can call, their schemas and validation.
//! - [`http`]: the Streamable HTTP transport, on loopback only.
//! - [`stdio`]: a bridge for clients that can only launch a local command.
//!
//! Nothing here knows about Tauri, Bluetooth or USB: the app supplies a
//! [`Phone`], the tests supply a fake one.

mod base64;
pub mod http;
pub mod server;
pub mod stdio;
pub mod tools;

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::gesture::{Gesture, Screen};

/// One picture of the iPhone's screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub jpeg: Vec<u8>,
    /// Size in pixels. Tool coordinates are in these units.
    pub width: u32,
    pub height: u32,
}

/// How gestures reach the phone right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlLink {
    Bluetooth,
    Usb,
    None,
}

/// What `get_status` reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhoneStatus {
    pub phone_name: Option<String>,
    pub control: ControlLink,
    /// Whether `screenshot` can work right now.
    pub mirror_available: bool,
}

/// The iPhone, as the connector sees it. The app implements this over
/// Bluetooth, USB and screen mirroring; tests implement it with a fake.
pub trait Phone: Send + Sync {
    fn status(&self) -> PhoneStatus;
    fn screenshot(&self) -> Result<Frame, String>;
    /// Perform a gesture and return when it is done. `screen` is the size
    /// the gesture's coordinates are measured in.
    fn perform(&self, gesture: &Gesture, screen: Screen) -> Result<(), String>;
}

/// One tool call, for the app's "recent calls" list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallRecord {
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
    pub tool: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// Most calls kept in a [`CallLog`].
pub const MAX_CALLS: usize = 50;

/// The last [`MAX_CALLS`] tool calls, oldest first.
#[derive(Debug, Default)]
pub struct CallLog(Mutex<VecDeque<CallRecord>>);

impl CallLog {
    pub fn record(&self, tool: &str, outcome: Result<(), String>) {
        let at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut calls = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if calls.len() == MAX_CALLS {
            calls.pop_front();
        }
        calls.push_back(CallRecord {
            at_ms,
            tool: tool.to_string(),
            ok: outcome.is_ok(),
            error: outcome.err(),
        });
    }

    pub fn snapshot(&self) -> Vec<CallRecord> {
        let calls = self.0.lock().unwrap_or_else(|e| e.into_inner());
        calls.iter().cloned().collect()
    }
}

/// A tool's name and one-line description, for the app's tool list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolInfo {
    pub name: &'static str,
    pub description: &'static str,
}

/// What the app's Connectors card shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    /// The user's choice.
    pub enabled: bool,
    /// Whether the server is actually accepting connections.
    pub listening: bool,
    pub port: Option<u16>,
    /// The address to give an MCP client, when listening.
    pub url: Option<String>,
    /// Why the server is not listening although it is enabled.
    pub error: Option<String>,
    /// This app's executable, for clients that launch `--mcp-stdio`.
    pub binary_path: Option<String>,
    pub tools: Vec<ToolInfo>,
    /// Recent tool calls, oldest first.
    pub calls: Vec<CallRecord>,
}

impl McpStatus {
    pub fn new(
        enabled: bool,
        port: Option<u16>,
        error: Option<String>,
        binary_path: Option<String>,
        calls: Vec<CallRecord>,
    ) -> Self {
        McpStatus {
            enabled,
            listening: port.is_some(),
            port,
            url: port.map(http::url),
            error,
            binary_path,
            tools: tools::infos(),
            calls,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_call_log_keeps_only_the_newest_calls() {
        let log = CallLog::default();
        for i in 0..MAX_CALLS + 5 {
            log.record(&format!("tool{i}"), Ok(()));
        }
        log.record("tap", Err("nope".into()));
        let calls = log.snapshot();
        assert_eq!(calls.len(), MAX_CALLS);
        assert_eq!(calls[0].tool, "tool6");
        let last = calls.last().unwrap();
        assert_eq!((last.tool.as_str(), last.ok), ("tap", false));
        assert_eq!(last.error.as_deref(), Some("nope"));
    }

    #[test]
    fn status_serialises_in_the_shape_the_ui_mirrors() {
        let status = McpStatus::new(true, Some(7391), None, Some("/x/edart".into()), vec![]);
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["listening"], true);
        assert_eq!(json["url"], "http://127.0.0.1:7391/mcp");
        assert_eq!(json["binaryPath"], "/x/edart");
        assert_eq!(json["tools"][0]["name"], "get_status");

        let off = McpStatus::new(false, None, None, None, vec![]);
        assert!(!off.listening);
        assert_eq!(off.url, None);
    }
}
