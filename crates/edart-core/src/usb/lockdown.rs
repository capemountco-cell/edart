//! Lockdown service messages: a 4-byte big-endian length, then a property
//! list. Used for the one lockdown service Edart talks to itself,
//! `com.apple.amfi.lockdown`, which controls Developer Mode.
//!
//! The actions are the ones Xcode sends (documented by pymobiledevice3's
//! `amfi` service): 0 makes the Developer Mode switch appear in Settings,
//! 1 turns it on and restarts the phone, 2 confirms it after the restart.

/// The lockdown service that controls Developer Mode.
pub const AMFI_SERVICE: &str = "com.apple.amfi.lockdown";
/// Lockdown domain and key that say whether Developer Mode is on.
pub const AMFI_DOMAIN: &str = "com.apple.security.mac.amfi";
pub const DEVELOPER_MODE_KEY: &str = "DeveloperModeStatus";

/// Largest reply accepted; real ones are a few hundred bytes.
pub const MAX_MESSAGE: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmfiAction {
    /// Show the Developer Mode switch in Settings › Privacy & Security.
    Reveal = 0,
    /// Turn Developer Mode on. The phone restarts. Refused when the phone
    /// has a passcode: then only the switch in Settings can do it.
    Arm = 1,
    /// Answer "Turn On" to the question the phone asks after restarting.
    Accept = 2,
}

/// The request for an action, framed and ready to send.
pub fn amfi_request(action: AmfiAction) -> Vec<u8> {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\t<key>action</key>\n\t<integer>{}</integer>\n</dict>\n</plist>\n",
        action as u8
    );
    frame(xml.as_bytes())
}

/// Prefix a property list with its length.
pub fn frame(plist: &[u8]) -> Vec<u8> {
    let mut out = (plist.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(plist);
    out
}

/// The length a reply's 4-byte header announces, refused if absurd.
pub fn reply_len(header: [u8; 4]) -> Result<usize, String> {
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > MAX_MESSAGE {
        Err(format!("the iPhone sent a reply of {len} bytes"))
    } else {
        Ok(len)
    }
}

/// The two fields of the service's reply that matter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AmfiReply {
    pub success: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmfiError {
    /// The phone has a passcode, so it will not turn Developer Mode on for
    /// a computer: the person has to use the switch in Settings.
    PasscodeSet,
    Refused(String),
}

impl std::fmt::Display for AmfiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AmfiError::PasscodeSet => write!(f, "the iPhone has a passcode"),
            AmfiError::Refused(why) => write!(f, "the iPhone refused: {why}"),
        }
    }
}

pub fn amfi_result(reply: &AmfiReply) -> Result<(), AmfiError> {
    if let Some(error) = &reply.error {
        return Err(if error.to_ascii_lowercase().contains("passcode") {
            AmfiError::PasscodeSet
        } else {
            AmfiError::Refused(error.clone())
        });
    }
    if reply.success == Some(true) {
        Ok(())
    } else {
        Err(AmfiError::Refused("no reason given".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_a_length_and_an_xml_plist() {
        let bytes = amfi_request(AmfiAction::Arm);
        let len = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(len, bytes.len() - 4);
        let xml = std::str::from_utf8(&bytes[4..]).unwrap();
        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("<key>action</key>\n\t<integer>1</integer>"));
        assert!(xml.trim_end().ends_with("</plist>"));
        let reveal = amfi_request(AmfiAction::Reveal);
        assert!(std::str::from_utf8(&reveal[4..])
            .unwrap()
            .contains("<integer>0</integer>"));
        let accept = amfi_request(AmfiAction::Accept);
        assert!(std::str::from_utf8(&accept[4..])
            .unwrap()
            .contains("<integer>2</integer>"));
    }

    #[test]
    fn reply_lengths_are_bounded() {
        assert_eq!(reply_len([0, 0, 1, 0]), Ok(256));
        assert!(reply_len([0, 0, 0, 0]).is_err());
        assert!(reply_len([0xFF, 0xFF, 0xFF, 0xFF]).is_err());
    }

    #[test]
    fn a_passcode_refusal_is_told_apart() {
        let ok = AmfiReply {
            success: Some(true),
            error: None,
        };
        assert_eq!(amfi_result(&ok), Ok(()));
        let passcode = AmfiReply {
            success: None,
            error: Some("Device has a passcode set".into()),
        };
        assert_eq!(amfi_result(&passcode), Err(AmfiError::PasscodeSet));
        let other = AmfiReply {
            success: None,
            error: Some("nope".into()),
        };
        assert_eq!(amfi_result(&other), Err(AmfiError::Refused("nope".into())));
        assert!(amfi_result(&AmfiReply::default()).is_err());
        let failed = AmfiReply {
            success: Some(false),
            error: None,
        };
        assert!(amfi_result(&failed).is_err());
    }
}
