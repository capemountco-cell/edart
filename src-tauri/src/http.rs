//! One blocking HTTPS request, on macOS's own URL loading system.
//!
//! The agent makes one request per step and Preferences makes two to test
//! the token. NSURLSession already does TLS with the system's trust store
//! and honours the system's proxy settings, so no HTTP or TLS library is
//! bundled for it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::autoreleasepool;
use objc2_foundation::{
    NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURLResponse, NSURLSession,
    NSURLSessionConfiguration, NSURL,
};

pub struct Response {
    pub status: u16,
    pub body: String,
}

/// Send a request and wait for the whole answer.
///
/// `cancel` is checked a few times a second; setting it abandons the
/// request. Errors are sentences for the user.
pub fn request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<Response, String> {
    autoreleasepool(|_| {
        let address = NSURL::URLWithString(&NSString::from_str(url))
            .ok_or_else(|| format!("“{url}” is not a valid address"))?;
        let request = NSMutableURLRequest::requestWithURL(&address);
        request.setHTTPMethod(&NSString::from_str(method));
        request.setTimeoutInterval(timeout.as_secs_f64());
        for (field, value) in headers {
            request.setValue_forHTTPHeaderField(
                Some(&NSString::from_str(value)),
                &NSString::from_str(field),
            );
        }
        if let Some(body) = body {
            request.setHTTPBody(Some(&NSData::with_bytes(body)));
        }

        let (tx, rx) = mpsc::channel::<Result<Response, String>>();
        let done = RcBlock::new(
            move |data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
                // SAFETY: the system passes valid objects or nil, alive for
                // the duration of this call.
                let (data, response, error) =
                    unsafe { (data.as_ref(), response.as_ref(), error.as_ref()) };
                let result = match (error, response) {
                    (Some(error), _) => Err(error.localizedDescription().to_string()),
                    (None, Some(response)) => match response.downcast_ref::<NSHTTPURLResponse>() {
                        Some(http) => Ok(Response {
                            status: u16::try_from(http.statusCode()).unwrap_or(0),
                            body: data
                                .map(|d| String::from_utf8_lossy(&d.to_vec()).into_owned())
                                .unwrap_or_default(),
                        }),
                        None => Err("the answer was not an HTTP response".to_string()),
                    },
                    (None, None) => Err("there was no answer".to_string()),
                };
                let _ = tx.send(result);
            },
        );

        // An ephemeral session: no cookies, cache or credentials on disk.
        let session = NSURLSession::sessionWithConfiguration(
            &NSURLSessionConfiguration::ephemeralSessionConfiguration(),
        );
        // SAFETY: the block matches the completion handler's signature and
        // is safe to call from the session's own queue (it only sends on a
        // channel).
        let task = unsafe { session.dataTaskWithRequest_completionHandler(&request, &done) };
        task.resume();
        // Lets the session go once the task is over, instead of leaking it.
        session.finishTasksAndInvalidate();

        // The session enforces `timeout` itself; this is a backstop.
        let deadline = Instant::now() + timeout + Duration::from_secs(10);
        loop {
            match rx.recv_timeout(Duration::from_millis(150)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("the request was dropped".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                task.cancel();
                return Err("cancelled".into());
            }
            if Instant::now() >= deadline {
                task.cancel();
                return Err("the request timed out".into());
            }
        }
    })
}
