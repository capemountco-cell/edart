//! The app's logic.
//!
//! Everything that makes a decision lives here, not in the Tauri crate and not
//! in React. This crate does not depend on Tauri, so it builds and its tests
//! run on Linux in CI, without a window, a webview or a Mac.

pub mod agent;
pub mod base64;
pub mod gatt;
pub mod gesture;
pub mod hid;
pub mod image;
pub mod jpeg;
pub mod mirror;
pub mod settings;
pub mod status;
pub mod usb;
