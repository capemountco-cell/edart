//! The Bluetooth GATT database that makes the Mac look like a keyboard and
//! mouse (HID over GATT), written as plain data.
//!
//! Everything that decides whether an iPhone accepts Edart is a static fact
//! about this table, so it is asserted here, without Bluetooth hardware. The
//! macOS layer only turns it into CoreBluetooth objects.
//!
//! Facts learned on real hardware by Mac_iPhone_touch (MIT):
//! - macOS refuses every 16-bit service UUID in `addService:`
//!   (`CBErrorUUIDNotAllowed`), but accepts the same service written on the
//!   Bluetooth base UUID: 0x1812 == 00001812-0000-1000-8000-00805F9B34FB.
//!   Characteristics and descriptors may stay 16-bit.
//! - The Report Map and every Report must require encryption. With plain
//!   permissions iOS connects and subscribes but never treats the bytes as
//!   input, and reports no error. Protocol Mode is the one exception: the
//!   phone writes it before pairing.
//! - The service order is cached by iOS; changing it needs a re-pair.
//!
//! From iPhone Use (MIT, <https://github.com/YueChen-C/iPhone-use>), the
//! project both of the above descend from:
//! - Publish Generic Attribute (0x1801) with Service Changed. An iPhone that
//!   is already bonded with the Mac (iCloud, Handoff) trusts its cached copy
//!   of the Mac's services, writes the Control Point and never subscribes.
//!   Indicating Service Changed makes it rediscover and find the HID service.
//! - Serve the Report Map on request rather than as a cached value, so its
//!   encryption requirement is enforced and the phone bonds.

use crate::hid;

// Characteristic property bits, numerically equal to CoreBluetooth's
// `CBCharacteristicProperties`.
pub const PROP_READ: u32 = 0x02;
pub const PROP_WRITE_NO_RSP: u32 = 0x04;
pub const PROP_WRITE: u32 = 0x08;
pub const PROP_NOTIFY: u32 = 0x10;
pub const PROP_INDICATE: u32 = 0x20;

// Attribute permission bits, numerically equal to `CBAttributePermissions`.
pub const PERM_READ: u32 = 0x01;
pub const PERM_WRITE: u32 = 0x02;
pub const PERM_READ_ENC: u32 = 0x04;
pub const PERM_WRITE_ENC: u32 = 0x08;

pub const SERVICE_GATT: u16 = 0x1801;
pub const SERVICE_HID: u16 = 0x1812;
pub const SERVICE_DEVICE_INFO: u16 = 0x180A;
pub const SERVICE_BATTERY: u16 = 0x180F;

pub const CHAR_SERVICE_CHANGED: u16 = 0x2A05;
pub const CHAR_DATABASE_HASH: u16 = 0x2B2A;
pub const CHAR_HID_INFO: u16 = 0x2A4A;
pub const CHAR_REPORT_MAP: u16 = 0x2A4B;
pub const CHAR_CONTROL_POINT: u16 = 0x2A4C;
pub const CHAR_REPORT: u16 = 0x2A4D;
pub const CHAR_PROTOCOL_MODE: u16 = 0x2A4E;
pub const CHAR_MANUFACTURER: u16 = 0x2A29;
pub const CHAR_PNP_ID: u16 = 0x2A50;
pub const CHAR_BATTERY_LEVEL: u16 = 0x2A19;

/// Report Reference descriptor: `[report id, report type]`.
pub const DESC_REPORT_REFERENCE: u16 = 0x2908;

/// Report types in a Report Reference.
pub const REPORT_TYPE_INPUT: u8 = 1;
pub const REPORT_TYPE_OUTPUT: u8 = 2;

/// bcdHID 1.11, country 0, normally connectable.
pub const HID_INFORMATION: [u8; 4] = [0x11, 0x01, 0x00, 0x02];
/// Vendor ID source 2 (USB), vendor 0xFFFF, product 0x0001, version 0x0001.
pub const PNP_ID: [u8; 7] = [0x02, 0xFF, 0xFF, 0x00, 0x01, 0x00, 0x01];
pub const MANUFACTURER: &str = "Edart";

/// Service Changed value: "everything from handle 0x0010 to the end".
pub const SERVICE_CHANGED_ALL: [u8; 4] = [0x10, 0x00, 0xFF, 0xFF];

/// The name the iPhone shows in its Bluetooth device list.
pub const ADVERTISED_NAME: &str = "Edart";

/// How the macOS layer answers a read of a characteristic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A fixed value CoreBluetooth serves itself.
    Static(Vec<u8>),
    /// Answered at read time with this many zero bytes (or the given bytes).
    Dynamic(Vec<u8>),
    /// Write-only; never read.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharSpec {
    pub uuid: u16,
    pub properties: u32,
    pub permissions: u32,
    pub value: Value,
    /// `(report id, report type)` when the characteristic is a Report.
    pub report: Option<(u8, u8)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSpec {
    pub uuid: u16,
    pub characteristics: Vec<CharSpec>,
}

