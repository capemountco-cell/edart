//! HID reports: the bytes the Mac sends to the iPhone as a Bluetooth
//! keyboard and mouse.
//!
//! The Report Map and the report layouts come from Mac_iPhone_touch
//! (MIT, <https://github.com/Heresyouyou/Mac_iPhone_touch>), which took them
//! from iPhone Use (MIT). They are verified on real iPhones, and iOS caches
//! them in its pairing record: change a single byte and the phone shows
//! "connected" with no pointer until the user forgets and re-pairs Edart.
//! The tests below pin the exact bytes.

/// Keyboard: 8 bytes, modifiers + reserved + six key codes.
pub const RID_KEYBOARD: u8 = 1;
/// Absolute pointer: 5 bytes, buttons + X + Y (little-endian, 0..=32767).
/// Positions only: iOS ignores its button bits.
pub const RID_ABSOLUTE: u8 = 2;
/// Relative mouse: 4 bytes, buttons + dx + dy + wheel (signed bytes).
/// Carries clicks and drags.
pub const RID_RELATIVE: u8 = 3;
/// Consumer control: 2 bytes, one usage (Home is 0x0223).
pub const RID_CONSUMER: u8 = 4;

/// Largest coordinate an absolute report carries, on both axes.
pub const ABSOLUTE_MAX: u16 = 32767;

/// Consumer usage for the Home button (AC Home).
pub const USAGE_HOME: u16 = 0x0223;

/// The left mouse button bit.
pub const BUTTON_LEFT: u8 = 0x01;

/// Byte length of each input report, by report ID.
pub fn report_len(report_id: u8) -> usize {
    match report_id {
        RID_KEYBOARD => 8,
        RID_ABSOLUTE => 5,
        RID_RELATIVE => 4,
        RID_CONSUMER => 2,
        _ => 0,
    }
}

/// Keyboard (65) + absolute pointer (54) + relative mouse (54) + consumer (25)
/// = 198 bytes, in that order. iOS parses it in order.
pub const REPORT_MAP: [u8; 198] = [
    0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x85, 0x01, 0x05, 0x07, 0x19, 0xE0, 0x29, 0xE7, 0x15, 0x00,
    0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x95, 0x06,
    0x75, 0x08, 0x15, 0x00, 0x25, 0x65, 0x05, 0x07, 0x19, 0x00, 0x29, 0x65, 0x81, 0x00, 0x95, 0x05,
    0x75, 0x01, 0x05, 0x08, 0x19, 0x01, 0x29, 0x05, 0x91, 0x02, 0x95, 0x01, 0x75, 0x03, 0x91, 0x01,
    0xC0, 0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x85, 0x02, 0x09, 0x01, 0xA1, 0x00, 0x05, 0x09, 0x19,
    0x01, 0x29, 0x03, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03, 0x75, 0x01, 0x81, 0x02, 0x95, 0x01, 0x75,
    0x05, 0x81, 0x01, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x16, 0x00, 0x00, 0x26, 0xFF, 0x7F, 0x75,
    0x10, 0x95, 0x02, 0x81, 0x02, 0xC0, 0xC0, 0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x85, 0x03, 0x09,
    0x01, 0xA1, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29, 0x03, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03, 0x75,
    0x01, 0x81, 0x02, 0x95, 0x01, 0x75, 0x05, 0x81, 0x01, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x09,
    0x38, 0x15, 0x81, 0x25, 0x7F, 0x75, 0x08, 0x95, 0x03, 0x81, 0x06, 0xC0, 0xC0, 0x05, 0x0C, 0x09,
    0x01, 0xA1, 0x01, 0x85, 0x04, 0x15, 0x00, 0x26, 0xFF, 0x03, 0x19, 0x00, 0x2A, 0xFF, 0x03, 0x75,
    0x10, 0x95, 0x01, 0x81, 0x00, 0xC0,
];

/// One input report: which channel, and its payload (no report ID byte;
/// the ID lives in the characteristic's Report Reference descriptor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub id: u8,
    pub payload: Vec<u8>,
}

pub fn keyboard(modifiers: u8, usage: u8) -> Report {
    Report {
        id: RID_KEYBOARD,
        payload: vec![modifiers, 0, usage, 0, 0, 0, 0, 0],
    }
}

pub fn absolute(x: u16, y: u16) -> Report {
    let x = x.min(ABSOLUTE_MAX);
    let y = y.min(ABSOLUTE_MAX);
    Report {
        id: RID_ABSOLUTE,
        payload: vec![
            0,
            (x & 0xFF) as u8,
            (x >> 8) as u8,
            (y & 0xFF) as u8,
            (y >> 8) as u8,
        ],
    }
}

pub fn relative(buttons: u8, dx: i32, dy: i32, wheel: i32) -> Report {
    let b = |v: i32| v.clamp(-127, 127) as i8 as u8;
    Report {
        id: RID_RELATIVE,
        payload: vec![buttons, b(dx), b(dy), b(wheel)],
    }
}

