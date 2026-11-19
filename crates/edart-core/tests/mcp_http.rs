//! The MCP connector end to end: a real server on a real loopback port, a
//! fake phone behind it, and a client that speaks plain HTTP over TCP.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use edart_core::gesture::{Gesture, Screen};
use edart_core::mcp::http::{HttpServer, MAX_BODY};
use edart_core::mcp::server::Server;
use edart_core::mcp::stdio::{self, Bridge, Posted};
use edart_core::mcp::{CallLog, ControlLink, Frame, Phone, PhoneStatus};
use serde_json::{json, Value};

#[derive(Default)]
struct FakePhone {
    performed: Arc<Mutex<Vec<(Gesture, Screen)>>>,
}

impl Phone for FakePhone {
    fn status(&self) -> PhoneStatus {
        PhoneStatus {
            phone_name: Some("Test iPhone".into()),
            control: ControlLink::Usb,
            mirror_available: true,
        }
    }
    fn screenshot(&self) -> Result<Frame, String> {
        Ok(Frame {
            jpeg: b"\xff\xd8\xff\xe0 not really a jpeg".to_vec(),
            width: 390,
            height: 844,
        })
    }
    fn perform(&self, gesture: &Gesture, screen: Screen) -> Result<(), String> {
        self.performed
            .lock()
            .unwrap()
            .push((gesture.clone(), screen));
        Ok(())
    }
}

struct Running {
    server: HttpServer,
    performed: Arc<Mutex<Vec<(Gesture, Screen)>>>,
    log: Arc<CallLog>,
}

fn start() -> Running {
    let phone = FakePhone::default();
    let performed = phone.performed.clone();
    let log = Arc::new(CallLog::default());
    let handler = Arc::new(Server::new(phone, log.clone()));
    // Port 0: any free port, so tests can run side by side.
    let server = HttpServer::start(handler, 0..=0).expect("server starts");
    Running {
        server,
        performed,
        log,
    }
}

struct Response {
    status: u16,
    head: String,
    body: String,
}

impl Response {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).expect("a JSON body")
    }
}

/// Send raw bytes, read until the server closes.
fn send(port: u16, request: &[u8]) -> Response {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connects");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    // The server may answer (and stop reading) before a large request is
    // fully written; that is not this helper's failure.
    let _ = stream.write_all(request);
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw);
    let raw = String::from_utf8(raw).expect("UTF-8 response");
    let (head, body) = raw.split_once("\r\n\r\n").expect("a complete response");
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    Response {
        status,
        head: head.to_string(),
        body: body.to_string(),
    }
}

fn request(method: &str, port: u16, extra_headers: &str, body: &str) -> Vec<u8> {
    format!(
        "{method} /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra_headers}\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

const JSON: &str =
    "Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n";

fn rpc(port: u16, message: Value) -> Response {
    send(port, &request("POST", port, JSON, &message.to_string()))
}

fn call(port: u16, name: &str, arguments: Value) -> Value {
    let response = rpc(
        port,
        json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
                "params": { "name": name, "arguments": arguments } }),
    );
    assert_eq!(response.status, 200);
    response.json()["result"].clone()
}

