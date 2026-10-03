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
    #[error("ffmpeg was not found")]
    NoFfmpeg,
    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),
    #[error("no speech found in the audio")]
    NoSpeech,
    #[error("cancelled")]
    Cancelled,
    /// The worker process died without an answer; `cpu` when the processor lacked an
    /// instruction it was built for.
    #[error("{}", if *cpu { "the processor does not support an instruction this needs" } else { "the background process crashed" })]
    Crashed { cpu: bool, detail: String },
    /// A failure reported by the worker process, as text.
    #[error("{0}")]
    Other(String),
}
