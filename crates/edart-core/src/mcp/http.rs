//! The Streamable HTTP transport: `POST /mcp` with one JSON-RPC message,
//! answered with `application/json`. Loopback only.
//!
//! Hand-written on `std::net` rather than a library: the server needs one
//! route, one method and no keep-alive, which is less code than adapting an
//! HTTP crate, and it keeps the app's dependency list (and what has to be
//! trusted to parse bytes from other local programs) short.
//!
//! What keeps other things out:
//! - it binds 127.0.0.1, so nothing off this Mac can connect;
//! - a web page in the user's browser *can* reach 127.0.0.1, so requests
//!   carrying an `Origin` that is not this machine are refused, as are
//!   requests whose `Host` is not this machine (DNS rebinding), and the
//!   body must be `application/json`, which a page cannot send across
//!   origins without a preflight this server never grants;
//! - request size, header size, time and concurrent connections are capped.

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::server::{error_response, Server, INVALID_REQUEST, PROTOCOL_VERSIONS};
use super::Phone;

/// The port Edart listens on, and the last one it tries when that and the
/// ones after it are taken.
pub const DEFAULT_PORT: u16 = 7391;
pub const LAST_PORT: u16 = 7399;
pub const PATH: &str = "/mcp";
/// Largest request body accepted.
pub const MAX_BODY: usize = 1024 * 1024;
/// Sent as the `Server` header, so the stdio bridge can tell Edart from
/// another program that happens to hold the port.
pub const SERVER_HEADER: &str = "edart-mcp";

const MAX_HEAD: usize = 16 * 1024;
const MAX_CONNECTIONS: usize = 16;
/// A request must arrive in full within this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// The address to give an MCP client.
pub fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}{PATH}")
}

/// What answers a request body. [`Server`] in the app, anything in tests.
pub trait Handler: Send + Sync {
    /// `None` when the message needs no reply.
    fn handle(&self, body: &[u8]) -> Option<String>;
}

impl<P: Phone> Handler for Server<P> {
    fn handle(&self, body: &[u8]) -> Option<String> {
        Server::handle(self, body)
    }
}

/// A running server. Stops when dropped.
pub struct HttpServer {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HttpServer {
    /// Listen on the first free port of `ports` (port 0 picks any free one)
    /// and serve on a thread of its own.
    pub fn start(handler: Arc<dyn Handler>, ports: RangeInclusive<u16>) -> io::Result<Self> {
        let mut last_error = io::Error::new(io::ErrorKind::InvalidInput, "no port to try");
        for port in ports {
            match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
                Ok(listener) => return Self::serve(listener, handler),
                Err(e) => last_error = e,
            }
        }
        Err(last_error)
    }

