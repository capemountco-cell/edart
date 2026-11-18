//! Controlling the iPhone over the USB cable, the way Xcode's device tools
//! do: touch, keyboard and button events sent to the phone's own developer
//! HID services. No Bluetooth pairing, no pointer, no AssistiveTouch.
//!
//! The path, end to end:
//! 1. lockdown (over usbmux): is a phone plugged in, does it trust this Mac,
//!    is Developer Mode on ([`readiness`], [`lockdown`]);
//! 2. the Mac's CoreDevice service hands out a connected socket to a service
//!    on the phone ([`coredevice`]);
//! 3. that socket speaks RemoteXPC: XPC objects ([`xpc`]) in HTTP/2 DATA
//!    frames ([`remotexpc`]);
//! 4. raw HID reports go to the phone's touchscreen and keyboard ([`hid`]),
//!    planned from the same [`crate::gesture::Gesture`] values the Bluetooth
//!    path takes ([`plan`]).
//!
//! Everything here is bytes and decisions, so it is tested on Linux. The
//! calls into Apple's frameworks live in the Tauri crate (`src-tauri/src/usb`).
//!
//! Ported from iphone-use by Sunghyun Kim (MIT,
//! <https://github.com/sunghyun-k/iphone-use>, verified there on iOS 27.3)
//! and from iPhone Use by YueChen-C (MIT,
//! <https://github.com/YueChen-C/iPhone-use>), which builds on it.

pub mod coredevice;
pub mod hid;
pub mod lockdown;
pub mod plan;
pub mod readiness;
pub mod remotexpc;
pub mod xpc;