pub fn consumer(usage: u16) -> Report {
    Report {
        id: RID_CONSUMER,
        payload: vec![(usage & 0xFF) as u8, (usage >> 8) as u8],
    }
}

/// Modifier bit for left Shift.
pub const MOD_SHIFT: u8 = 0x02;

/// The HID key for one character on a US layout: (modifiers, usage).
/// `None` for characters a US keyboard cannot type.
pub fn key_for_char(c: char) -> Option<(u8, u8)> {
    let plain = |u: u8| Some((0, u));
    let shifted = |u: u8| Some((MOD_SHIFT, u));
    match c {
        'a'..='z' => plain(0x04 + (c as u8 - b'a')),
        'A'..='Z' => shifted(0x04 + (c as u8 - b'A')),
        '1'..='9' => plain(0x1E + (c as u8 - b'1')),
        '0' => plain(0x27),
        '\n' => plain(0x28),
        '\t' => plain(0x2B),
        ' ' => plain(0x2C),
        '-' => plain(0x2D),
        '=' => plain(0x2E),
        '[' => plain(0x2F),
        ']' => plain(0x30),
        '\\' => plain(0x31),
        ';' => plain(0x33),
        '\'' => plain(0x34),
        '`' => plain(0x35),
        ',' => plain(0x36),
        '.' => plain(0x37),
        '/' => plain(0x38),
        '!' => shifted(0x1E),
        '@' => shifted(0x1F),
        '#' => shifted(0x20),
        '$' => shifted(0x21),
        '%' => shifted(0x22),
        '^' => shifted(0x23),
        '&' => shifted(0x24),
        '*' => shifted(0x25),
        '(' => shifted(0x26),
        ')' => shifted(0x27),
        '_' => shifted(0x2D),
        '+' => shifted(0x2E),
        '{' => shifted(0x2F),
        '}' => shifted(0x30),
        '|' => shifted(0x31),
        ':' => shifted(0x33),
        '"' => shifted(0x34),
        '~' => shifted(0x35),
        '<' => shifted(0x36),
        '>' => shifted(0x37),
        '?' => shifted(0x38),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_map_is_the_verified_one() {
        assert_eq!(REPORT_MAP.len(), 198);
        // Each collection is where the reference puts it.
        assert_eq!(
            &REPORT_MAP[..8],
            &[0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x85, 0x01]
        );
        assert_eq!(
            &REPORT_MAP[65..73],
            &[0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x85, 0x02]
        );
        assert_eq!(
            &REPORT_MAP[119..127],
            &[0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0x85, 0x03]
        );
        assert_eq!(
            &REPORT_MAP[173..181],
            &[0x05, 0x0C, 0x09, 0x01, 0xA1, 0x01, 0x85, 0x04]
        );
        // A fingerprint of every byte, so an accidental edit fails here. The
        // reference publishes SHA-256 b74d9ebb…d80e3 for these same bytes.
        let sum: u32 = REPORT_MAP.iter().map(|&b| b as u32).sum();
        let weighted: u64 = REPORT_MAP
            .iter()
            .enumerate()
            .map(|(i, &b)| (i as u64 + 1) * b as u64)
            .sum();
        assert_eq!((sum, weighted), (10672, 1108821));
    }

    #[test]
    fn encoders_have_the_declared_lengths() {
        assert_eq!(keyboard(0, 4).payload.len(), report_len(RID_KEYBOARD));
        assert_eq!(absolute(1, 2).payload.len(), report_len(RID_ABSOLUTE));
        assert_eq!(relative(0, 1, 2, 3).payload.len(), report_len(RID_RELATIVE));
        assert_eq!(consumer(USAGE_HOME).payload.len(), report_len(RID_CONSUMER));
    }

    #[test]
    fn absolute_is_little_endian_and_clamped() {
        assert_eq!(
            absolute(0x1234, 0x0102).payload,
            vec![0, 0x34, 0x12, 0x02, 0x01]
        );
        assert_eq!(absolute(u16::MAX, 0).payload, vec![0, 0xFF, 0x7F, 0, 0]);
    }

    #[test]
    fn relative_clamps_to_signed_bytes() {
        assert_eq!(relative(1, 500, -500, 0).payload, vec![1, 127, 0x81, 0]);
    }

    #[test]
    fn home_is_ac_home() {
        assert_eq!(consumer(USAGE_HOME).payload, vec![0x23, 0x02]);
    }

    #[test]
    fn keys_follow_the_us_layout() {
        assert_eq!(key_for_char('a'), Some((0, 0x04)));
        assert_eq!(key_for_char('Z'), Some((MOD_SHIFT, 0x1D)));
        assert_eq!(key_for_char('0'), Some((0, 0x27)));
        assert_eq!(key_for_char('?'), Some((MOD_SHIFT, 0x38)));
        assert_eq!(key_for_char('é'), None);
    }
}
