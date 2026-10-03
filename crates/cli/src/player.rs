//! `--player`: one video for the mpv and VLC plugins. Output lines, tab separated:
//! - `subtitle<TAB><path>`: the subtitle to load (at most one);
//! - `message<TAB><text>`: what to show in the player.
//!
//! Exit code 0 when there is a subtitle (or, with `--auto`, nothing to do), 1 when none was found.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use submagician_core::engine::Engine;
use submagician_core::settings::Settings;
use submagician_core::{audio, media, probe, score, speech};

use crate::Args;

fn message(text: &str) {
    println!("message\t{}", text.replace(['\t', '\n'], " "));
}

pub async fn run(args: &Args, settings: &Settings) -> ExitCode {
    let [input] = args.paths.as_slice() else {
        message("give one video");
        return ExitCode::from(2);
    };
    let path = path_from_input(input);
    let Some(mut video) = media::media_file(&path) else {
        message("not a video file on this computer");
        return ExitCode::from(2);
    };
    let languages = settings.language_codes();
    let first = languages[0].as_str();
    let ffmpeg_setting = PathBuf::from(&settings.ffmpeg_path);

    if !args.force {
        if let Some(existing) = video.existing.iter().find(|s| s.language == Some(first)) {
            if args.auto {
                // The player loads subtitles next to the video by itself.
                return ExitCode::SUCCESS;
            }
            println!("subtitle\t{}", existing.path.display());
            message(&format!("{first} subtitle next to the video"));
            return ExitCode::SUCCESS;
        }
        if args.auto
            && let Some(ffprobe) = audio::find_ffprobe(Some(&ffmpeg_setting))
        {
            video.embedded = probe::embedded_languages(&ffprobe, &video.path).unwrap_or_default();
            if video.embedded.contains(&first) {
                return ExitCode::SUCCESS;
            }
        }
    }

    let engine = settings.engine();
    let query = Engine::query_for(&video, &languages);
    let outcome = engine.search(&query, args.fresh).await;
    let ffmpeg = audio::find_ffmpeg(Some(&ffmpeg_setting));
    if let Some(best) = score::best(&outcome.candidates, &languages) {
        let c = &outcome.candidates[best];
        match engine.fetch(&video, &query, c).await {
            Ok(saved) => {
                let note = match &ffmpeg {
                    Some(ffmpeg) if settings.auto_sync && !args.no_sync => {
                        crate::sync_note(ffmpeg, &video.path, &saved.path)
                    }
                    _ => String::new(),
                };
                println!("subtitle\t{}", saved.path.display());
                message(&format!("{} subtitle from {}{note}", c.language, c.provider));
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                message(&format!("download failed: {e}"));
                return ExitCode::from(1);
            }
        }
    }

    let model = (args.generate || settings.generate_when_missing)
        .then(|| speech::model(&settings.whisper_model).and_then(|m| m.installed()))
        .flatten();
    if let (Some(model), Some(ffmpeg)) = (model, &ffmpeg) {
        return match crate::generate(&model, ffmpeg, &video.path, first) {
            Ok(path) => {
                println!("subtitle\t{}", path.display());
                message("no subtitle online; written from the audio");
                ExitCode::SUCCESS
            }
            Err(e) => {
                message(&format!("nothing found, and from the audio: {e}"));
                ExitCode::from(1)
            }
        };
    }
    match outcome.errors.first() {
        Some(e) if outcome.candidates.is_empty() => message(&format!("nothing found ({e})")),
        _ => message(&format!("no {} subtitle found", languages.join(", "))),
    }
    ExitCode::from(1)
}

/// A path, or a `file://` URI as VLC gives it (percent-encoded UTF-8).
fn path_from_input(input: &Path) -> PathBuf {
    let text = input.to_string_lossy();
    let Some(rest) = text.strip_prefix("file://") else { return input.to_path_buf() };
    let decoded = percent_decode(rest);
    // file:///C:/x → C:/x ; file://server/share → //server/share
    if cfg!(windows) {
        let decoded = String::from_utf8_lossy(&decoded).into_owned();
        let local = match decoded.strip_prefix('/') {
            Some(p) if p.as_bytes().get(1) == Some(&b':') => p.to_owned(),
            Some(p) => p.to_owned(),
            None => format!("//{decoded}"),
        };
        return PathBuf::from(local.replace('/', "\\"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        // "file://localhost/x" names the same file as "file:///x".
        let local = match decoded.strip_prefix(b"localhost") {
            Some(p) if p.starts_with(b"/") => p.to_vec(),
            _ => decoded,
        };
        PathBuf::from(std::ffi::OsString::from_vec(local))
    }
    #[cfg(not(unix))]
    PathBuf::from(String::from_utf8_lossy(&decoded).into_owned())
}

fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = text.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_file_uris() {
        assert_eq!(percent_decode("a%20b%C3%A7%zz%4"), b"a b\xC3\xA7%zz%4");
        if cfg!(windows) {
            assert_eq!(path_from_input(Path::new("file:///C:/My%20Films/x.mkv")), PathBuf::from(r"C:\My Films\x.mkv"));
            assert_eq!(path_from_input(Path::new("file://nas/films/x.mkv")), PathBuf::from(r"\\nas\films\x.mkv"));
        } else {
            assert_eq!(path_from_input(Path::new("file:///home/a/%C3%A7%20x.mkv")), PathBuf::from("/home/a/ç x.mkv"));
            assert_eq!(path_from_input(Path::new("file://localhost/x.mkv")), PathBuf::from("/x.mkv"));
            assert_eq!(path_from_input(Path::new("/plain/path.mkv")), PathBuf::from("/plain/path.mkv"));
        }
    }
}