#[test]
fn a_client_can_initialize_list_tools_screenshot_and_tap() {
    let running = start();
    let port = running.server.port();

    let init = rpc(
        port,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": { "name": "test", "version": "1" } } }),
    );
    assert_eq!(init.status, 200);
    assert!(init
        .head
        .to_lowercase()
        .contains("content-type: application/json"));
    assert!(init.head.contains("Server: edart-mcp"));
    let result = &init.json()["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(result["serverInfo"]["name"], "edart");

    // A notification: accepted, nothing to say.
    let initialized = rpc(
        port,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    );
    assert_eq!(initialized.status, 202);
    assert_eq!(initialized.body, "");

    let list = rpc(
        port,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    );
    let tools = list.json()["result"]["tools"].as_array().unwrap().len();
    assert_eq!(tools, 9);

    // Tapping before looking is refused, as a tool error.
    let early = call(port, "tap", json!({ "x": 10, "y": 10 }));
    assert_eq!(early["isError"], true);

    let shot = call(port, "screenshot", json!({}));
    assert_eq!(shot["isError"], false);
    assert_eq!(shot["content"][0]["type"], "image");
    assert_eq!(shot["content"][0]["mimeType"], "image/jpeg");
    assert!(shot["content"][0]["data"]
        .as_str()
        .unwrap()
        .starts_with("/9j/4CBub3Qg"));
    assert!(shot["content"][1]["text"]
        .as_str()
        .unwrap()
        .contains("390x844"));

    let tap = call(port, "tap", json!({ "x": 195, "y": 400 }));
    assert_eq!(tap["isError"], false, "{tap}");
    let outside = call(port, "tap", json!({ "x": 391, "y": 400 }));
    assert_eq!(outside["isError"], true);

    let performed = running.performed.lock().unwrap();
    assert_eq!(
        *performed,
        [(
            Gesture::Tap { x: 195.0, y: 400.0 },
            Screen {
                width: 390.0,
                height: 844.0
            }
        )]
    );
    let log: Vec<(String, bool)> = running
        .log
        .snapshot()
        .into_iter()
        .map(|c| (c.tool, c.ok))
        .collect();
    assert_eq!(
        log,
        [
            ("tap".to_string(), false),
            ("screenshot".to_string(), true),
            ("tap".to_string(), true),
            ("tap".to_string(), false)
        ]
    );
}

#[test]
fn requests_from_web_pages_are_refused() {
    let running = start();
    let port = running.server.port();
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": { "name": "press_home" } })
    .to_string();

    for origin in [
        "https://evil.example",
        "http://localhost.evil.example",
        "null",
    ] {
        let headers = format!("{JSON}Origin: {origin}\r\n");
        let response = send(port, &request("POST", port, &headers, &body));
        assert_eq!(response.status, 403, "{origin}");
        assert!(!response.head.to_lowercase().contains("access-control"));
    }
    // DNS rebinding: the page's own name resolves to 127.0.0.1, so the
    // browser sends that name as Host.
    let rebound = format!(
        "POST /mcp HTTP/1.1\r\nHost: evil.example:{port}\r\n{JSON}Content-Length: {}\r\n\r\n{body}",
        body.len()
    );
    assert_eq!(send(port, rebound.as_bytes()).status, 403);
    // What a cross-origin form or `no-cors` fetch can send without asking.
    let form = request("POST", port, "Content-Type: text/plain\r\n", &body);
    assert_eq!(send(port, &form).status, 415);
    // The preflight a page would need is never granted.
    let preflight = request(
        "OPTIONS",
        port,
        "Origin: https://evil.example\r\nAccess-Control-Request-Method: POST\r\n",
        "",
    );
    assert_eq!(send(port, &preflight).status, 403);

    assert!(
        running.performed.lock().unwrap().is_empty(),
        "nothing reached the phone"
    );

    // A page served from this machine (a local MCP inspector) may call.
    let headers = format!("{JSON}Origin: http://localhost:6274\r\n");
    assert_eq!(
        send(port, &request("POST", port, &headers, &body)).status,
        200
    );
    assert_eq!(running.performed.lock().unwrap().len(), 1);
}

#[test]
fn only_post_to_the_endpoint_is_served() {
    let running = start();
    let port = running.server.port();
    for method in ["GET", "DELETE", "OPTIONS", "PUT"] {
        let response = send(port, &request(method, port, "", ""));
        assert_eq!(response.status, 405, "{method}");
        assert!(response.head.contains("Allow: POST"));
    }
    let elsewhere =
        format!("POST / HTTP/1.1\r\nHost: localhost\r\n{JSON}Content-Length: 2\r\n\r\n{{}}");
    assert_eq!(send(port, elsewhere.as_bytes()).status, 404);
    assert_eq!(send(port, b"hello\r\n\r\n").status, 400);
    let no_length = format!("POST /mcp HTTP/1.1\r\nHost: localhost\r\n{JSON}\r\n");
    assert_eq!(send(port, no_length.as_bytes()).status, 411);
    let old = request(
        "POST",
        port,
        &format!("{JSON}MCP-Protocol-Version: 2020-01-01\r\n"),
        "{}",
    );
    assert_eq!(send(port, &old).status, 400);
    let current = request(
        "POST",
        port,
        &format!("{JSON}MCP-Protocol-Version: 2025-06-18\r\n"),
        r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
    );
    assert_eq!(send(port, &current).status, 200);
}

#[test]
fn oversized_requests_are_refused() {
    let running = start();
    let port = running.server.port();
    // Announced too big: refused before any of it is read.
    let announced = format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\n{JSON}Content-Length: {}\r\n\r\n",
        MAX_BODY + 1
    );
    assert_eq!(send(port, announced.as_bytes()).status, 413);
    // Actually too big.
    let big = "x".repeat(MAX_BODY + 1);
    assert_eq!(send(port, &request("POST", port, JSON, &big)).status, 413);
    // Exactly at the limit is read in full (and is not JSON-RPC).
    let padded = format!("{{}}{}", " ".repeat(MAX_BODY - 2));
    let at_limit = send(port, &request("POST", port, JSON, &padded));
    assert_eq!(at_limit.status, 200);
    assert_eq!(at_limit.json()["error"]["code"], -32600);
    // Headers that never end.
    let mut endless = b"POST /mcp HTTP/1.1\r\n".to_vec();
    endless.extend(format!("X-Pad: {}\r\n", "a".repeat(20_000)).into_bytes());
    assert_eq!(send(port, &endless).status, 431);
}

