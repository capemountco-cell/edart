//! The app's settings file: the AI token and, for testing, another API
//! address.
//!
//! The token is a secret, so the file is written readable by the user
//! only, and the UI is never handed the token back: it learns whether one
//! is saved.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Where the agent asks for its next step unless the settings say otherwise.
pub const DEFAULT_API_BASE: &str = "https://edart-api.lovable.app";

/// What is stored on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Overrides [`DEFAULT_API_BASE`]. No trailing slash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
}

/// What the UI may see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub token_set: bool,
    pub api_base: String,
    /// The address is the built-in one.
    pub api_base_is_default: bool,
    /// Where the file is, to show the user.
    pub path: String,
}

/// A change from the UI. A field left out is left alone; an empty string
/// clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsChange {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
}

impl Settings {
    pub fn api_base(&self) -> &str {
        self.api_base.as_deref().unwrap_or(DEFAULT_API_BASE)
    }

    pub fn step_url(&self) -> String {
        format!("{}/api/agent/step", self.api_base())
    }

    pub fn health_url(&self) -> String {
        format!("{}/api/health", self.api_base())
    }

    pub fn view(&self, path: &Path) -> SettingsView {
        SettingsView {
            token_set: self.token.is_some(),
            api_base: self.api_base().to_string(),
            api_base_is_default: self.api_base.is_none(),
            path: path.display().to_string(),
        }
    }

    /// Apply a change, or say why it is not acceptable. Nothing changes on
    /// an error.
    pub fn apply(&mut self, change: SettingsChange) -> Result<(), String> {
        let token = change.token.map(|t| clean_token(&t)).transpose()?;
        let api_base = change.api_base.map(|b| clean_api_base(&b)).transpose()?;
        if let Some(token) = token {
            self.token = token;
        }
        if let Some(api_base) = api_base {
            self.api_base = api_base;
        }
        Ok(())
    }
}

/// A token goes into an HTTP header, so it must be one line of printable
/// ASCII. Empty means "forget the token".
fn clean_token(token: &str) -> Result<Option<String>, String> {
    let token = token.trim();
    if token.is_empty() {
        return Ok(None);
    }
    if token.len() > 512 {
        return Err("That token is too long.".into());
    }
    if !token.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err("A token can’t contain spaces or special characters.".into());
    }
    Ok(Some(token.to_string()))
}

/// The API address must be HTTPS, because the token and screenshots travel
/// to it; plain HTTP is allowed only to this Mac, for testing. Empty means
/// "use the built-in address".
fn clean_api_base(base: &str) -> Result<Option<String>, String> {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() || base == DEFAULT_API_BASE {
        return Ok(None);
    }
    if base.len() > 300 || base.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("That address isn’t valid.".into());
    }
    let host_and_path = if let Some(rest) = base.strip_prefix("https://") {
        rest
    } else if let Some(rest) = base.strip_prefix("http://") {
        let host = rest.split(['/', ':']).next().unwrap_or("");
        if host != "localhost" && host != "127.0.0.1" {
            return Err(
                "The address must start with https://. Plain http:// only works for localhost."
                    .into(),
            );
        }
        rest
    } else {
        return Err("The address must start with https://.".into());
    };
    let host = host_and_path.split('/').next().unwrap_or("");
    if host.is_empty() || host.contains('@') || base.contains(['?', '#']) {
        return Err("That address isn’t valid.".into());
    }
    Ok(Some(base.to_string()))
}

/// Read the settings. A missing file is the defaults; an unreadable or
/// damaged one is an error, so a save does not silently wipe a token.
pub fn load(path: &Path) -> Result<Settings, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("The settings file is damaged ({e}). Save the token again.")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(format!("Couldn’t read the settings file: {e}")),
    }
}

/// Write the settings, readable and writable by the user only. Written to
/// a temporary file first, so a crash cannot leave half a file.
pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        // An older temporary file keeps its old mode; set it again.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(token: Option<&str>, api_base: Option<&str>) -> SettingsChange {
        SettingsChange {
            token: token.map(str::to_string),
            api_base: api_base.map(str::to_string),
        }
    }

    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("edart-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("settings.json")
    }

    #[test]
    fn defaults_point_at_the_built_in_api() {
        let settings = Settings::default();
        assert_eq!(
            settings.step_url(),
            "https://edart-api.lovable.app/api/agent/step"
        );
        assert_eq!(
            settings.health_url(),
            "https://edart-api.lovable.app/api/health"
        );
        let view = settings.view(Path::new("/x/settings.json"));
        assert!(!view.token_set);
        assert!(view.api_base_is_default);
    }

    #[test]
    fn a_token_is_trimmed_saved_and_never_shown() {
        let mut settings = Settings::default();
        settings
            .apply(change(Some("  s3cret-Token_1  "), None))
            .unwrap();
        assert_eq!(settings.token.as_deref(), Some("s3cret-Token_1"));
        let view = serde_json::to_string(&settings.view(Path::new("/x"))).unwrap();
        assert!(view.contains("\"tokenSet\":true"));
        assert!(!view.contains("s3cret"));
        // Leaving the field out keeps it; an empty string clears it.
        settings.apply(change(None, None)).unwrap();
        assert!(settings.token.is_some());
        settings.apply(change(Some(""), None)).unwrap();
        assert_eq!(settings.token, None);
    }

    #[test]
    fn tokens_that_cannot_go_in_a_header_are_refused() {
        let mut settings = Settings::default();
        for bad in ["two words", "line\nbreak", "naïve", &"x".repeat(513)] {
            assert!(settings.apply(change(Some(bad), None)).is_err(), "{bad:?}");
        }
        assert_eq!(settings.token, None);
    }

    #[test]
    fn the_api_address_must_be_https_or_this_mac() {
        let mut settings = Settings::default();
        for good in [
            "https://example.com",
            "https://example.com/base/",
            "http://localhost:8787",
            "http://127.0.0.1:3000/x",
        ] {
            settings.apply(change(None, Some(good))).unwrap();
            assert_eq!(
                settings.api_base.as_deref(),
                Some(good.trim_end_matches('/'))
            );
        }
        for bad in [
            "http://example.com",
            "http://localhost.evil.com",
            "ftp://example.com",
            "example.com",
            "https://",
            "https://user@example.com",
            "https://example.com/?a=b",
            "https://exa mple.com",
        ] {
            assert!(settings.apply(change(None, Some(bad))).is_err(), "{bad}");
        }
        // Empty, or the built-in address, goes back to the default.
        settings.apply(change(None, Some(""))).unwrap();
        assert_eq!(settings.api_base, None);
        settings
            .apply(change(None, Some("https://edart-api.lovable.app/")))
            .unwrap();
        assert_eq!(settings.api_base, None);
    }

    #[test]
    fn a_bad_field_changes_nothing() {
        let mut settings = Settings::default();
        let result = settings.apply(change(Some("good-token"), Some("nope")));
        assert!(result.is_err());
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn settings_round_trip_through_a_private_file() {
        let path = temp_file("roundtrip");
        assert_eq!(load(&path).unwrap(), Settings::default());
        let settings = Settings {
            token: Some("abc".into()),
            api_base: Some("http://localhost:1".into()),
        };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Saving again over an existing file keeps it private.
        save(&path, &Settings::default()).unwrap();
        assert_eq!(load(&path).unwrap(), Settings::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_damaged_file_is_an_error_not_a_reset() {
        let path = temp_file("damaged");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        assert!(load(&path).unwrap_err().contains("damaged"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }
}
