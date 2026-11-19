//! USB control on platforms other than macOS: not supported. Keeps the
//! command surface identical so the UI needs no special cases.

use edart_core::usb::plan::UsbStep;
use edart_core::usb::readiness::{DeveloperModeOutcome, UsbFacts, UsbStatus};

const NEEDS_MAC: &str = "USB control needs macOS.";

#[allow(dead_code)]
pub fn start() {}

pub fn status() -> UsbStatus {
    UsbStatus::new(&UsbFacts::default(), None, Vec::new())
}

pub fn is_ready() -> bool {
    false
}

pub fn enable_developer_mode() -> Result<DeveloperModeOutcome, String> {
    Err(NEEDS_MAC.into())
}

pub fn prepare() -> Result<(), String> {
    Err(NEEDS_MAC.into())
}

pub fn perform(_steps: Vec<UsbStep>) -> Result<(), String> {
    Err(NEEDS_MAC.into())
}