#[test]
fn chunked_bodies_and_json_errors_work_over_http() {
    let running = start();
    let port = running.server.port();
    let chunked = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{JSON}Transfer-Encoding: chunked\r\n\r\n\
         11\r\n{{\"jsonrpc\":\"2.0\",\r\n1c\r\n\"id\":\"c\",\"method\":\"ping\"}}   \r\n0\r\n\r\n"
    );
    let response = send(port, chunked.as_bytes());
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(
        response.json(),
        json!({ "jsonrpc": "2.0", "id": "c", "result": {} })
    );

    let garbage = send(port, &request("POST", port, JSON, "{oops"));
    assert_eq!(garbage.status, 200);
    assert_eq!(garbage.json()["error"]["code"], -32700);
    let batch = send(port, &request("POST", port, JSON, "[]"));
    assert_eq!(batch.json()["error"]["code"], -32600);
}

#[test]
fn stopping_the_server_frees_the_port() {
    let mut running = start();
    let port = running.server.port();
    assert_eq!(
        rpc(port, json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" })).status,
        200
    );
    running.server.stop();
    assert!(
        stdio::post(port, "{}").is_err(),
        "nothing answers after stop"
    );
    // And it can be taken again at once.
    let handler = Arc::new(Server::new(
        FakePhone::default(),
        Arc::new(CallLog::default()),
    ));
    let again = HttpServer::start(handler, port..=port).expect("the port is free again");
    assert_eq!(again.port(), port);
}

#[test]
fn a_busy_port_moves_the_server_to_the_next_one() {
    let first = start();
    let taken = first.server.port();
    let handler = Arc::new(Server::new(
        FakePhone::default(),
        Arc::new(CallLog::default()),
    ));
    let Ok(second) = HttpServer::start(handler, taken..=taken.saturating_add(3)) else {
        // The neighbours happened to be taken too; nothing to learn here.
        return;
    };
    assert_ne!(second.port(), taken);
    assert_eq!(
        rpc(
            second.port(),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" })
        )
        .status,
        200
    );
}

#[test]
fn the_stdio_bridge_forwards_to_the_running_app() {
    let running = start();
    let port = running.server.port();
    assert_eq!(
        stdio::post(
            port,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        )
        .unwrap(),
        Posted::Accepted
    );

    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"press_home"}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"screenshot"}}"#,
    ]
    .join("\n");
    let mut output = Vec::new();
    stdio::run(input.as_bytes(), &mut output, port..=port).unwrap();
    let lines: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON message per line"))
        .collect();
    assert_eq!(
        lines.len(),
        3,
        "the notification and the blank line print nothing"
    );
    assert_eq!(lines[0]["result"]["serverInfo"]["name"], "edart");
    assert_eq!(lines[1]["id"], 2);
    assert_eq!(lines[1]["result"]["isError"], false);
    assert_eq!(lines[2]["result"]["content"][0]["type"], "image");
    assert_eq!(running.performed.lock().unwrap()[0].0, Gesture::PressHome);
}

#[test]
fn the_stdio_bridge_says_to_open_edart_when_the_app_is_closed() {
    // A port nothing listens on: start a server to learn a free one, stop it.
    let mut gone = start();
    let port = gone.server.port();
    gone.server.stop();

    let mut bridge = Bridge::new(port..=port);
    let init: Value = serde_json::from_str(
        &bridge
            .answer(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        init["result"]["serverInfo"]["name"], "edart",
        "the client stays connected"
    );
    assert_eq!(
        bridge.answer(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        None
    );
    let list: Value = serde_json::from_str(
        &bridge
            .answer(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 9);
    let call: Value = serde_json::from_str(
        &bridge
            .answer(r#"{"jsonrpc":"2.0","id":"x","method":"tools/call","params":{"name":"get_status"}}"#)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(call["id"], "x");
    assert_eq!(call["result"]["isError"], true);
    assert!(call["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Open the Edart app"));

    // The app opens: the same bridge finds it.
    let handler = Arc::new(Server::new(
        FakePhone::default(),
        Arc::new(CallLog::default()),
    ));
    let _back = HttpServer::start(handler, port..=port).expect("the port is free");
    let call: Value = serde_json::from_str(
        &bridge
            .answer(
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_status"}}"#,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(call["result"]["isError"], false);
}

#[test]
fn the_stdio_bridge_ignores_other_programs_on_the_port() {
    // Something that is not Edart holds the port and answers 200.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nServer: other\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            );
        }
    });
    assert!(stdio::post(port, "{}").is_err());
}