impl ServiceSpec {
    /// The UUID string to hand to `addService:`: always the 128-bit form.
    pub fn wire_uuid(&self) -> String {
        sig128(self.uuid)
    }
}

/// A 16-bit SIG UUID written on the Bluetooth base UUID.
pub fn sig128(short: u16) -> String {
    format!("{short:08X}-0000-1000-8000-00805F9B34FB")
}

/// A 16-bit UUID as CoreBluetooth's short string form ("2A4D").
pub fn short(uuid: u16) -> String {
    format!("{uuid:04X}")
}

fn input_report(id: u8) -> CharSpec {
    CharSpec {
        uuid: CHAR_REPORT,
        properties: PROP_READ | PROP_NOTIFY,
        permissions: PERM_READ_ENC,
        value: Value::Dynamic(vec![0; hid::report_len(id)]),
        report: Some((id, REPORT_TYPE_INPUT)),
    }
}

/// A Database Hash for this launch, made from `seed`. A phone that
/// supports GATT caching compares it with the one it remembers; a new
/// value tells it that its copy of Edart's services is out of date, as it
/// is after Edart is reinstalled.
pub fn database_hash(seed: u128) -> [u8; 16] {
    // Spread the seed so nearby seeds give unrelated hashes (splitmix64).
    let mix = |mut z: u64| {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let lo = mix(seed as u64);
    let hi = mix((seed >> 64) as u64 ^ lo);
    let mut out = [0; 16];
    out[..8].copy_from_slice(&lo.to_le_bytes());
    out[8..].copy_from_slice(&hi.to_le_bytes());
    out
}

/// Generic Attribute (0x1801): Service Changed, plus the Database Hash
/// when one is given. macOS may refuse the hash; then the service is
/// published again without it.
pub fn generic_attribute(db_hash: Option<[u8; 16]>) -> ServiceSpec {
    let mut characteristics = vec![CharSpec {
        uuid: CHAR_SERVICE_CHANGED,
        properties: PROP_INDICATE,
        permissions: PERM_READ,
        value: Value::None,
        report: None,
    }];
    if let Some(hash) = db_hash {
        characteristics.push(CharSpec {
            uuid: CHAR_DATABASE_HASH,
            properties: PROP_READ,
            permissions: PERM_READ,
            // Served live, so the phone reading it shows in the log.
            value: Value::Dynamic(hash.to_vec()),
            report: None,
        });
    }
    ServiceSpec {
        uuid: SERVICE_GATT,
        characteristics,
    }
}

/// The services to publish, in publishing order.
pub fn services(db_hash: [u8; 16]) -> Vec<ServiceSpec> {
    vec![
        generic_attribute(Some(db_hash)),
        ServiceSpec {
            uuid: SERVICE_HID,
            characteristics: vec![
                CharSpec {
                    uuid: CHAR_HID_INFO,
                    properties: PROP_READ,
                    permissions: PERM_READ,
                    // Served on request so the first, unencrypted read shows
                    // up in the log: proof the phone reached Edart before
                    // any pairing.
                    value: Value::Dynamic(HID_INFORMATION.to_vec()),
                    report: None,
                },
                CharSpec {
                    uuid: CHAR_REPORT_MAP,
                    properties: PROP_READ,
                    permissions: PERM_READ_ENC,
                    // Served on request: a cached value may skip encryption.
                    value: Value::Dynamic(hid::REPORT_MAP.to_vec()),
                    report: None,
                },
                CharSpec {
                    uuid: CHAR_CONTROL_POINT,
                    properties: PROP_WRITE_NO_RSP,
                    permissions: PERM_WRITE_ENC,
                    value: Value::None,
                    report: None,
                },
                CharSpec {
                    uuid: CHAR_PROTOCOL_MODE,
                    properties: PROP_READ | PROP_WRITE_NO_RSP,
                    permissions: PERM_READ | PERM_WRITE,
                    // 1 = Report Protocol.
                    value: Value::Dynamic(vec![1]),
                    report: None,
                },
                input_report(hid::RID_KEYBOARD),
                input_report(hid::RID_ABSOLUTE),
                input_report(hid::RID_RELATIVE),
                input_report(hid::RID_CONSUMER),
                // Keyboard LEDs, written by the phone.
                CharSpec {
                    uuid: CHAR_REPORT,
                    properties: PROP_READ | PROP_WRITE | PROP_WRITE_NO_RSP,
                    permissions: PERM_READ_ENC | PERM_WRITE_ENC,
                    value: Value::Dynamic(vec![0]),
                    report: Some((hid::RID_KEYBOARD, REPORT_TYPE_OUTPUT)),
                },
            ],
        },
        ServiceSpec {
            uuid: SERVICE_DEVICE_INFO,
            characteristics: vec![
                CharSpec {
                    uuid: CHAR_MANUFACTURER,
                    properties: PROP_READ,
                    permissions: PERM_READ,
                    value: Value::Static(MANUFACTURER.as_bytes().to_vec()),
                    report: None,
                },
                CharSpec {
                    uuid: CHAR_PNP_ID,
                    properties: PROP_READ,
                    permissions: PERM_READ,
                    value: Value::Static(PNP_ID.to_vec()),
                    report: None,
                },
            ],
        },
        ServiceSpec {
            uuid: SERVICE_BATTERY,
            characteristics: vec![CharSpec {
                uuid: CHAR_BATTERY_LEVEL,
                properties: PROP_READ | PROP_NOTIFY,
                permissions: PERM_READ,
                value: Value::Dynamic(vec![100]),
                report: None,
            }],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn services() -> Vec<ServiceSpec> {
        super::services(database_hash(7))
    }

    fn hid_service() -> ServiceSpec {
        services()
            .into_iter()
            .find(|s| s.uuid == SERVICE_HID)
            .unwrap()
    }

    #[test]
    fn services_are_published_as_128_bit() {
        assert_eq!(sig128(0x1812), "00001812-0000-1000-8000-00805F9B34FB");
        for s in services() {
            assert_eq!(s.wire_uuid().len(), 36);
        }
    }

    #[test]
    fn service_changed_comes_first_then_hid() {
        let s = services();
        assert_eq!(s[0].uuid, SERVICE_GATT);
        assert_eq!(s[0].characteristics[0].uuid, CHAR_SERVICE_CHANGED);
        assert_eq!(s[0].characteristics[0].properties, PROP_INDICATE);
        assert_eq!(s[1].uuid, SERVICE_HID);
    }

    #[test]
    fn encrypted_characteristics_are_never_cached_values() {
        // CoreBluetooth answers cached values itself; encryption must be
        // enforced on every request, so encrypted ones are served live.
        for s in services() {
            for c in s.characteristics {
                if c.permissions & (PERM_READ_ENC | PERM_WRITE_ENC) != 0 {
                    assert!(!matches!(c.value, Value::Static(_)), "{:04X}", c.uuid);
                }
            }
        }
    }

    #[test]
    fn report_map_and_reports_require_encryption() {
        for c in hid_service().characteristics {
            let encrypted = c.permissions & (PERM_READ_ENC | PERM_WRITE_ENC) != 0;
            let plain = c.permissions & (PERM_READ | PERM_WRITE) != 0;
            match c.uuid {
                CHAR_REPORT_MAP | CHAR_REPORT | CHAR_CONTROL_POINT => {
                    assert!(encrypted && !plain, "{:04X} must be encrypted only", c.uuid)
                }
                // The phone writes Protocol Mode before pairing.
                CHAR_PROTOCOL_MODE => assert!(plain && !encrypted),
                _ => {}
            }
        }
    }

    #[test]
    fn every_report_has_a_reference_and_the_right_length() {
        let reports: Vec<_> = hid_service()
            .characteristics
            .into_iter()
            .filter(|c| c.uuid == CHAR_REPORT)
            .collect();
        let inputs: Vec<u8> = reports
            .iter()
            .filter_map(|c| match c.report {
                Some((id, REPORT_TYPE_INPUT)) => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(
            inputs,
            vec![
                hid::RID_KEYBOARD,
                hid::RID_ABSOLUTE,
                hid::RID_RELATIVE,
                hid::RID_CONSUMER
            ]
        );
        for c in &reports {
            let (id, kind) = c.report.expect("every Report needs a Report Reference");
            if kind == REPORT_TYPE_INPUT {
                assert_eq!(c.value, Value::Dynamic(vec![0; hid::report_len(id)]));
                assert!(c.properties & PROP_NOTIFY != 0);
            }
        }
    }

    #[test]
    fn static_values_are_read_only() {
        // CoreBluetooth caches a static value only on a read-only characteristic.
        for s in services() {
            for c in s.characteristics {
                if let Value::Static(_) = c.value {
                    assert_eq!(c.properties, PROP_READ, "{:04X}", c.uuid);
                }
            }
        }
    }

    #[test]
    fn the_database_hash_is_served_live_and_changes_with_the_seed() {
        let gatt = &services()[0];
        let hash = gatt
            .characteristics
            .iter()
            .find(|c| c.uuid == CHAR_DATABASE_HASH)
            .expect("Generic Attribute carries a Database Hash");
        assert_eq!(hash.properties, PROP_READ);
        assert_eq!(hash.value, Value::Dynamic(database_hash(7).to_vec()));
        assert_ne!(database_hash(7), database_hash(8));
        assert_ne!(database_hash(1 << 64), database_hash(2 << 64));
    }

    #[test]
    fn without_the_hash_only_service_changed_is_left() {
        let gatt = generic_attribute(None);
        assert_eq!(gatt.characteristics.len(), 1);
        assert_eq!(gatt.characteristics[0].uuid, CHAR_SERVICE_CHANGED);
    }

    #[test]
    fn advertised_name_fits_the_31_byte_packet() {
        // flags (3) + 16-bit service list (4) + name header (2) + name
        assert!(3 + 4 + 2 + ADVERTISED_NAME.len() <= 31);
    }
}
