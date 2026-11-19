//! Text recognition on platforms other than macOS: not supported. The
//! trading rails treat that as "can't check", and refuse.

use edart_core::guard::OcrFrame;

pub fn read() -> Result<OcrFrame, String> {
    Err("reading the screen’s text needs macOS".into())
}

pub fn read_frame(_frame: &edart_core::image::Rgb, _seq: u64) -> Result<OcrFrame, String> {
    read()
}
