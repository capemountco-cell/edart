//! The messages exchanged with the Mac's own CoreDevice service
//! (`com.apple.CoreDevice.CoreDeviceService`, part of Xcode), which holds
//! the connection to the phone and hands out a connected socket per service.
//!
//! Edart asks it the way `devicectl` does, instead of opening its own
//! tunnel: a second tunnel peer displaces macOS's, and Xcode then loses the
//! phone until it is replugged. The sequence is check in, take a usage
//! assertion, then ask for a socket.
//!
//! Message shapes are from iphone-use's `CoreDeviceClient.swift` (MIT,
//! Sunghyun Kim), verified there with Xcode 27 and iOS 27.3. They are the
//! "action" dialect; the CoreDevice shipped with Xcode 15 spoke an older
//! "feature" dialect that this does not send.

use super::xpc::{uuid_string, XpcValue};

/// Where Xcode installs the framework that contains the service.
pub const FRAMEWORK_DIR: &str = "/Library/Developer/PrivateFrameworks/CoreDevice.framework";
/// The Mach service name inside it.
pub const SERVICE_NAME: &str = "com.apple.CoreDevice.CoreDeviceService";

pub const ACTION_ACQUIRE_ASSERTION: &str = "com.apple.coredevice.action.acquireusageassertion";
pub const ACTION_CREATE_SOCKET: &str = "com.apple.coredevice.action.createservicesocket";

/// The version sent when the installed framework's cannot be read: the one
/// `devicectl` used when the reference was written.
pub const FALLBACK_VERSION: &str = "629.3";

/// The first message: introduce this client. The answer comes back as an
/// event, not as the reply.
pub fn check_in_request(request_id: [u8; 16]) -> XpcValue {
    XpcValue::dict([
        (
            "mangledTypeName",
            XpcValue::string("CoreDevice.DeviceManagerCheckInRequest"),
        ),
        (
            "value",
            XpcValue::dict([("identifier", XpcValue::Uuid(request_id))]),
        ),
    ])
}

/// The service checks its client's version, so the installed framework's
/// own version is sent, as five numbers plus the original text.
pub fn version_value(version: &str) -> XpcValue {
    let parts: Vec<u64> = version.split('.').filter_map(|p| p.parse().ok()).collect();
    let components = (0..5)
        .map(|i| XpcValue::Uint64(parts.get(i).copied().unwrap_or(0)))
        .collect();
    XpcValue::dict([
        (
            "originalComponentsCount",
            XpcValue::Int64(parts.len() as i64),
        ),
        ("components", XpcValue::Array(components)),
        ("stringValue", XpcValue::string(version)),
    ])
}

/// One action for one device.
pub fn action_request(
    action: &str,
    device_identifier: &str,
    version: &str,
    invocation_id: [u8; 16],
    input: XpcValue,
) -> XpcValue {
    XpcValue::dict([
        ("CoreDevice.coreDeviceVersion", version_value(version)),
        (
            "CoreDevice.deviceIdentifier",
            XpcValue::string(device_identifier),
        ),
        (
            "CoreDevice.CoreDeviceDDIProtocolVersion",
            XpcValue::Int64(2),
        ),
        (
            "CoreDevice.invocationIdentifier",
            XpcValue::string(uuid_string(&invocation_id)),
        ),
        ("CoreDevice.input", input),
        ("CoreDevice.actionIdentifier", XpcValue::string(action)),
    ])
}

/// Input of the usage assertion: keep a trusted connection to the phone up,
/// and keep the phone awake, for as long as the assertion lives. The native
/// side adds the `endpoint` the fulfilment is announced on.
pub fn assertion_input() -> XpcValue {
    let state = |id: &str| XpcValue::dict([("identifier", XpcValue::string(id))]);
    XpcValue::dict([
        ("reason", XpcValue::string("Edart")),
        (
            "requirements",
            XpcValue::dict([
                (
                    "deviceStates",
                    XpcValue::Array(vec![
                        state("com.apple.coredevice.remoteServiceDiscoveryTrustedConnectivityAvailable"),
                        state("com.apple.coredevice.powerAssertionTaken"),
                    ]),
                ),
                ("capabilities", XpcValue::Array(vec![])),
            ]),
        ),
    ])
}

