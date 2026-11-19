//! The text on the iPhone's screen, read on this Mac.
//!
//! The trading rails need to know what a gesture would land on before it
//! is sent. This runs macOS Vision's text recognition on the newest
//! frame of the mirror and hands back each line with its place on the
//! screen. Nothing leaves the Mac, and nothing is installed on the
//! phone.
//!
//! The approach (a `VNRecognizeTextRequest` at the accurate level, the
//! first candidate of each observation, boxes flipped from Vision's
//! bottom-left origin) follows iPhone Use's `ScreenOCR.swift` (MIT,
//! © 2026 YueChen, https://github.com/YueChen-C/iPhone-use).
//!
//! What the text means is decided in `edart_core::guard`.

use std::sync::Mutex;
use std::time::Duration;

use edart_core::guard::{OcrFrame, OcrLine};
use edart_core::image::Rgb;
use edart_core::jpeg;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::AllocAnyThread;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSString};
use objc2_vision::{
    VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
    VNRequestTextRecognitionLevel,
};

use crate::mirror;

/// A frame older than this is not trusted to be what the phone shows
/// now. Kept just above the mirror's own stall restart (8 s): a phone may
/// send frames only when its screen changes, and a still screen's last
/// frame is still what it shows. A frozen stream is caught by the mirror
/// restarting, after which this rejects the old frame. Not yet measured
/// on a real iPhone; if a still Kalshi screen blocks gestures, look here.
const MAX_AGE: Duration = Duration::from_secs(10);
/// Vision is handed a JPEG; this keeps small text sharp.
const QUALITY: u8 = 92;

/// The text of the last frame read, so two looks at the same frame cost
/// one recognition.
static LAST: Mutex<Option<(u64, OcrFrame)>> = Mutex::new(None);

/// Read the text on the newest frame of the mirror. An error means the
/// screen cannot be checked.
pub fn read() -> Result<OcrFrame, String> {
    let (frame, seq, age) = mirror::frame().ok_or("the iPhone’s screen isn’t showing in Edart")?;
    if age > MAX_AGE {
        return Err(format!(
            "the picture of the iPhone’s screen is {} seconds old",
            age.as_secs()
        ));
    }
    read_frame(&frame, seq)
}

/// Read the text on one frame of the mirror; `seq` is its number.
pub fn read_frame(frame: &Rgb, seq: u64) -> Result<OcrFrame, String> {
    // Held across the recognition, so two callers share one pass.
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cached, text)) = last.as_ref() {
        if *cached == seq {
            return Ok(text.clone());
        }
    }
    let picture =
        jpeg::encode(frame, QUALITY).ok_or("the picture of the screen couldn’t be encoded")?;
    let (width, height) = (frame.width as f64, frame.height as f64);
    let lines = autoreleasepool(|_| recognize(&picture, width, height))?;
    let text = OcrFrame {
        width,
        height,
        lines,
    };
    *last = Some((seq, text.clone()));
    Ok(text)
}

fn recognize(jpeg: &[u8], width: f64, height: f64) -> Result<Vec<OcrLine>, String> {
    let data = NSData::with_bytes(jpeg);
    let options: Retained<NSDictionary<VNImageOption, AnyObject>> = NSDictionary::new();
    let handler = VNImageRequestHandler::initWithData_options(
        VNImageRequestHandler::alloc(),
        &data,
        &options,
    );

    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setRecognitionLanguages(&NSArray::from_retained_slice(&[NSString::from_str(
        "en-US",
    )]));
    // Correction helps with ordinary words; these are the ones it does
    // not know and must leave alone.
    request.setUsesLanguageCorrection(true);
    request.setCustomWords(&NSArray::from_retained_slice(&[
        NSString::from_str("Kalshi"),
        NSString::from_str("ZEC"),
        NSString::from_str("COMBO"),
    ]));

    let as_request: Retained<VNRequest> =
        Retained::into_super(Retained::into_super(request.clone()));
    handler
        .performRequests_error(&NSArray::from_retained_slice(&[as_request]))
        .map_err(|e| format!("text recognition failed: {}", e.localizedDescription()))?;

    let observations = request.results().ok_or("text recognition gave no answer")?;
    let mut lines = Vec::with_capacity(observations.len());
    for observation in observations.iter() {
        let Some(best) = observation.topCandidates(1).iter().next() else {
            continue;
        };
        let text = best.string().to_string();
        if text.trim().is_empty() {
            continue;
        }
        // SAFETY: a plain getter on an object this thread owns.
        let bounds = unsafe { observation.boundingBox() };
        lines.push(OcrLine::from_vision(
            text,
            best.confidence(),
            (
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.width,
                bounds.size.height,
            ),
            width,
            height,
        ));
    }
    Ok(lines)
}
