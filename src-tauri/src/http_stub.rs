//! HTTPS on platforms other than macOS: not supported. Keeps the command
//! surface identical so the UI needs no special cases.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub struct Response {
    pub status: u16,
    pub body: String,
}

pub fn request(
    _method: &str,
    _url: &str,
    _headers: &[(&str, &str)],
    _body: Option<&[u8]>,
    _timeout: Duration,
    _cancel: Option<&AtomicBool>,
) -> Result<Response, String> {
    Err("the agent needs macOS".into())
}