    fn serve(listener: TcpListener, handler: Arc<dyn Handler>) -> io::Result<Self> {
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = thread::Builder::new()
            .name("mcp-http".into())
            .spawn(move || accept_loop(listener, handler, stopped))?;
        Ok(HttpServer {
            port,
            stop,
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Stop listening. Requests already being answered finish.
    pub fn stop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.stop.store(true, Ordering::SeqCst);
        // The accept loop is blocked waiting for a connection: give it one.
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
        let _ = TcpStream::connect_timeout(&addr, Duration::from_secs(1));
        let _ = thread.join();
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(listener: TcpListener, handler: Arc<dyn Handler>, stop: Arc<AtomicBool>) {
    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else {
            // Out of file descriptors, most likely. Do not spin.
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        // Bound to loopback, so this cannot fail; checked all the same.
        if !stream.peer_addr().is_ok_and(|a| a.ip().is_loopback()) {
            continue;
        }
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::SeqCst);
            finish(&stream, &Reply::refuse(503, "Edart is busy. Try again."));
            continue;
        }
        let slot = Slot(active.clone());
        let handler = handler.clone();
        let spawned = thread::Builder::new()
            .name("mcp-request".into())
            .spawn(move || {
                let _slot = slot;
                let reply = exchange(&stream, handler.as_ref());
                finish(&stream, &reply);
            });
        drop(spawned);
    }
}

/// Frees a connection slot when its thread ends, however it ends.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Reply {
    status: u16,
    body: Option<String>,
}

impl Reply {
    /// An HTTP-level refusal. The body is a JSON-RPC error so a client
    /// that shows it shows something readable.
    fn refuse(status: u16, message: &str) -> Self {
        Reply {
            status,
            body: Some(
                error_response(serde_json::Value::Null, INVALID_REQUEST, message).to_string(),
            ),
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        _ => "Service Unavailable",
    }
}

/// Send the reply and close. One request per connection: simple, and a
/// local client loses nothing by reconnecting.
fn finish(mut stream: &TcpStream, reply: &Reply) {
    let body = reply.body.as_deref().unwrap_or("");
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nServer: {SERVER_HEADER}\r\nConnection: close\r\n\
         X-Content-Type-Options: nosniff\r\nContent-Length: {}\r\n",
        reply.status,
        reason(reply.status),
        body.len()
    );
    if reply.body.is_some() {
        head.push_str("Content-Type: application/json\r\n");
    }
    if reply.status == 405 {
        head.push_str("Allow: POST\r\n");
    }
    head.push_str("\r\n");
    let sent = stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body.as_bytes()))
        .and_then(|()| stream.flush());
    if sent.is_err() {
        return;
    }
    // Closing with unread request bytes would reset the connection and
    // could lose the reply, so say "done" and let the client finish first.
    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut sink = [0u8; 8192];
    let mut drained = 0;
    while drained < 2 * MAX_BODY {
        match stream.read(&mut sink) {
            Ok(n) if n > 0 => drained += n,
            _ => break,
        }
    }
}

/// Bytes from the client, with a deadline for the whole request.
struct Incoming<'a> {
    stream: &'a TcpStream,
    buffer: Vec<u8>,
    deadline: Instant,
}

impl Incoming<'_> {
    fn fill(&mut self) -> Result<(), Reply> {
        let late = || Reply::refuse(408, "The request took too long to arrive.");
        let left = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(late)?;
        // A zero timeout means "wait forever" to the OS.
        let left = left.max(Duration::from_millis(1));
        self.stream
            .set_read_timeout(Some(left))
            .map_err(|_| late())?;
        let mut chunk = [0u8; 8192];
        let mut stream = self.stream;
        match stream.read(&mut chunk) {
            Ok(0) => Err(Reply::refuse(400, "The request ended early.")),
            Ok(n) => {
                self.buffer.extend_from_slice(&chunk[..n]);
                Ok(())
            }
            Err(_) => Err(late()),
        }
    }

    /// Take everything up to `end` (and drop `end`). At most `max` bytes.
    fn take_until(&mut self, end: &[u8], max: usize, too_long: u16) -> Result<Vec<u8>, Reply> {
        loop {
            if let Some(at) = self.buffer.windows(end.len()).position(|w| w == end) {
                if at > max {
                    return Err(Reply::refuse(
                        too_long,
                        "The request's headers are too long.",
                    ));
                }
                let taken = self.buffer[..at].to_vec();
                self.buffer.drain(..at + end.len());
                return Ok(taken);
            }
            if self.buffer.len() > max {
                return Err(Reply::refuse(
                    too_long,
                    "The request's headers are too long.",
                ));
            }
            self.fill()?;
        }
    }

    fn take_exact(&mut self, n: usize) -> Result<Vec<u8>, Reply> {
        while self.buffer.len() < n {
            self.fill()?;
        }
        Ok(self.buffer.drain(..n).collect())
    }
}

struct Head {
    method: String,
    path: String,
    /// Names lower-cased.
    headers: Vec<(String, String)>,
}

