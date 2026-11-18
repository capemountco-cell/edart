//! XPC objects and libxpc's serialization, which RemoteXPC puts on the wire.
//!
//! All integers are little-endian; strings and data are padded to a 4-byte
//! boundary. Ported from iphone-use's `XPCObject.swift` (MIT, Sunghyun Kim).

use std::collections::BTreeMap;

/// One XPC object. Dictionaries are kept sorted by key, so the encoding of a
/// message is the same every time.
#[derive(Debug, Clone, PartialEq)]
pub enum XpcValue {
    Null,
    Bool(bool),
    Int64(i64),
    Uint64(u64),
    Double(f64),
    Data(Vec<u8>),
    String(String),
    Uuid([u8; 16]),
    Array(Vec<XpcValue>),
    Dictionary(BTreeMap<String, XpcValue>),
}

const NULL: u32 = 0x1000;
const BOOL: u32 = 0x2000;
const INT64: u32 = 0x3000;
const UINT64: u32 = 0x4000;
const DOUBLE: u32 = 0x5000;
const DATA: u32 = 0x8000;
const STRING: u32 = 0x9000;
const UUID: u32 = 0xA000;
const ARRAY: u32 = 0xE000;
const DICTIONARY: u32 = 0xF000;

fn u32le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u64le(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn align(out: &mut Vec<u8>) {
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

impl XpcValue {
    /// Build a dictionary from `(key, value)` pairs.
    pub fn dict<K: Into<String>>(entries: impl IntoIterator<Item = (K, XpcValue)>) -> XpcValue {
        XpcValue::Dictionary(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    pub fn string(s: impl Into<String>) -> XpcValue {
        XpcValue::String(s.into())
    }

    /// The value under `key`, if this is a dictionary that has it.
    pub fn get(&self, key: &str) -> Option<&XpcValue> {
        match self {
            XpcValue::Dictionary(entries) => entries.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            XpcValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[XpcValue]> {
        match self {
            XpcValue::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The wire form of this object.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            XpcValue::Null => u32le(out, NULL),
            XpcValue::Bool(v) => {
                u32le(out, BOOL);
                u32le(out, u32::from(*v));
            }
            XpcValue::Int64(v) => {
                u32le(out, INT64);
                u64le(out, *v as u64);
            }
            XpcValue::Uint64(v) => {
                u32le(out, UINT64);
                u64le(out, *v);
            }
            XpcValue::Double(v) => {
                u32le(out, DOUBLE);
                u64le(out, v.to_bits());
            }
            XpcValue::Data(bytes) => {
                u32le(out, DATA);
                u32le(out, bytes.len() as u32);
                out.extend_from_slice(bytes);
                align(out);
            }
            XpcValue::String(s) => {
                // The length counts the terminating NUL.
                u32le(out, STRING);
                u32le(out, s.len() as u32 + 1);
                out.extend_from_slice(s.as_bytes());
                out.push(0);
                align(out);
            }
            XpcValue::Uuid(bytes) => {
                u32le(out, UUID);
                out.extend_from_slice(bytes);
            }
            XpcValue::Array(items) => {
                let mut body = Vec::new();
                u32le(&mut body, items.len() as u32);
                for item in items {
                    item.encode_into(&mut body);
                }
                u32le(out, ARRAY);
                u32le(out, body.len() as u32);
                out.extend_from_slice(&body);
            }
            XpcValue::Dictionary(entries) => {
                let mut body = Vec::new();
                u32le(&mut body, entries.len() as u32);
                for (key, value) in entries {
                    body.extend_from_slice(key.as_bytes());
                    body.push(0);
                    align(&mut body);
                    value.encode_into(&mut body);
                }
                u32le(out, DICTIONARY);
                u32le(out, body.len() as u32);
                out.extend_from_slice(&body);
            }
        }
    }
}

/// Format 16 bytes as an upper-case UUID string, the way `NSUUID` prints it.
pub fn uuid_string(bytes: &[u8; 16]) -> String {
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        hex[0..4].concat(),
        hex[4..6].concat(),
        hex[6..8].concat(),
        hex[8..10].concat(),
        hex[10..16].concat()
    )
}

/// Turn 16 random bytes into a version 4 UUID.
pub fn uuid_v4(mut random: [u8; 16]) -> [u8; 16] {
    random[6] = (random[6] & 0x0F) | 0x40;
    random[8] = (random[8] & 0x3F) | 0x80;
    random
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_encode_with_their_type_tag() {
        assert_eq!(XpcValue::Null.encode(), [0x00, 0x10, 0, 0]);
        assert_eq!(
            XpcValue::Bool(true).encode(),
            [0x00, 0x20, 0, 0, 1, 0, 0, 0]
        );
        assert_eq!(
            XpcValue::Uint64(257).encode(),
            [0x00, 0x40, 0, 0, 0x01, 0x01, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            XpcValue::Int64(-1).encode(),
            [0x00, 0x30, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
        );
    }

    #[test]
    fn strings_count_their_nul_and_pad_to_four_bytes() {
        // "abc" + NUL is exactly 4 bytes: no padding.
        assert_eq!(
            XpcValue::string("abc").encode(),
            [0x00, 0x90, 0, 0, 4, 0, 0, 0, b'a', b'b', b'c', 0]
        );
        // "abcd" + NUL is 5 bytes: padded to 8.
        assert_eq!(
            XpcValue::string("abcd").encode(),
            [0x00, 0x90, 0, 0, 5, 0, 0, 0, b'a', b'b', b'c', b'd', 0, 0, 0, 0]
        );
    }

    #[test]
    fn data_is_padded_but_its_length_is_not() {
        assert_eq!(
            XpcValue::Data(vec![1, 2, 3, 4, 5]).encode(),
            [0x00, 0x80, 0, 0, 5, 0, 0, 0, 1, 2, 3, 4, 5, 0, 0, 0]
        );
    }

    #[test]
    fn an_empty_dictionary_is_a_count_of_zero() {
        assert_eq!(
            XpcValue::dict::<&str>([]).encode(),
            [0x00, 0xF0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn dictionaries_sort_keys_and_pad_them() {
        let d = XpcValue::dict([("b", XpcValue::Bool(false)), ("a", XpcValue::Uint64(1))]);
        let bytes = d.encode();
        let mut want = vec![0x00, 0xF0, 0, 0];
        // body: count(4) + "a\0" padded(4) + uint64(12) + "b\0" padded(4) + bool(8)
        want.extend_from_slice(&32u32.to_le_bytes());
        want.extend_from_slice(&2u32.to_le_bytes());
        want.extend_from_slice(&[b'a', 0, 0, 0]);
        want.extend_from_slice(&[0x00, 0x40, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        want.extend_from_slice(&[b'b', 0, 0, 0]);
        want.extend_from_slice(&[0x00, 0x20, 0, 0, 0, 0, 0, 0]);
        assert_eq!(bytes, want);
    }

    #[test]
    fn arrays_carry_a_byte_length_and_a_count() {
        let a = XpcValue::Array(vec![XpcValue::Uint64(7), XpcValue::Null]);
        let mut want = vec![0x00, 0xE0, 0, 0];
        want.extend_from_slice(&20u32.to_le_bytes());
        want.extend_from_slice(&2u32.to_le_bytes());
        want.extend_from_slice(&[0x00, 0x40, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0]);
        want.extend_from_slice(&[0x00, 0x10, 0, 0]);
        assert_eq!(a.encode(), want);
    }

    #[test]
    fn uuids_print_like_nsuuid() {
        let bytes = uuid_v4([0xAB; 16]);
        assert_eq!(bytes[6], 0x4B);
        assert_eq!(bytes[8], 0xAB);
        assert_eq!(uuid_string(&bytes), "ABABABAB-ABAB-4BAB-ABAB-ABABABABABAB");
    }

    #[test]
    fn lookups_are_none_on_the_wrong_type() {
        let d = XpcValue::dict([("k", XpcValue::string("v"))]);
        assert_eq!(d.get("k").and_then(XpcValue::as_str), Some("v"));
        assert_eq!(d.get("missing"), None);
        assert_eq!(XpcValue::Null.get("k"), None);
        assert_eq!(XpcValue::Null.as_array(), None);
    }
}
