//! Settings stored as JSON in the user's config folder, shared by the app and the CLI.

use std::fs;
use std::path::PathBuf;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::cache::SearchCache;
use crate::engine::Engine;
use crate::provider::Provider;
use crate::provider::gestdown::Gestdown;
use crate::provider::opensubtitles::{Credentials, OpenSubtitles};
use crate::provider::subdl::SubDl;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Wanted subtitle languages, most wanted first ("tr, en").
    pub languages: String,
    pub recursive: bool,
    pub skip_existing: bool,
    pub last_folder: Option<PathBuf>,
    pub opensubtitles_username: String,
    // TODO(phase 5): move to the OS keyring.
    pub opensubtitles_password: String,
    pub opensubtitles_api_key: String,
    /// Sync each downloaded subtitle to the video's audio.
    pub auto_sync: bool,
    /// ffmpeg to use; empty means look next to the app and on PATH.
    pub ffmpeg_path: String,
    pub use_opensubtitles: bool,
    pub use_subdl: bool,
    pub use_addic7ed: bool,
    /// Overrides the built-in SubDL key.
    pub subdl_api_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            languages: default_languages(),
            recursive: true,
            skip_existing: true,
            last_folder: None,
            opensubtitles_username: String::new(),
            opensubtitles_password: String::new(),
            opensubtitles_api_key: String::new(),
            auto_sync: true,
            ffmpeg_path: String::new(),
            use_opensubtitles: true,
            use_subdl: true,
            use_addic7ed: true,
            subdl_api_key: String::new(),
        }
    }
}

/// Turkish first when the system is Turkish, then English.
fn default_languages() -> String {
    match system_language().as_deref() {
        Some("en") | None => "en".into(),
        Some(code) if crate::lang::find(code).is_some() => format!("{code}, en"),
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

    /// Folder for cached search results.
    pub fn search_cache_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "SubMagician").map(|d| d.cache_dir().join("search"))
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
        let codes = crate::lang::parse_list(&self.languages);
        if codes.is_empty() { vec!["en".into()] } else { codes.into_iter().map(String::from).collect() }
    }

    /// An engine with the providers switched on here (SubDL only when it has a key) and the
    /// search cache.
    pub fn engine(&self) -> Engine {
        let mut providers: Vec<Arc<dyn Provider>> = Vec::new();
        if self.use_opensubtitles {
            let credentials = Credentials {
                username: self.opensubtitles_username.clone(),
                password: self.opensubtitles_password.clone(),
            };
            providers.push(Arc::new(OpenSubtitles::new(Some(self.opensubtitles_api_key.clone()), Some(credentials))));
        }
        if self.use_subdl
            && let Some(subdl) = SubDl::new(Some(self.subdl_api_key.clone()))
        {
            providers.push(Arc::new(subdl));
        }
        if self.use_addic7ed {
            providers.push(Arc::new(Gestdown::new()));
        }
        let mut engine = Engine::new(providers);
        if let Some(dir) = Self::search_cache_dir() {
            engine = engine.with_cache(SearchCache::new(dir));
        }
        engine
    }
}
