//! The app's logic.
//!
//! Everything that makes a decision lives here, not in the Tauri crate and not
//! in React. This crate does not depend on Tauri, so it builds and its tests
//! run on Linux in CI, without a window, a webview or a Mac.

pub mod gatt;
pub mod gesture;
pub mod hid;
pub mod mcp;
pub mod status;
pub mod usb;