impl Head {
    fn parse(bytes: &[u8]) -> Option<Head> {
        let text = std::str::from_utf8(bytes).ok()?;
        let mut lines = text.split("\r\n");
        let mut request = lines.next()?.split(' ');
        let (method, target, version) = (request.next()?, request.next()?, request.next()?);
        if request.next().is_some() || !version.starts_with("HTTP/1.") || method.is_empty() {
            return None;
        }
        let mut headers = Vec::new();
        for line in lines {
            let (name, value) = line.split_once(':')?;
            if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c.is_control()) {
                return None;
            }
            headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
        }
        Some(Head {
            method: method.to_string(),
            path: target.split('?').next()?.to_string(),
            headers,
        })
    }

    /// The header's value; `Err` if it is sent more than once (two
    /// answers to "who is asking" or "how long" is an attack or a bug).
    fn header(&self, name: &str) -> Result<Option<&str>, Reply> {
        let mut values = self
            .headers
            .iter()
            .filter(|h| h.0 == name)
            .map(|h| h.1.as_str());
        let first = values.next();
        if values.next().is_some() {
            return Err(Reply::refuse(400, "A header is repeated."));
        }
        Ok(first)
    }
}

/// Whether `authority` (`host` or `host:port`) names this machine.
fn is_loopback_authority(authority: &str) -> bool {
    let (host, port) = match authority.strip_prefix("[::1]") {
        Some("") => return true,
        Some(rest) => match rest.strip_prefix(':') {
            Some(port) => ("localhost", Some(port)),
            None => return false,
        },
        None => match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        },
    };
    let port_ok =
        port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()));
    port_ok && (host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost"))
}

/// Whether a request with this `Origin` may be answered: only pages served
/// from this machine. Compared exactly, so `http://localhost.evil.example`
/// and `null` are refused.
pub fn origin_allowed(origin: &str) -> bool {
    origin
        .strip_prefix("http://")
        .is_some_and(is_loopback_authority)
}

/// Whether a request sent to this `Host` was meant for this server.
pub fn host_allowed(host: &str) -> bool {
    is_loopback_authority(host)
}

fn exchange(stream: &TcpStream, handler: &dyn Handler) -> Reply {
    let mut incoming = Incoming {
        stream,
        buffer: Vec::new(),
        deadline: Instant::now() + REQUEST_TIMEOUT,
    };
    match read_request(&mut incoming) {
        Ok(body) => match handler.handle(&body) {
            Some(json) => Reply {
                status: 200,
                body: Some(json),
            },
            None => Reply {
                status: 202,
                body: None,
            },
        },
        Err(reply) => reply,
    }
}