/// Input of the socket action.
pub fn socket_input(service: &str) -> XpcValue {
    XpcValue::dict([("serviceName", XpcValue::string(service))])
}

/// Events arrive wrapped (`value.request`); replies do not.
fn unwrap_event(message: &XpcValue) -> &XpcValue {
    message
        .get("value")
        .and_then(|v| v.get("request"))
        .unwrap_or(message)
}

fn type_name(message: &XpcValue) -> Option<&str> {
    message.get("mangledTypeName").and_then(XpcValue::as_str)
}

/// The device snapshots of a check-in-complete event, or `None` if the
/// message is something else.
pub fn check_in_snapshots(message: &XpcValue) -> Option<&[XpcValue]> {
    let message = unwrap_event(message);
    if type_name(message)? != "CoreDevice.DeviceManagerCheckInCompleteEvent" {
        return None;
    }
    message
        .get("value")?
        .get("initialDeviceSnapshots")?
        .as_array()
}

/// Whether `value` mentions the phone's UDID anywhere: either as a plain
/// `udid` string or as a `{descriptorIdentifier: {key: "udid"}, value}`
/// property, which is how newer versions nest it.
fn mentions_udid(value: &XpcValue, udid: &str) -> bool {
    match value {
        XpcValue::Dictionary(entries) => {
            let plain = entries
                .get("udid")
                .and_then(XpcValue::as_str)
                .is_some_and(|v| v.eq_ignore_ascii_case(udid));
            let described = entries
                .get("descriptorIdentifier")
                .and_then(|d| d.get("key"))
                .and_then(XpcValue::as_str)
                == Some("udid")
                && entries
                    .get("value")
                    .and_then(XpcValue::as_str)
                    .is_some_and(|v| v.eq_ignore_ascii_case(udid));
            plain || described || entries.values().any(|v| mentions_udid(v, udid))
        }
        XpcValue::Array(items) => items.iter().any(|v| mentions_udid(v, udid)),
        _ => false,
    }
}

/// CoreDevice's own identifier (not the UDID) for the phone with `udid`.
/// `None` if CoreDevice does not list that phone.
pub fn device_identifier(snapshots: &[XpcValue], udid: &str) -> Option<String> {
    snapshots.iter().find_map(|snapshot| {
        if !mentions_udid(snapshot, udid) {
            return None;
        }
        match snapshot.get("deviceInfo")?.get("identifier")? {
            XpcValue::Uuid(bytes) => Some(uuid_string(bytes)),
            _ => None,
        }
    })
}

/// Whether a message on the assertion's endpoint says the assertion took
/// hold. (The action's own reply only arrives when the assertion ends.)
pub fn is_assertion_fulfilled(message: &XpcValue) -> bool {
    type_name(unwrap_event(message)).is_some_and(|t| t.ends_with("AssertionFulfilledMessage"))
}

/// The error in a reply, as text. `None` for a success.
pub fn reply_error(reply: &XpcValue) -> Option<String> {
    let error = reply.get("CoreDevice.error")?;
    let text = error
        .get("userInfo")
        .and_then(|i| i.get("NSLocalizedDescription"))
        .and_then(XpcValue::as_str)
        .unwrap_or("unknown error");
    Some(text.to_string())
}

/// CoreDevice reports the socket action as "not supported" by the device
/// when the Developer Disk Image, which holds the HID services, is not
/// mounted (for example after the phone restarts).
pub fn means_disk_image_missing(error: &str) -> bool {
    error.contains("not supported")
}

