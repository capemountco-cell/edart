//! The Model Context Protocol, server side: one JSON-RPC 2.0 message in,
//! at most one out. No transport here; see [`super::http`].

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::{tools, CallLog, Phone};
use crate::gesture::Screen;

/// Protocol revisions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26"];

pub const SERVER_NAME: &str = "edart";

/// Told to the model when it connects.
pub const INSTRUCTIONS: &str = "Edart lets you see and control the user's real iPhone. \
Start with `screenshot`: you cannot act on a screen you have not seen. Coordinates for `tap`, \
`double_tap`, `long_press` and `swipe` are pixels of the most recent screenshot, measured from \
its top-left corner. Do one action at a time, then take another screenshot to check what \
happened before the next one; use `wait` if the screen is still changing. `type_text` types \
into whatever has focus, so tap a text field first. `press_home` returns to the Home Screen. \
If a tool reports that the iPhone is not connected, tell the user to open Edart and finish its \
setup. This is a real device: do not buy, send, delete or change settings unless the user \
asked for exactly that.";

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

/// The revision to answer `initialize` with: the client's if this server
/// speaks it, else the newest this server speaks.
pub fn negotiate(requested: Option<&str>) -> &'static str {
    PROTOCOL_VERSIONS
        .iter()
        .copied()
        .find(|v| Some(*v) == requested)
        .unwrap_or(PROTOCOL_VERSIONS[0])
}

pub fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn result_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// An MCP server for one phone.
pub struct Server<P> {
    phone: P,
    /// Size of the last screenshot handed out: what coordinates refer to.
    last_screenshot: Mutex<Option<Screen>>,
    log: Arc<CallLog>,
}

impl<P: Phone> Server<P> {
    pub fn new(phone: P, log: Arc<CallLog>) -> Self {
        Server {
            phone,
            last_screenshot: Mutex::new(None),
            log,
        }
    }

    /// Handle one message. `None` when it needs no reply (a notification,
    /// or a response to something this server never asked).
    pub fn handle(&self, body: &[u8]) -> Option<String> {
        let reply = match serde_json::from_slice::<Value>(body) {
            Ok(message) => self.handle_value(&message)?,
            Err(_) => error_response(Value::Null, PARSE_ERROR, "That is not valid JSON."),
        };
        Some(reply.to_string())
    }

    fn handle_value(&self, message: &Value) -> Option<Value> {
        let invalid = |id: Value, why: &str| Some(error_response(id, INVALID_REQUEST, why));
        let object = match message {
            Value::Object(object) => object,
            Value::Array(_) => {
                return invalid(Value::Null, "Batches are not supported. Send one message.")
            }
            _ => return invalid(Value::Null, "A message must be a JSON object."),
        };
        // An id is a string or a number; anything else cannot be echoed.
        let id = match object.get("id") {
            None => None,
            Some(id @ (Value::String(_) | Value::Number(_))) => Some(id.clone()),
            Some(_) => return invalid(Value::Null, "\"id\" must be a string or a number."),
        };
        let echo = id.clone().unwrap_or(Value::Null);
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return invalid(echo, "\"jsonrpc\" must be \"2.0\".");
        }
        let method = match object.get("method") {
            Some(Value::String(method)) => method.as_str(),
            Some(_) => return invalid(echo, "\"method\" must be a string."),
            // A response from the client. This server sends no requests.
            None if object.contains_key("result") || object.contains_key("error") => return None,
            None => return invalid(echo, "The message has no \"method\"."),
        };
        // No id: a notification. `notifications/initialized` and any other
        // are accepted without a reply, as JSON-RPC requires.
        let id = id?;
        let params = object.get("params");
        Some(match method {
            "initialize" => {
                let requested = params
                    .and_then(|p| p.get("protocolVersion"))
                    .and_then(Value::as_str);
                result_response(
                    id,
                    json!({
                        "protocolVersion": negotiate(requested),
                        "capabilities": { "tools": {} },
                        "serverInfo": {
                            "name": SERVER_NAME,
                            "title": "Edart",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                        "instructions": INSTRUCTIONS,
                    }),
                )
            }
            "ping" => result_response(id, json!({})),
            "tools/list" => result_response(id, json!({ "tools": tools::definitions() })),
            "tools/call" => self.call_tool(id, params),
            _ => error_response(
                id,
                METHOD_NOT_FOUND,
                &format!("Edart has no method \"{method}\"."),
            ),
        })
    }

