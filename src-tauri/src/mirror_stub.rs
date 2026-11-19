//! Screen mirroring on platforms other than macOS: not supported. Keeps
//! the command surface identical so the UI needs no special cases.

use edart_core::gesture::Screen;
use edart_core::mirror::{MirrorFacts, MirrorStatus};

pub fn already_allowed() -> bool {
    false
}

pub fn start() {}

pub fn status() -> MirrorStatus {
    MirrorStatus::new(&MirrorFacts::default(), 0, Vec::new())
}

pub fn preview_jpeg() -> Option<Vec<u8>> {
    None
}

pub fn screenshot() -> Option<(Vec<u8>, Screen)> {
    None
}

pub fn frame() -> Option<(
    std::sync::Arc<edart_core::image::Rgb>,
    u64,
    std::time::Duration,
)> {
    None
}
