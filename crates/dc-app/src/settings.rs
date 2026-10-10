//! Settings kept between runs (theme, which dock tab is open). A small JSON file in the user's
//! config folder; a missing or damaged file means the defaults, never an error.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    pub show_activity: bool,
    pub nav: crate::navigation::NavPreset,
}

fn file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))?;
    Some(base.join("designcomputer").join("settings.json"))
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn load() -> Settings {
        file()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| Settings::parse(&t))
            .unwrap_or_default()
    }

    /// Writes the settings; failure is ignored (they are a convenience).
    pub fn save(&self) {
        let Some(p) = file() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let tmp = p.with_extension("tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_or_partial_files_mean_defaults_and_unknown_keys_are_ignored() {
        assert_eq!(Settings::parse("not json"), Settings::default());
        assert_eq!(Settings::parse("{}"), Settings::default());
        let s = Settings::parse(r#"{"dark": true, "future": 1}"#);
        assert!(s.dark && !s.show_activity);
        let back = Settings::parse(&serde_json::to_string(&s).unwrap());
        assert_eq!(back, s);
    }
}
