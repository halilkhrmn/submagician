//! Syncing a subtitle file to its video's audio in seconds where possible:
//!
//! 1. Speech found earlier for the same file (kept on disk) is used again at once.
//! 2. A long video gets a quick look first: a few short windows across the film are decoded at
//!    the same time and one frame-rate ratio + shift is fitted to them. When every window agrees
//!    with that shift (no cut or added scenes), that is the answer.
//! 3. Otherwise the whole audio is read, in pieces decoded at the same time (one ffmpeg per CPU
//!    core), and alass aligns with splits. That speech is kept for next time.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use sha2::{Digest, Sha256};

use crate::sync::{self, Report, Span};
use crate::{Result, audio, output};

/// Videos shorter than this are read whole straight away.
const QUICK_MIN_MS: i64 = 12 * 60_000;
const WINDOWS: i64 = 10;
const WINDOW_MS: i64 = 40_000;
/// The quick fit is trusted only when this much of the subtitle time inside the windows is on
/// speech afterwards.
const QUICK_MIN_OVERLAP: f32 = 0.55;
/// …and only when this many lines fall inside the windows (sparse subtitles need the full read).
const QUICK_MIN_CUES: usize = 25;
/// Speech files kept in the cache.
const KEEP: usize = 300;

/// Which way the result was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Method {
    /// Speech of this video was already known.
    Cached,
    /// Windows across the video were enough.
    Quick,
    /// The whole audio was read.
    Full,
}

/// Syncs the subtitle at `subtitle` to the audio of `video` and rewrites it when the result fits
/// the speech better. `progress` gets 0.0..=1.0.
pub fn sync_to_audio(
    subtitle: &Path,
    video: &Path,
    ffmpeg: &Path,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(f32) + Send),
) -> Result<(Report, Method)> {
    let mut doc = sync::load(subtitle)?;
    if let Some(speech) = cached(video) {
        log::info!("speech of {} from the cache", video.display());
        return finish(subtitle, &mut doc, &speech, Method::Cached);
    }
    let duration = audio::duration_ms(ffmpeg, video)?;
    if let Some(total) = duration.filter(|&t| t >= QUICK_MIN_MS && !doc.cues.is_empty()) {
        let windows = plan_windows(total);
        let samples = audio::sample_speech(ffmpeg, video, &windows, cancel, &mut |p| progress(p * 0.25))?;
        let fit = sync::quick_fit(&doc, &windows, &samples);
        log::info!("quick look: {fit:?}");
        if fit.consistent && fit.overlap_after >= QUICK_MIN_OVERLAP && fit.cues_inside >= QUICK_MIN_CUES {
            let applied = fit.overlap_after >= fit.overlap_before + sync::MIN_GAIN;
            if applied {
                sync::apply_quick(&mut doc, &fit);
                output::rewrite(subtitle, &doc.render())?;
            }
            progress(1.0);
            let report = Report {
                offset_ms: if applied { fit.offset_ms } else { 0 },
                ratio: if applied { fit.ratio } else { 1.0 },
                splits: 0,
                overlap_before: fit.overlap_before,
                overlap_after: if applied { fit.overlap_after } else { fit.overlap_before },
                applied,
            };
            return Ok((report, Method::Quick));
        }
        let speech = audio::extract_speech(ffmpeg, video, cancel, &mut |p| progress(0.25 + p * 0.75))?;
        store(video, &speech);
        return finish(subtitle, &mut doc, &speech, Method::Full);
    }
    let speech = audio::extract_speech(ffmpeg, video, cancel, progress)?;
    store(video, &speech);
    finish(subtitle, &mut doc, &speech, Method::Full)
}

fn finish(
    subtitle: &Path,
    doc: &mut crate::timing::Document,
    speech: &[Span],
    method: Method,
) -> Result<(Report, Method)> {
    let report = sync::align(doc, speech);
    if report.applied {
        output::rewrite(subtitle, &doc.render())?;
    }
    Ok((report, method))
}

/// `WINDOWS` windows of `WINDOW_MS`, spread over the video between 3 % and 97 % (titles and
/// credits have little speech).
fn plan_windows(total: i64) -> Vec<(i64, i64)> {
    let first = total * 3 / 100;
    let last = total * 97 / 100 - WINDOW_MS;
    let step = (last - first) / (WINDOWS - 1);
    (0..WINDOWS).map(|i| (first + i * step, WINDOW_MS)).collect()
}

/// Speech of a video, read before (by any sync), when the file has not changed since.
pub fn cached(video: &Path) -> Option<Vec<Span>> {
    let text = std::fs::read_to_string(cache_file(video)?).ok()?;
    let spans: Vec<Span> = text
        .lines()
        .filter_map(|l| {
            let (s, e) = l.split_once(' ')?;
            Some((s.parse().ok()?, e.parse().ok()?))
        })
        .collect();
    (!spans.is_empty()).then_some(spans)
}

/// Keeps the speech of `video` for next time; the oldest files go beyond [`KEEP`].
pub fn store(video: &Path, speech: &[Span]) {
    let Some(path) = cache_file(video) else { return };
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let write = || -> std::io::Result<()> {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&path)?);
        for (s, e) in speech {
            writeln!(out, "{s} {e}")?;
        }
        out.flush()
    };
    if let Err(e) = write() {
        log::warn!("speech not cached: {e}");
        return;
    }
    prune(dir);
}

/// Drops the kept speech of `video`.
pub fn forget(video: &Path) {
    if let Some(path) = cache_file(video) {
        let _ = std::fs::remove_file(path);
    }
}

fn cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "SubMagician").map(|d| d.cache_dir().join("speech"))
}

/// The cache file for `video`: named after its path, size and modification time, so a changed
/// or replaced file is read again.
fn cache_file(video: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(video).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let path = std::fs::canonicalize(video).unwrap_or_else(|_| video.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(meta.len().to_le_bytes());
    hasher.update(modified.to_le_bytes());
    let name: String = hasher.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect();
    Some(cache_dir()?.join(format!("{name}.txt")))
}

fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> =
        entries.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if files.len() <= KEEP {
        return;
    }
    files.sort();
    for (_, old) in &files[..files.len() - KEEP] {
        let _ = std::fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_windows_inside_the_film() {
        let w = plan_windows(7_200_000);
        assert_eq!(w.len(), WINDOWS as usize);
        assert_eq!(w[0], (216_000, WINDOW_MS));
        let last = w.last().unwrap();
        assert!(last.0 + last.1 <= 7_200_000 * 97 / 100, "{w:?}");
        assert!(w.windows(2).all(|p| p[0].0 + p[0].1 < p[1].0), "no overlaps: {w:?}");
    }

    #[test]
    fn caches_speech_per_file() {
        let dir = std::env::temp_dir().join(format!("submagician-autosync-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("a.mkv");
        std::fs::write(&video, b"one").unwrap();
        store(&video, &[(10, 20), (30, 45)]);
        let file = cache_file(&video).unwrap();
        assert_eq!(cached(&video), Some(vec![(10, 20), (30, 45)]));
        std::fs::write(&video, b"changed!").unwrap();
        assert_eq!(cached(&video), None, "a changed file is read again");
        if let Some(old) = cache_file(&dir.join("gone.mkv")) {
            panic!("no cache file for a missing video: {old:?}");
        }
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
