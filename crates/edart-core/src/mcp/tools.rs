//! The tools an MCP client can call: their schemas, argument checks, and
//! what each does to the [`Phone`].

use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::{base64, Phone, ToolInfo};
use crate::gesture::{Gesture, Screen};

/// Longest `wait`.
pub const MAX_WAIT_MS: u64 = 5_000;
/// Longest press or swipe. The gesture planner's own limit.
pub const MAX_GESTURE_MS: u64 = 10_000;
/// Most characters one `type_text` call takes.
pub const MAX_TEXT_CHARS: usize = 2_000;

const NEEDS_SCREENSHOT: &str = "Take a screenshot first: coordinates are pixels of the most \
     recent screenshot.";

struct Def {
    name: &'static str,
    description: &'static str,
    /// `(name, schema, required)` for each argument.
    args: fn() -> Vec<(&'static str, Value, bool)>,
}

fn coordinate(what: &str) -> Value {
    json!({ "type": "number", "minimum": 0, "description": what })
}

fn duration(what: &str, max: u64) -> Value {
    json!({ "type": "integer", "minimum": 0, "maximum": max, "description": what })
}

const X: &str = "Pixels from the left edge of the most recent screenshot.";
const Y: &str = "Pixels from the top edge of the most recent screenshot.";

const DEFS: &[Def] = &[
    Def {
        name: "get_status",
        description: "Report which iPhone is connected, how it is controlled (bluetooth, usb \
                      or none) and whether screenshots are available.",
        args: Vec::new,
    },
    Def {
        name: "screenshot",
        description: "Take a picture of the iPhone's screen. Call this before any tap or swipe, \
                      and again after an action to see its result.",
        args: Vec::new,
    },
    Def {
        name: "tap",
        description: "Tap the iPhone's screen at a point of the most recent screenshot.",
        args: || vec![("x", coordinate(X), true), ("y", coordinate(Y), true)],
    },
    Def {
        name: "double_tap",
        description: "Tap twice quickly at a point of the most recent screenshot.",
        args: || vec![("x", coordinate(X), true), ("y", coordinate(Y), true)],
    },
    Def {
        name: "long_press",
        description: "Press and hold at a point of the most recent screenshot.",
        args: || {
            vec![
                ("x", coordinate(X), true),
                ("y", coordinate(Y), true),
                (
                    "ms",
                    duration(
                        "How long to hold, in milliseconds. Default 800.",
                        MAX_GESTURE_MS,
                    ),
                    false,
                ),
            ]
        },
    },
    Def {
        name: "swipe",
        description: "Drag a finger from one point of the most recent screenshot to another. \
                      To scroll down a page, swipe from lower to higher on the screen.",
        args: || {
            vec![
                ("from_x", coordinate(X), true),
                ("from_y", coordinate(Y), true),
                ("to_x", coordinate(X), true),
                ("to_y", coordinate(Y), true),
                (
                    "ms",
                    duration(
                        "How long the swipe takes, in milliseconds. Default 300.",
                        MAX_GESTURE_MS,
                    ),
                    false,
                ),
            ]
        },
    },
    Def {
        name: "type_text",
        description: "Type text on the iPhone as a keyboard would. Tap a text field first so \
                      it has focus. Only characters on a US keyboard can be typed.",
        args: || {
            vec![(
                "text",
                json!({ "type": "string", "minLength": 1, "maxLength": MAX_TEXT_CHARS }),
                true,
            )]
        },
    },
    Def {
        name: "press_home",
        description: "Go to the iPhone's Home Screen.",
        args: Vec::new,
    },
    Def {
        name: "wait",
        description: "Pause, for example while an app opens or an animation finishes.",
        args: || {
            vec![(
                "ms",
                duration("How long to wait, in milliseconds.", MAX_WAIT_MS),
                true,
            )]
        },
    },
];

fn find(name: &str) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.name == name)
}

pub fn exists(name: &str) -> bool {
    find(name).is_some()
}

/// Names and descriptions, for the app's tool list.
pub fn infos() -> Vec<ToolInfo> {
    DEFS.iter()
        .map(|d| ToolInfo {
            name: d.name,
            description: d.description,
        })
        .collect()
}

/// The `tools` array of a `tools/list` result.
pub fn definitions() -> Value {
    let tools: Vec<Value> = DEFS
        .iter()
        .map(|d| {
            let args = (d.args)();
            let required: Vec<&str> = args.iter().filter(|a| a.2).map(|a| a.0).collect();
            let properties: Map<String, Value> = args
                .into_iter()
                .map(|(name, schema, _)| (name.to_string(), schema))
                .collect();
            json!({
                "name": d.name,
                "description": d.description,
                "inputSchema": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false,
                },
            })
        })
        .collect();
    Value::Array(tools)
}

fn number(args: &Map<String, Value>, name: &str) -> Result<f64, String> {
    match args.get(name) {
        None | Some(Value::Null) => Err(format!("Missing argument \"{name}\".")),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("\"{name}\" must be a number.")),
    }
}

fn millis(args: &Map<String, Value>, name: &str, max: u64) -> Result<Option<u64>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(ms) if ms <= max => Ok(Some(ms)),
            _ => Err(format!(
                "\"{name}\" must be a whole number of milliseconds from 0 to {max}."
            )),
        },
    }
}

