//! SubMagician core: everything that does not need a window.
//!
//! Scanning folders, identifying videos (hash + file name), asking subtitle providers,
//! ranking what they return, and saving the chosen subtitle next to the video as UTF-8.

pub mod applog;
pub mod archive;
pub mod audio;
pub mod autosync;
pub mod cache;
pub mod engine;
pub mod error;
pub mod hash;
pub mod integration;
pub mod jobs;
pub mod lang;
pub mod media;
pub mod name;
mod net;
pub mod output;
pub mod packaging;
pub mod players;
pub mod probe;
pub mod provider;
pub mod report;
pub mod score;
pub mod settings;
#[cfg(feature = "whisper")]
pub mod speech;
pub mod sync;
pub mod text;
pub mod timing;
pub mod tools;
pub mod update;
pub mod watch;
pub mod whatsnew;

pub use error::{Error, Result};

/// This build's version ("0.1.0").
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Releases, issues and the update check.
pub const APP_REPO: &str = "halilkhrmn/submagician";
/// Where "Report a problem" e-mails go.
pub const SUPPORT_EMAIL: &str = "halilkahraman@yandex.com";
