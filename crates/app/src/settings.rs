//! Settings stored as JSON in the user's config folder.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Wanted subtitle languages, most wanted first ("tr, en").
    pub languages: String,
    pub recursive: bool,
    pub skip_existing: bool,
    /// "auto", "en" or "tr".
    pub ui_language: String,
    pub last_folder: Option<PathBuf>,
    pub opensubtitles_username: String,
    // TODO(phase 5): move to the OS keyring.
    pub opensubtitles_password: String,
    pub opensubtitles_api_key: String,
    /// Sync each downloaded subtitle to the video's audio.
    pub auto_sync: bool,
    /// ffmpeg to use; empty means look next to the app and on PATH.
    pub ffmpeg_path: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            languages: default_languages(),
            recursive: true,
            skip_existing: true,
            ui_language: "auto".into(),
            last_folder: None,
            opensubtitles_username: String::new(),
            opensubtitles_password: String::new(),
            opensubtitles_api_key: String::new(),
            auto_sync: true,
            ffmpeg_path: String::new(),
        }
    }
}

/// Turkish first when the system is Turkish, then English.
fn default_languages() -> String {
    match system_language().as_deref() {
        Some("en") | None => "en".into(),
        Some(code) if submagician_core::lang::find(code).is_some() => format!("{code}, en"),
        _ => "en".into(),
    }
}

/// Two-letter language of the OS locale ("tr" for "tr-TR").
pub fn system_language() -> Option<String> {
    let locale = sys_locale::get_locale()?;
    Some(locale.split(['-', '_']).next()?.to_ascii_lowercase())
}

impl Settings {
    fn path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "SubMagician").map(|d| d.config_dir().join("settings.json"))
    }

    pub fn load() -> Settings {
        Self::path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).map_err(|e| log::warn!("bad settings file: {e}")).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::path() else { return Ok(()) };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, serde_json::to_vec_pretty(self).expect("settings serialize"))
    }

    /// Parsed language codes; English when the list has nothing usable.
    pub fn language_codes(&self) -> Vec<String> {
        let codes = submagician_core::lang::parse_list(&self.languages);
        if codes.is_empty() { vec!["en".into()] } else { codes.into_iter().map(String::from).collect() }
    }

    /// The UI translation to select: "" for English.
    pub fn ui_translation(&self) -> &'static str {
        let lang = match self.ui_language.as_str() {
            "auto" => system_language().unwrap_or_default(),
            other => other.to_owned(),
        };
        if lang == "tr" { "tr" } else { "" }
    }
}