/// `CFBundleVersion` from an XML Info.plist. `None` if it is not there (or
/// the file is a binary plist, which the native side reads another way).
pub fn bundle_version(info_plist: &str) -> Option<String> {
    let after = info_plist.split("<key>CFBundleVersion</key>").nth(1)?;
    let start = after.find("<string>")? + "<string>".len();
    let end = after[start..].find("</string>")? + start;
    let version = after[start..end].trim();
    let plausible = !version.is_empty()
        && version.len() <= 32
        && version.chars().all(|c| c.is_ascii_digit() || c == '.');
    plausible.then(|| version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const UDID: &str = "00008140-001A2B3C4D5E6F70";
    const DEVICE: [u8; 16] = [0x11; 16];

    fn snapshot(udid_property: XpcValue) -> XpcValue {
        XpcValue::dict([
            (
                "deviceInfo",
                XpcValue::dict([("identifier", XpcValue::Uuid(DEVICE))]),
            ),
            ("properties", XpcValue::Array(vec![udid_property])),
        ])
    }

    fn described(udid: &str) -> XpcValue {
        XpcValue::dict([
            (
                "descriptorIdentifier",
                XpcValue::dict([("key", XpcValue::string("udid"))]),
            ),
            ("value", XpcValue::string(udid)),
        ])
    }

    fn check_in_event(snapshots: Vec<XpcValue>) -> XpcValue {
        let complete = XpcValue::dict([
            (
                "mangledTypeName",
                XpcValue::string("CoreDevice.DeviceManagerCheckInCompleteEvent"),
            ),
            (
                "value",
                XpcValue::dict([("initialDeviceSnapshots", XpcValue::Array(snapshots))]),
            ),
        ]);
        // Events arrive wrapped in a service event.
        XpcValue::dict([
            (
                "mangledTypeName",
                XpcValue::string("CoreDevice.ServiceEvent"),
            ),
            ("value", XpcValue::dict([("request", complete)])),
        ])
    }

    #[test]
    fn check_in_carries_the_request_id() {
        let m = check_in_request([7; 16]);
        assert_eq!(
            m.get("mangledTypeName").unwrap().as_str(),
            Some("CoreDevice.DeviceManagerCheckInRequest")
        );
        assert_eq!(
            m.get("value").unwrap().get("identifier"),
            Some(&XpcValue::Uuid([7; 16]))
        );
    }

    #[test]
    fn the_version_is_five_numbers_and_the_text() {
        let v = version_value("629.3");
        assert_eq!(v.get("originalComponentsCount"), Some(&XpcValue::Int64(2)));
        assert_eq!(v.get("stringValue").unwrap().as_str(), Some("629.3"));
        assert_eq!(
            v.get("components"),
            Some(&XpcValue::Array(vec![
                XpcValue::Uint64(629),
                XpcValue::Uint64(3),
                XpcValue::Uint64(0),
                XpcValue::Uint64(0),
                XpcValue::Uint64(0),
            ]))
        );
    }

    #[test]
    fn an_action_names_the_device_and_itself() {
        let m = action_request(
            ACTION_CREATE_SOCKET,
            "DEVICE-ID",
            "700.1",
            [0; 16],
            socket_input("some.service"),
        );
        assert_eq!(
            m.get("CoreDevice.actionIdentifier").unwrap().as_str(),
            Some("com.apple.coredevice.action.createservicesocket")
        );
        assert_eq!(
            m.get("CoreDevice.deviceIdentifier").unwrap().as_str(),
            Some("DEVICE-ID")
        );
        assert_eq!(
            m.get("CoreDevice.CoreDeviceDDIProtocolVersion"),
            Some(&XpcValue::Int64(2))
        );
        assert_eq!(
            m.get("CoreDevice.invocationIdentifier").unwrap().as_str(),
            Some("00000000-0000-0000-0000-000000000000")
        );
        assert_eq!(
            m.get("CoreDevice.input")
                .unwrap()
                .get("serviceName")
                .unwrap()
                .as_str(),
            Some("some.service")
        );
        // The older dialect's key must not be sent.
        assert_eq!(m.get("CoreDevice.featureIdentifier"), None);
    }

    #[test]
    fn the_assertion_asks_for_connectivity_and_power() {
        let input = assertion_input();
        let states = input
            .get("requirements")
            .unwrap()
            .get("deviceStates")
            .unwrap()
            .as_array()
            .unwrap();
        let ids: Vec<&str> = states
            .iter()
            .map(|s| s.get("identifier").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "com.apple.coredevice.remoteServiceDiscoveryTrustedConnectivityAvailable",
                "com.apple.coredevice.powerAssertionTaken"
            ]
        );
    }

    #[test]
    fn the_phone_is_found_by_udid_in_either_shape() {
        let event = check_in_event(vec![
            snapshot(described("SOMEONE-ELSE")),
            snapshot(described(&UDID.to_lowercase())),
        ]);
        let snapshots = check_in_snapshots(&event).unwrap();
        // The first snapshot has the same identifier here; what matters is
        // that a snapshot without the UDID is skipped.
        assert_eq!(
            device_identifier(snapshots, UDID).as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(device_identifier(&snapshots[..1], UDID), None);

        let plain = XpcValue::dict([(
            "deviceInfo",
            XpcValue::dict([
                ("identifier", XpcValue::Uuid(DEVICE)),
                ("udid", XpcValue::string(UDID)),
            ]),
        )]);
        assert!(device_identifier(&[plain], UDID).is_some());
    }

    #[test]
    fn other_events_are_not_a_check_in() {
        let other = XpcValue::dict([("mangledTypeName", XpcValue::string("CoreDevice.Other"))]);
        assert_eq!(check_in_snapshots(&other), None);
        assert_eq!(check_in_snapshots(&XpcValue::Null), None);
        assert_eq!(check_in_snapshots(&check_in_event(vec![])), Some(&[][..]));
    }

    #[test]
    fn fulfilment_is_recognised_under_both_type_names() {
        for name in [
            "CoreDevice.AssertionFulfilledMessage",
            "CoreDeviceService.AssertionFulfilledMessage",
        ] {
            let m = XpcValue::dict([("mangledTypeName", XpcValue::string(name))]);
            assert!(is_assertion_fulfilled(&m));
        }
        assert!(!is_assertion_fulfilled(&XpcValue::dict::<&str>([])));
    }

    #[test]
    fn errors_come_out_as_their_description() {
        let reply = XpcValue::dict([(
            "CoreDevice.error",
            XpcValue::dict([(
                "userInfo",
                XpcValue::dict([(
                    "NSLocalizedDescription",
                    XpcValue::string(
                        "The capability “Create Service Socket” is not supported by this device.",
                    ),
                )]),
            )]),
        )]);
        let text = reply_error(&reply).unwrap();
        assert!(means_disk_image_missing(&text));
        let bare = XpcValue::dict([("CoreDevice.error", XpcValue::dict::<&str>([]))]);
        assert_eq!(reply_error(&bare).as_deref(), Some("unknown error"));
        let ok = XpcValue::dict([("CoreDevice.output", XpcValue::dict::<&str>([]))]);
        assert_eq!(reply_error(&ok), None);
        assert!(!means_disk_image_missing(
            "A connection could not be established"
        ));
    }

    #[test]
    fn the_framework_version_is_read_from_its_info_plist() {
        let plist = "<dict>\n<key>CFBundleName</key>\n<string>CoreDevice</string>\n\
                     <key>CFBundleVersion</key>\n\t<string>701.12.3</string>\n</dict>";
        assert_eq!(bundle_version(plist).as_deref(), Some("701.12.3"));
        assert_eq!(bundle_version("bplist00\u{1}"), None);
        assert_eq!(
            bundle_version("<key>CFBundleVersion</key><string>1; rm</string>"),
            None
        );
    }
}
