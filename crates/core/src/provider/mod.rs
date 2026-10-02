//! Subtitle sources. Each one sits behind [`Provider`], so a broken site only takes out itself
//! and adding a site means adding one file here.

use std::future::Future;
use std::pin::Pin;

use crate::Result;
use crate::archive::SubtitleFile;
use crate::name::ParsedName;

pub mod gestdown;
pub mod opensubtitles;
pub mod subdl;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What we know about the video we want subtitles for.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub file_name: String,
    pub size: u64,
    /// OpenSubtitles hash, when the file is big enough to have one.
    pub hash: Option<String>,
    pub name: ParsedName,
    /// Wanted language codes (see [`crate::lang`]), most wanted first.
    pub languages: Vec<String>,
}

/// One subtitle a provider offers.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    /// [`Provider::name`] of the provider that offers it.
    pub provider: String,
    /// Provider-specific id used to download it.
    pub id: String,
    pub language: String,
    /// Release name the uploader gave ("Inception.2010.1080p.BluRay.x264-SPARKS").
    pub release: String,
    pub file_name: Option<String>,
    /// The provider says this subtitle was made for exactly this file (hash match).
    pub hash_match: bool,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub fps: Option<f32>,
    pub downloads: u64,
    pub rating: f32,
    pub hearing_impaired: bool,
    pub machine_translated: bool,
    pub trusted: bool,
    pub uploader: Option<String>,
    /// Filled by [`crate::score`]; higher is better.
    pub score: i32,
}

#[derive(Debug, Clone)]
pub struct Downloaded {
    pub files: Vec<SubtitleFile>,
    /// Downloads left today, when the provider says.
    pub remaining: Option<i64>,
}

pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    fn search<'a>(&'a self, query: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<Candidate>>>;
    fn download<'a>(&'a self, candidate: &'a Candidate) -> BoxFuture<'a, Result<Downloaded>>;
}

pub fn user_agent() -> String {
    format!("SubMagician v{}", env!("CARGO_PKG_VERSION"))
}