    fn call_tool(&self, id: Value, params: Option<&Value>) -> Value {
        let bad = |why: &str| error_response(id.clone(), INVALID_PARAMS, why);
        let Some(Value::Object(params)) = params else {
            return bad("\"params\" must be an object with a tool \"name\".");
        };
        let Some(Value::String(name)) = params.get("name") else {
            return bad("\"name\" must be the name of a tool.");
        };
        if !tools::exists(name) {
            return bad(&format!("Unknown tool: {name}"));
        }
        let empty = serde_json::Map::new();
        let args = match params.get("arguments") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(args)) => args,
            Some(_) => return bad("\"arguments\" must be an object."),
        };
        // From here on, a failure is the tool's, not the protocol's: the
        // model sees it and can try something else.
        let outcome = tools::call(&self.phone, &self.last_screenshot, name, args);
        self.log
            .record(name, outcome.as_ref().map(|_| ()).map_err(Clone::clone));
        let result = match outcome {
            Ok(content) => json!({ "content": content, "isError": false }),
            Err(message) => json!({
                "content": [{ "type": "text", "text": message }],
                "isError": true,
            }),
        };
        result_response(id, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gesture::Gesture;
    use crate::mcp::{ControlLink, Frame, PhoneStatus};

    #[derive(Default)]
    struct FakePhone {
        performed: Mutex<Vec<(Gesture, Screen)>>,
        no_mirror: bool,
        refuse: Option<&'static str>,
    }

    impl Phone for FakePhone {
        fn status(&self) -> PhoneStatus {
            PhoneStatus {
                phone_name: Some("iPhone Air".into()),
                control: ControlLink::Bluetooth,
                mirror_available: !self.no_mirror,
            }
        }
        fn screenshot(&self) -> Result<Frame, String> {
            if self.no_mirror {
                return Err("Screen mirroring isn't available yet.".into());
            }
            Ok(Frame {
                jpeg: vec![0xff, 0xd8, 0xff],
                width: 1170,
                height: 2532,
            })
        }
        fn perform(&self, gesture: &Gesture, screen: Screen) -> Result<(), String> {
            if let Some(why) = self.refuse {
                return Err(why.into());
            }
            self.performed
                .lock()
                .unwrap()
                .push((gesture.clone(), screen));
            Ok(())
        }
    }

    fn server(phone: FakePhone) -> Server<FakePhone> {
        Server::new(phone, Arc::new(CallLog::default()))
    }

    fn ask(server: &Server<FakePhone>, message: Value) -> Value {
        let reply = server
            .handle(message.to_string().as_bytes())
            .expect("a reply");
        serde_json::from_str(&reply).unwrap()
    }

    fn call(server: &Server<FakePhone>, name: &str, arguments: Value) -> Value {
        let reply = ask(
            server,
            json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call",
                    "params": { "name": name, "arguments": arguments } }),
        );
        assert_eq!(reply["id"], 9);
        assert!(reply.get("error").is_none(), "protocol error: {reply}");
        reply["result"].clone()
    }

    fn error_text(result: &Value) -> &str {
        assert_eq!(result["isError"], true, "expected a tool error: {result}");
        result["content"][0]["text"].as_str().unwrap()
    }

    const PHONE: Screen = Screen {
        width: 1170.0,
        height: 2532.0,
    };

    #[test]
    fn initialize_echoes_a_supported_version_and_describes_the_server() {
        let s = server(FakePhone::default());
        let reply = ask(
            &s,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-03-26", "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" } } }),
        );
        let result = &reply["result"];
        assert_eq!(reply["id"], 1);
        assert_eq!(result["protocolVersion"], "2025-03-26");
        assert_eq!(result["serverInfo"]["name"], "edart");
        assert_eq!(result["capabilities"], json!({ "tools": {} }));
        let instructions = result["instructions"].as_str().unwrap();
        assert!(instructions.contains("screenshot"));
        assert!(instructions.contains("one action at a time"));
    }

    #[test]
    fn initialize_offers_the_newest_version_to_an_unknown_client() {
        let s = server(FakePhone::default());
        for params in [json!({ "protocolVersion": "1999-01-01" }), json!({})] {
            let reply = ask(
                &s,
                json!({ "jsonrpc": "2.0", "id": "a", "method": "initialize", "params": params }),
            );
            assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
            assert_eq!(reply["id"], "a");
        }
    }

    #[test]
    fn notifications_and_client_responses_get_no_reply() {
        let s = server(FakePhone::default());
        for message in [
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": {} }),
            json!({ "jsonrpc": "2.0", "id": 3, "result": {} }),
        ] {
            assert_eq!(s.handle(message.to_string().as_bytes()), None);
        }
    }

    #[test]
    fn ping_answers_with_an_empty_result_and_the_same_id() {
        let s = server(FakePhone::default());
        let reply = ask(
            &s,
            json!({ "jsonrpc": "2.0", "id": "p-1", "method": "ping" }),
        );
        assert_eq!(
            reply,
            json!({ "jsonrpc": "2.0", "id": "p-1", "result": {} })
        );
    }

    #[test]
    fn protocol_errors_use_the_json_rpc_codes() {
        let s = server(FakePhone::default());
        let code = |body: &str| {
            let reply: Value = serde_json::from_str(&s.handle(body.as_bytes()).unwrap()).unwrap();
            (
                reply["error"]["code"].as_i64().unwrap(),
                reply["id"].clone(),
            )
        };
        assert_eq!(code("{not json"), (PARSE_ERROR, Value::Null));
        assert_eq!(code("[]"), (INVALID_REQUEST, Value::Null));
        assert_eq!(
            code(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#),
            (INVALID_REQUEST, Value::Null)
        );
        assert_eq!(code("42"), (INVALID_REQUEST, Value::Null));
        assert_eq!(
            code(r#"{"id":1,"method":"ping"}"#),
            (INVALID_REQUEST, json!(1))
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":2,"method":7}"#),
            (INVALID_REQUEST, json!(2))
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":2}"#),
            (INVALID_REQUEST, json!(2))
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":{},"method":"ping"}"#),
            (INVALID_REQUEST, Value::Null)
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#),
            (METHOD_NOT_FOUND, json!(3))
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call"}"#),
            (INVALID_PARAMS, json!(4))
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"fly"}}"#),
            (INVALID_PARAMS, json!(5))
        );
        assert_eq!(
            code(
                r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"tap","arguments":[1,2]}}"#
            ),
            (INVALID_PARAMS, json!(6))
        );
    }

    #[test]
    fn tools_list_describes_every_tool_with_a_schema() {
        let s = server(FakePhone::default());
        let reply = ask(
            &s,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        );
        let tools = reply["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "get_status",
                "screenshot",
                "tap",
                "double_tap",
                "long_press",
                "swipe",
                "type_text",
                "press_home",
                "wait"
            ]
        );
        for tool in tools {
            assert!(!tool["description"].as_str().unwrap().is_empty());
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        }
        let swipe = &tools[5]["inputSchema"];
        assert_eq!(
            swipe["required"],
            json!(["from_x", "from_y", "to_x", "to_y"])
        );
        assert_eq!(swipe["properties"]["ms"]["maximum"], 10_000);
        assert_eq!(
            tools[8]["inputSchema"]["properties"]["ms"]["maximum"],
            5_000
        );
        assert_eq!(tools[7]["inputSchema"]["required"], json!([]));
    }

    #[test]
    fn get_status_reports_the_phone_and_its_links() {
        let s = server(FakePhone::default());
        let result = call(&s, "get_status", json!({}));
        assert_eq!(result["isError"], false);
        let status: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            status,
            json!({ "phone_name": "iPhone Air", "control": "bluetooth", "mirror_available": true })
        );
    }

    #[test]
    fn screenshot_returns_a_jpeg_and_its_size() {
        let s = server(FakePhone::default());
        let result = call(&s, "screenshot", json!({}));
        assert_eq!(result["isError"], false);
        assert_eq!(
            result["content"][0],
            json!({ "type": "image", "data": "/9j/", "mimeType": "image/jpeg" })
        );
        assert_eq!(result["content"][1]["type"], "text");
        assert!(result["content"][1]["text"]
            .as_str()
            .unwrap()
            .contains("1170x2532"));
    }

    #[test]
    fn a_missing_mirror_is_a_tool_error_not_a_protocol_error() {
        let s = server(FakePhone {
            no_mirror: true,
            ..Default::default()
        });
        let result = call(&s, "screenshot", json!({}));
        assert!(error_text(&result).contains("isn't available yet"));
    }

    #[test]
    fn pointing_tools_need_a_screenshot_first() {
        let s = server(FakePhone::default());
        for (tool, args) in [
            ("tap", json!({ "x": 1, "y": 1 })),
            ("double_tap", json!({ "x": 1, "y": 1 })),
            ("long_press", json!({ "x": 1, "y": 1 })),
            (
                "swipe",
                json!({ "from_x": 1, "from_y": 1, "to_x": 2, "to_y": 2 }),
            ),
        ] {
            let result = call(&s, tool, args);
            assert!(
                error_text(&result).contains("Take a screenshot first"),
                "{tool}"
            );
        }
        assert!(s.phone.performed.lock().unwrap().is_empty());
    }

    #[test]
    fn tools_that_point_nowhere_work_without_a_screenshot() {
        let s = server(FakePhone::default());
        assert_eq!(call(&s, "press_home", json!({}))["isError"], false);
        assert_eq!(
            call(&s, "type_text", json!({ "text": "hi" }))["isError"],
            false
        );
        assert_eq!(call(&s, "wait", json!({ "ms": 1 }))["isError"], false);
        assert_eq!(call(&s, "get_status", json!({}))["isError"], false);
        let performed = s.phone.performed.lock().unwrap();
        assert_eq!(performed[0].0, Gesture::PressHome);
        assert_eq!(performed[1].0, Gesture::TypeText { text: "hi".into() });
        // The stand-in screen must be one the planners accept.
        assert!(performed[0].1.width > 0.0 && performed[0].1.height > 0.0);
    }

    #[test]
    fn gestures_reach_the_phone_in_screenshot_units() {
        let s = server(FakePhone::default());
        call(&s, "screenshot", json!({}));
        assert_eq!(
            call(&s, "tap", json!({ "x": 100, "y": 200.5 }))["isError"],
            false
        );
        call(&s, "double_tap", json!({ "x": 0, "y": 0 }));
        call(
            &s,
            "long_press",
            json!({ "x": 1170, "y": 2532, "ms": 1200 }),
        );
        call(&s, "long_press", json!({ "x": 5, "y": 6 }));
        call(
            &s,
            "swipe",
            json!({ "from_x": 500, "from_y": 2000, "to_x": 500, "to_y": 400, "ms": 250 }),
        );
        let performed = s.phone.performed.lock().unwrap();
        let gestures: Vec<&Gesture> = performed.iter().map(|p| &p.0).collect();
        assert_eq!(
            gestures,
            [
                &Gesture::Tap { x: 100.0, y: 200.5 },
                &Gesture::DoubleTap { x: 0.0, y: 0.0 },
                &Gesture::LongPress {
                    x: 1170.0,
                    y: 2532.0,
                    ms: Some(1200)
                },
                &Gesture::LongPress {
                    x: 5.0,
                    y: 6.0,
                    ms: None
                },
                &Gesture::Swipe {
                    from_x: 500.0,
                    from_y: 2000.0,
                    to_x: 500.0,
                    to_y: 400.0,
                    ms: Some(250)
                },
            ]
        );
        assert!(performed.iter().all(|p| p.1 == PHONE));
    }

    #[test]
    fn bad_arguments_are_tool_errors_and_touch_nothing() {
        let s = server(FakePhone::default());
        call(&s, "screenshot", json!({}));
        let cases = [
            (
                "tap",
                json!({ "x": 1171, "y": 10 }),
                "outside the screenshot",
            ),
            ("tap", json!({ "x": 10, "y": -1 }), "outside the screenshot"),
            ("tap", json!({ "x": 10 }), "Missing argument \"y\""),
            ("tap", json!({ "x": "10", "y": 1 }), "must be a number"),
            (
                "tap",
                json!({ "x": 1, "y": 1, "force": 2 }),
                "no argument \"force\"",
            ),
            (
                "swipe",
                json!({ "from_x": 1, "from_y": 1, "to_x": 1, "to_y": 9999 }),
                "outside the screenshot",
            ),
            (
                "long_press",
                json!({ "x": 1, "y": 1, "ms": 10_001 }),
                "from 0 to 10000",
            ),
            (
                "long_press",
                json!({ "x": 1, "y": 1, "ms": 1.5 }),
                "whole number",
            ),
            ("wait", json!({ "ms": 5001 }), "from 0 to 5000"),
            ("wait", json!({}), "Missing argument \"ms\""),
            ("type_text", json!({ "text": "" }), "1 to 2000 characters"),
            ("type_text", json!({ "text": 5 }), "must be a string"),
            ("type_text", json!({}), "Missing argument \"text\""),
            ("press_home", json!({ "x": 1 }), "no argument \"x\""),
        ];
        for (tool, args, expected) in cases {
            let result = call(&s, tool, args.clone());
            let message = error_text(&result);
            assert!(message.contains(expected), "{tool} {args}: {message}");
        }
        assert!(s.phone.performed.lock().unwrap().is_empty());
    }

    #[test]
    fn a_phone_that_refuses_is_a_tool_error() {
        let s = server(FakePhone {
            refuse: Some("No iPhone is ready yet."),
            ..Default::default()
        });
        let result = call(&s, "press_home", json!({}));
        assert_eq!(error_text(&result), "No iPhone is ready yet.");
    }

    #[test]
    fn every_tool_call_is_logged_with_its_outcome() {
        let s = server(FakePhone::default());
        call(&s, "tap", json!({ "x": 1, "y": 1 }));
        call(&s, "screenshot", json!({}));
        call(&s, "tap", json!({ "x": 1, "y": 1 }));
        // Not tool calls: nothing to log.
        ask(
            &s,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        );
        ask(
            &s,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "x" } }),
        );
        let calls = s.log.snapshot();
        let summary: Vec<(&str, bool)> = calls.iter().map(|c| (c.tool.as_str(), c.ok)).collect();
        assert_eq!(
            summary,
            [("tap", false), ("screenshot", true), ("tap", true)]
        );
        assert!(calls[0]
            .error
            .as_deref()
            .unwrap()
            .contains("screenshot first"));
        assert!(calls[0].at_ms > 0);
    }
}
