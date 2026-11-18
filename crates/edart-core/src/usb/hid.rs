//! The HID reports and button events the phone's developer HID services
//! take, and the messages that carry them.
//!
//! Layouts are from iphone-use's `HIDService.swift` (MIT, Sunghyun Kim),
//! verified there on iOS 27.3. The tests pin the bytes.

use super::xpc::XpcValue;

/// The phone's service for raw HID reports (touchscreen, keyboard). It
/// lives on the Developer Disk Image.
pub const UNIVERSAL_HID_SERVICE: &str = "com.apple.coredevice.hid.universalhidservice";
/// The phone's service for hardware buttons (Home, lock, volume).
pub const INDIGO_SERVICE: &str = "com.apple.coredevice.hid.indigo";

/// The main touchscreen's HID surface. The phone creates it for each
/// connection to [`UNIVERSAL_HID_SERVICE`].
pub const TOUCH_SURFACE: u64 = 257;
/// The keyboard's HID surface, created the same way.
pub const KEYBOARD_SURFACE: u64 = 512;

/// Largest touch coordinate. Coordinates are fractions of the screen scaled
/// to 0..=65535, not pixels: anything out of range is dropped in silence.
pub const TOUCH_MAX: u16 = 65535;

/// How long a new connection must exist before its first report. The phone
/// builds the surfaces when the connection opens and drops, without an
/// error, reports that arrive before they are ready.
pub const SURFACE_SETTLE_MS: u64 = 400;
/// How long to wait after releasing a button. Closing sooner cancels a Home
/// press; pressing again sooner turns two presses into a double click.
pub const BUTTON_SETTLE_MS: u64 = 800;

const TOUCH_REPORT_ID: u8 = 0x09;
const TOUCH_CONTACT: u8 = 0xC2;
const TOUCH_RELEASE: u8 = 0x02;
const KEYBOARD_REPORT_ID: u8 = 0x01;

/// Left Shift, as a keyboard usage.
pub const KEY_LEFT_SHIFT: u8 = 0xE1;

/// Consumer-page usage of the Home button.
pub const BUTTON_HOME: u64 = 0x40;
const CONSUMER_PAGE: u64 = 0x0C;
pub const BUTTON_DOWN: u64 = 1;
pub const BUTTON_UP: u64 = 2;

fn timestamp_bytes(timestamp: u64) -> [u8; 6] {
    let b = timestamp.to_le_bytes();
    [b[0], b[1], b[2], b[3], b[4], b[5]]
}

/// The 58-byte touchscreen report.
///
/// ```text
/// 0      report id 0x09
/// 1-2    0x01 0x05
/// 3      0xC2 finger down / 0x02 lifted
/// 4-7    X, Y (u16 little-endian each, 0..=65535)
/// 8-39   zero
/// 40-43  0x02 0x00 0x00 0x00
/// 44-49  host timestamp in nanoseconds (48 bits, little-endian)
/// 50-57  zero
/// ```
pub fn touch_report(down: bool, x: u16, y: u16, timestamp: u64) -> Vec<u8> {
    let mut r = vec![0u8; 58];
    r[0] = TOUCH_REPORT_ID;
    r[1] = 0x01;
    r[2] = 0x05;
    r[3] = if down { TOUCH_CONTACT } else { TOUCH_RELEASE };
    r[4..6].copy_from_slice(&x.to_le_bytes());
    r[6..8].copy_from_slice(&y.to_le_bytes());
    r[40] = 0x02;
    r[44..50].copy_from_slice(&timestamp_bytes(timestamp));
    r
}

/// The 39-byte keyboard report: every key that is down right now, not a
/// change. No keys means everything is released.
///
/// ```text
/// 0      report id 0x01
/// 1-30   240-bit map: usage u is bit u%8 of byte 1 + u/8
/// 31-36  host timestamp in nanoseconds (48 bits, little-endian)
/// 37-38  zero
/// ```
pub fn keyboard_report(usages: &[u8], timestamp: u64) -> Vec<u8> {
    let mut r = vec![0u8; 39];
    r[0] = KEYBOARD_REPORT_ID;
    for &usage in usages.iter().filter(|&&u| u < 240) {
        r[1 + usize::from(usage) / 8] |= 1 << (usage % 8);
    }
    r[31..37].copy_from_slice(&timestamp_bytes(timestamp));
    r
}