/// A point of the last screenshot, or why it is not one.
fn point(
    args: &Map<String, Value>,
    (x_name, y_name): (&str, &str),
    screen: Screen,
) -> Result<(f64, f64), String> {
    let (x, y) = (number(args, x_name)?, number(args, y_name)?);
    if x < 0.0 || y < 0.0 || x > screen.width || y > screen.height {
        return Err(format!(
            "({x}, {y}) is outside the screenshot, which is {} by {} pixels.",
            screen.width, screen.height
        ));
    }
    Ok((x, y))
}

fn text(message: impl Into<String>) -> Vec<Value> {
    vec![json!({ "type": "text", "text": message.into() })]
}

const THEN_LOOK: &str = "Take a screenshot to see the result.";

/// Run a tool. `Ok` is the result's `content`; `Err` is a message for the
/// model (reported as a tool error, so it can correct itself).
pub fn call<P: Phone + ?Sized>(
    phone: &P,
    last_screenshot: &Mutex<Option<Screen>>,
    name: &str,
    args: &Map<String, Value>,
) -> Result<Vec<Value>, String> {
    let def = find(name).ok_or_else(|| format!("Unknown tool \"{name}\"."))?;
    let known = (def.args)();
    if let Some(extra) = args
        .keys()
        .find(|k| !known.iter().any(|a| a.0 == k.as_str()))
    {
        return Err(format!("\"{name}\" has no argument \"{extra}\"."));
    }

    let last = *last_screenshot.lock().unwrap_or_else(|e| e.into_inner());
    // Tools that point at the screen need the picture the points refer to.
    let seen = || last.ok_or_else(|| NEEDS_SCREENSHOT.to_string());
    // The others ignore the size, but the planner wants a valid one.
    let any_screen = last.unwrap_or(Screen {
        width: 1.0,
        height: 1.0,
    });

    match name {
        "get_status" => {
            let status = serde_json::to_string(&phone.status()).map_err(|e| e.to_string())?;
            Ok(text(status))
        }
        "screenshot" => {
            let frame = phone.screenshot()?;
            if frame.width == 0 || frame.height == 0 || frame.jpeg.is_empty() {
                return Err("The iPhone's screen came back empty. Try again.".into());
            }
            *last_screenshot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Screen {
                width: f64::from(frame.width),
                height: f64::from(frame.height),
            });
            Ok(vec![
                json!({
                    "type": "image",
                    "data": base64::encode(&frame.jpeg),
                    "mimeType": "image/jpeg",
                }),
                json!({
                    "type": "text",
                    "text": format!(
                        "Screenshot is {}x{} pixels. Coordinates for tap, swipe and the other \
                         gestures are pixels of this image, from its top-left corner.",
                        frame.width, frame.height
                    ),
                }),
            ])
        }
        "tap" => {
            let screen = seen()?;
            let (x, y) = point(args, ("x", "y"), screen)?;
            phone.perform(&Gesture::Tap { x, y }, screen)?;
            Ok(text(format!("Tapped ({x}, {y}). {THEN_LOOK}")))
        }
        "double_tap" => {
            let screen = seen()?;
            let (x, y) = point(args, ("x", "y"), screen)?;
            phone.perform(&Gesture::DoubleTap { x, y }, screen)?;
            Ok(text(format!("Double-tapped ({x}, {y}). {THEN_LOOK}")))
        }
        "long_press" => {
            let screen = seen()?;
            let (x, y) = point(args, ("x", "y"), screen)?;
            let ms = millis(args, "ms", MAX_GESTURE_MS)?;
            phone.perform(&Gesture::LongPress { x, y, ms }, screen)?;
            Ok(text(format!("Pressed and held ({x}, {y}). {THEN_LOOK}")))
        }
        "swipe" => {
            let screen = seen()?;
            let (from_x, from_y) = point(args, ("from_x", "from_y"), screen)?;
            let (to_x, to_y) = point(args, ("to_x", "to_y"), screen)?;
            let ms = millis(args, "ms", MAX_GESTURE_MS)?;
            let gesture = Gesture::Swipe {
                from_x,
                from_y,
                to_x,
                to_y,
                ms,
            };
            phone.perform(&gesture, screen)?;
            Ok(text(format!(
                "Swiped from ({from_x}, {from_y}) to ({to_x}, {to_y}). {THEN_LOOK}"
            )))
        }
        "type_text" => {
            let typed = match args.get("text") {
                Some(Value::String(s)) => s,
                Some(_) => return Err("\"text\" must be a string.".into()),
                None => return Err("Missing argument \"text\".".into()),
            };
            let chars = typed.chars().count();
            if chars == 0 || chars > MAX_TEXT_CHARS {
                return Err(format!(
                    "\"text\" must be 1 to {MAX_TEXT_CHARS} characters."
                ));
            }
            phone.perform(
                &Gesture::TypeText {
                    text: typed.clone(),
                },
                any_screen,
            )?;
            Ok(text(format!("Typed {chars} characters. {THEN_LOOK}")))
        }
        "press_home" => {
            phone.perform(&Gesture::PressHome, any_screen)?;
            Ok(text(format!("Pressed Home. {THEN_LOOK}")))
        }
        "wait" => {
            let ms = millis(args, "ms", MAX_WAIT_MS)?
                .ok_or_else(|| "Missing argument \"ms\".".to_string())?;
            std::thread::sleep(Duration::from_millis(ms));
            Ok(text(format!("Waited {ms} ms.")))
        }
        _ => Err(format!("Unknown tool \"{name}\".")),
    }
}
