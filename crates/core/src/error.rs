use std::io;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{provider}: {message} (HTTP {status})")]
    Api { provider: &'static str, status: u16, message: String },
    #[error("{provider}: download limit reached: {message}")]
    Quota { provider: &'static str, message: String },
    #[error("{provider}: login failed: {message}")]
    Auth { provider: &'static str, message: String },
    #[error("{provider}: not configured: {message}")]
    NotConfigured { provider: &'static str, message: String },
    #[error("unexpected response: {0}")]
    Parse(String),
    #[error("archive error: {0}")]
    Archive(String),
    #[error("no subtitle file inside the download")]
    NoSubtitleInArchive,
    #[error("file is too small to hash ({0} bytes)")]
    TooSmallToHash(u64),
}
