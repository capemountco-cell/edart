//! The stdio bridge: `edart --mcp-stdio`.
//!
//! Some MCP clients can only launch a local command and talk to it over
//! stdin and stdout. This is that command. It owns no phone: it forwards
//! each line to the running app's HTTP endpoint and prints the answer.
//!
//! When the app is not open, the bridge still answers `initialize` and
//! `tools/list` itself, so the client keeps the connection and its tool
//! list, and every tool call says "open Edart" until the app appears.

use std::io::{self, BufRead, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::http::{DEFAULT_PORT, LAST_PORT, PATH, SERVER_HEADER};
use super::server::{error_response, Server};
use super::{CallLog, ControlLink, Frame, Phone, PhoneStatus};
use crate::gesture::{Gesture, Screen};

pub const NOT_RUNNING: &str = "Edart isn't open. Open the Edart app on this Mac, then try again.";

const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
/// Longer than the slowest tool call (a 10 s press) by a wide margin.
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);

/// The answer to one forwarded message.
#[derive(Debug, PartialEq, Eq)]
pub enum Posted {
    /// The app replied with this JSON.
    Reply(String),
    /// The app took the message and has nothing to say (a notification).
    Accepted,
    /// The app refused the request, with this HTTP status.
    Refused(u16),
}

/// POST `body` to Edart on `port`. `Err` when nothing answers there, or
/// when what answers is not Edart.
pub fn post(port: u16, body: &str) -> io::Result<Posted> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
    stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
    write!(
        stream,
        "POST {PATH} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;

    let not_edart = || io::Error::new(io::ErrorKind::InvalidData, "not Edart");
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(not_edart)?;
    let head = std::str::from_utf8(&response[..split]).map_err(|_| not_edart())?;
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(not_edart)?;
    let is_edart = lines.any(|l| {
        l.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("server") && value.trim() == SERVER_HEADER
        })
    });
    if !is_edart {
        return Err(not_edart());
    }
    Ok(match status {
        200 => {
            let reply =
                String::from_utf8(response[split + 4..].to_vec()).map_err(|_| not_edart())?;
            Posted::Reply(reply)
        }
        202 => Posted::Accepted,
        other => Posted::Refused(other),
    })
}

/// Stands in for the phone while the app is closed. Never reached: tool
/// calls are answered before they get this far.
struct Closed;

impl Phone for Closed {
    fn status(&self) -> PhoneStatus {
        PhoneStatus {
            phone_name: None,
            control: ControlLink::None,
            mirror_available: false,
        }
    }
    fn screenshot(&self) -> Result<Frame, String> {
        Err(NOT_RUNNING.into())
    }
    fn perform(&self, _gesture: &Gesture, _screen: Screen) -> Result<(), String> {
        Err(NOT_RUNNING.into())
    }
}

/// Forwards messages to the app, finding it again if it restarts.
pub struct Bridge {
    ports: RangeInclusive<u16>,
    /// Where Edart answered last.
    port: Option<u16>,
    offline: Server<Closed>,
}

impl Bridge {
    pub fn new(ports: RangeInclusive<u16>) -> Self {
        Bridge {
            ports,
            port: None,
            offline: Server::new(Closed, Arc::new(CallLog::default())),
        }
    }

    fn forward(&mut self, message: &str) -> Option<Posted> {
        let known = self.port.take();
        for port in known
            .into_iter()
            .chain(self.ports.clone().filter(|p| Some(*p) != known))
        {
            if let Ok(posted) = post(port, message) {
                self.port = Some(port);
                return Some(posted);
            }
        }
        None
    }

    /// Answer one line from the client. `None` when there is nothing to
    /// print.
    pub fn answer(&mut self, message: &str) -> Option<String> {
        match self.forward(message) {
            Some(Posted::Reply(reply)) => Some(one_line(reply)),
            Some(Posted::Accepted) => None,
            Some(Posted::Refused(status)) => request_id(message).map(|id| {
                let why = format!("Edart refused the request (HTTP {status}).");
                error_response(id, -32000, &why).to_string()
            }),
            None => self.answer_offline(message),
        }
    }

    fn answer_offline(&self, message: &str) -> Option<String> {
        let parsed: Option<Value> = serde_json::from_str(message).ok();
        let is_call = parsed
            .as_ref()
            .and_then(|m| m.get("method"))
            .and_then(Value::as_str)
            == Some("tools/call");
        match request_id(message) {
            Some(id) if is_call => Some(
                json!({ "jsonrpc": "2.0", "id": id, "result": {
                    "content": [{ "type": "text", "text": NOT_RUNNING }],
                    "isError": true,
                } })
                .to_string(),
            ),
            _ => self.offline.handle(message.as_bytes()),
        }
    }
}

/// The id of a request, if `message` is one.
fn request_id(message: &str) -> Option<Value> {
    let parsed: Value = serde_json::from_str(message).ok()?;
    let id = parsed.get("id")?;
    (id.is_string() || id.is_number()).then(|| id.clone())
}

/// The stdio transport is one message per line.
fn one_line(reply: String) -> String {
    if !reply.contains(['\n', '\r']) {
        return reply;
    }
    match serde_json::from_str::<Value>(&reply) {
        Ok(value) => value.to_string(),
        Err(_) => reply.replace(['\n', '\r'], " "),
    }
}

/// Read messages from `input`, one per line, until it closes.
pub fn run(
    input: impl BufRead,
    mut output: impl Write,
    ports: RangeInclusive<u16>,
) -> io::Result<()> {
    let mut bridge = Bridge::new(ports);
    for line in input.lines() {
        let line = line?;
        let message = line.trim();
        if message.is_empty() {
            continue;
        }
        if let Some(reply) = bridge.answer(message) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// The whole of `edart --mcp-stdio`: bridge this process's stdin and
/// stdout to the running app.
pub fn run_process() {
    let _ = run(
        io::stdin().lock(),
        io::stdout().lock(),
        DEFAULT_PORT..=LAST_PORT,
    );
}