/// Read one request and return its body, or the refusal to send.
fn read_request(incoming: &mut Incoming) -> Result<Vec<u8>, Reply> {
    let head = incoming.take_until(b"\r\n\r\n", MAX_HEAD, 431)?;
    let head =
        Head::parse(&head).ok_or_else(|| Reply::refuse(400, "That is not an HTTP request."))?;

    if let Some(host) = head.header("host")? {
        if !host_allowed(host) {
            return Err(Reply::refuse(
                403,
                "Edart only answers requests addressed to this Mac.",
            ));
        }
    }
    if let Some(origin) = head.header("origin")? {
        if !origin_allowed(origin) {
            return Err(Reply::refuse(
                403,
                "Edart does not answer requests from web pages.",
            ));
        }
    }
    if head.path != PATH {
        return Err(Reply::refuse(404, "The MCP endpoint is /mcp."));
    }
    if head.method != "POST" {
        // GET would open a server-to-client stream and DELETE would end a
        // session; this server has neither.
        return Err(Reply::refuse(405, "Send JSON-RPC messages with POST."));
    }
    let json = head.header("content-type")?.is_some_and(|t| {
        t.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
    });
    if !json {
        return Err(Reply::refuse(415, "Content-Type must be application/json."));
    }
    if let Some(version) = head.header("mcp-protocol-version")? {
        if !PROTOCOL_VERSIONS.contains(&version) {
            return Err(Reply::refuse(400, "Unsupported MCP-Protocol-Version."));
        }
    }

    let too_big = || Reply::refuse(413, "The request is larger than 1 MB.");
    let length = head.header("content-length")?;
    let chunked = match head.header("transfer-encoding")? {
        None => false,
        Some(encoding) if encoding.eq_ignore_ascii_case("chunked") => true,
        Some(_) => return Err(Reply::refuse(400, "Unsupported Transfer-Encoding.")),
    };
    if chunked && length.is_some() {
        return Err(Reply::refuse(
            400,
            "Send Content-Length or chunks, not both.",
        ));
    }
    if head
        .header("expect")?
        .is_some_and(|e| e.eq_ignore_ascii_case("100-continue"))
    {
        let mut stream = incoming.stream;
        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
    }
    if chunked {
        let mut body = Vec::new();
        loop {
            let line = incoming.take_until(b"\r\n", 64, 400)?;
            let size = std::str::from_utf8(&line)
                .ok()
                .and_then(|l| usize::from_str_radix(l.split(';').next()?.trim(), 16).ok())
                .ok_or_else(|| Reply::refuse(400, "A chunk has no valid size."))?;
            if size == 0 {
                // Trailers, if any, are not needed: the reply closes.
                return Ok(body);
            }
            if size > MAX_BODY - body.len() {
                return Err(too_big());
            }
            body.extend(incoming.take_exact(size)?);
            if incoming.take_exact(2)? != b"\r\n" {
                return Err(Reply::refuse(400, "A chunk is malformed."));
            }
        }
    }
    let length = length.ok_or_else(|| Reply::refuse(411, "Content-Length is required."))?;
    let length: usize = length
        .parse()
        .map_err(|_| Reply::refuse(400, "Content-Length is not a number."))?;
    if length > MAX_BODY {
        return Err(too_big());
    }
    incoming.take_exact(length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_origins_on_this_machine_are_allowed() {
        for ok in [
            "http://localhost",
            "http://localhost:6274",
            "http://LOCALHOST:1420",
            "http://127.0.0.1",
            "http://127.0.0.1:7391",
            "http://[::1]:8080",
            "http://[::1]",
        ] {
            assert!(origin_allowed(ok), "{ok}");
        }
        for bad in [
            "",
            "null",
            "https://example.com",
            "http://example.com",
            "http://localhost.evil.example",
            "http://localhost.evil.example:80",
            "http://127.0.0.1.evil.example",
            "http://127.0.0.10",
            "http://localhost:80@evil.example",
            "http://localhost:",
            "http://localhost:80/path",
            "http://localhost:999999",
            "http://[::1]evil",
            "https://localhost",
            "file://",
            "localhost",
        ] {
            assert!(!origin_allowed(bad), "{bad}");
        }
    }

    #[test]
    fn only_hosts_on_this_machine_are_allowed() {
        assert!(host_allowed("127.0.0.1:7391"));
        assert!(host_allowed("localhost:7391"));
        assert!(host_allowed("localhost"));
        assert!(!host_allowed("attacker.example:7391"));
        assert!(!host_allowed("192.168.1.5:7391"));
        assert!(!host_allowed(""));
    }

    #[test]
    fn the_request_head_parser_refuses_nonsense() {
        let head = Head::parse(b"POST /mcp?x=1 HTTP/1.1\r\nHost: localhost\r\nX-A:  b ").unwrap();
        assert_eq!((head.method.as_str(), head.path.as_str()), ("POST", "/mcp"));
        assert_eq!(head.header("x-a").ok().flatten(), Some("b"));
        assert!(Head::parse(b"POST /mcp").is_none());
        assert!(Head::parse(b"POST /mcp SPDY/3").is_none());
        assert!(Head::parse(b"POST /mcp HTTP/1.1\r\nno colon here").is_none());
        assert!(Head::parse(b"POST /mcp HTTP/1.1\r\nBad Name: x").is_none());
        assert!(Head::parse(b"\xff\xfe").is_none());
        let twice = Head::parse(b"POST /mcp HTTP/1.1\r\nHost: a\r\nHost: b").unwrap();
        assert!(twice.header("host").is_err());
    }
}