/// The message that delivers one report to one HID surface.
pub fn report_message(surface: u64, report: Vec<u8>) -> XpcValue {
    XpcValue::dict([
        (
            "featureIdentifier",
            XpcValue::string("com.apple.coredevice.feature.remote.universalhidservice"),
        ),
        ("messageType", XpcValue::string("Request")),
        (
            "payload",
            XpcValue::dict([(
                "send",
                XpcValue::dict([
                    ("_0", XpcValue::Data(report)),
                    ("_1", XpcValue::Uint64(surface)),
                ]),
            )]),
        ),
    ])
}

/// The message that presses or releases a hardware button.
pub fn button_message(usage: u64, state: u64) -> XpcValue {
    XpcValue::dict([
        (
            "featureIdentifier",
            XpcValue::string("com.apple.coredevice.feature.remote.hid.button"),
        ),
        ("messageType", XpcValue::string("IndigoButtonEvent")),
        (
            "payload",
            XpcValue::dict([
                ("state", XpcValue::Uint64(state)),
                ("usagePage", XpcValue::Uint64(CONSUMER_PAGE)),
                ("usageCode", XpcValue::Uint64(usage)),
            ]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_touch_report_is_58_bytes_with_the_point_and_time() {
        let r = touch_report(true, 0x1234, 0xABCD, 0x0000_0605_0403_0201);
        assert_eq!(r.len(), 58);
        assert_eq!(&r[0..8], &[0x09, 0x01, 0x05, 0xC2, 0x34, 0x12, 0xCD, 0xAB]);
        assert!(r[8..40].iter().all(|&b| b == 0));
        assert_eq!(&r[40..44], &[0x02, 0, 0, 0]);
        assert_eq!(&r[44..50], &[1, 2, 3, 4, 5, 6]);
        assert!(r[50..].iter().all(|&b| b == 0));
        assert_eq!(touch_report(false, 0, 0, 0)[3], 0x02);
    }

    #[test]
    fn the_timestamp_keeps_only_48_bits() {
        let r = touch_report(true, 0, 0, u64::MAX);
        assert_eq!(&r[44..50], &[0xFF; 6]);
        assert_eq!(&r[50..], &[0; 8]);
    }

    #[test]
    fn a_keyboard_report_is_a_bitmap_of_held_keys() {
        // 'a' is usage 0x04: bit 4 of byte 1. Left Shift 0xE1: bit 1 of byte 29.
        let r = keyboard_report(&[0x04, KEY_LEFT_SHIFT], 0x0102_0304_0506);
        assert_eq!(r.len(), 39);
        assert_eq!(r[0], 0x01);
        assert_eq!(r[1], 0b0001_0000);
        assert_eq!(r[1 + 0xE1 / 8], 0b0000_0010);
        assert_eq!(r[1..31].iter().filter(|&&b| b != 0).count(), 2);
        assert_eq!(&r[31..37], &[6, 5, 4, 3, 2, 1]);
        assert_eq!(&r[37..], &[0, 0]);
    }

    #[test]
    fn no_keys_is_all_released_and_big_usages_are_ignored() {
        assert!(keyboard_report(&[], 0)[1..].iter().all(|&b| b == 0));
        assert!(keyboard_report(&[240, 255], 0)[1..].iter().all(|&b| b == 0));
    }

    #[test]
    fn a_report_message_names_the_surface() {
        let m = report_message(TOUCH_SURFACE, vec![9, 9]);
        assert_eq!(m.get("messageType").unwrap().as_str(), Some("Request"));
        let send = m.get("payload").unwrap().get("send").unwrap();
        assert_eq!(send.get("_0"), Some(&XpcValue::Data(vec![9, 9])));
        assert_eq!(send.get("_1"), Some(&XpcValue::Uint64(257)));
    }

    #[test]
    fn a_button_message_uses_the_consumer_page() {
        let m = button_message(BUTTON_HOME, BUTTON_DOWN);
        assert_eq!(
            m.get("messageType").unwrap().as_str(),
            Some("IndigoButtonEvent")
        );
        let p = m.get("payload").unwrap();
        assert_eq!(p.get("usagePage"), Some(&XpcValue::Uint64(0x0C)));
        assert_eq!(p.get("usageCode"), Some(&XpcValue::Uint64(0x40)));
        assert_eq!(p.get("state"), Some(&XpcValue::Uint64(1)));
    }
}
