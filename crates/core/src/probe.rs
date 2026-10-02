//! What is inside a video file, through ffprobe: the languages of its subtitle tracks, so a video
//! that already carries the wanted language is not given a second subtitle.

use std::path::Path;

use serde::Deserialize;

use crate::{Error, Result, audio, lang};

/// Language codes of the subtitle tracks in `video`, in track order, without repeats. Tracks
/// without a (known) language tag are left out.
pub fn embedded_languages(ffprobe: &Path, video: &Path) -> Result<Vec<&'static str>> {
    let output = audio::command(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "s",
            "-show_entries",
            "stream=index:stream_tags=language",
            "-of",
            "json",
        ])
        .arg(video)
        .stdin(std::process::Stdio::null())
        .output()?;
    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or_default().to_owned();
        return Err(Error::Ffmpeg(msg));
    }
    parse(&output.stdout)
}

fn parse(json: &[u8]) -> Result<Vec<&'static str>> {
    let probe: Probe = serde_json::from_slice(json).map_err(|e| Error::Parse(format!("ffprobe: {e}")))?;
    let mut out = Vec::new();
    for s in probe.streams {
        let code = s.tags.and_then(|t| t.language).and_then(|l| lang::find(&l)).map(|l| l.code);
        if let Some(code) = code.filter(|c| !out.contains(c)) {
            out.push(code);
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
}

#[derive(Deserialize)]
struct Stream {
    tags: Option<Tags>,
}

#[derive(Deserialize)]
struct Tags {
    language: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn parses_ffprobe_json() {
        let json = br#"{"programs":[],"streams":[{"index":2,"tags":{"language":"tur"}},{"index":3,"tags":{"language":"eng","title":"SDH"}},{"index":4},{"index":5,"tags":{"language":"und"}},{"index":6,"tags":{"language":"tur"}}]}"#;
        assert_eq!(parse(json).unwrap(), vec!["tr", "en"]);
        assert_eq!(parse(br#"{"streams":[]}"#).unwrap(), Vec::<&str>::new());
        assert!(parse(b"nope").is_err());
    }

    /// A real MKV with Turkish and English subtitle tracks. Needs ffmpeg + ffprobe; skipped
    /// otherwise unless `SUBMAGICIAN_REQUIRE_FFMPEG=1` (set in CI on Linux).
    #[test]
    fn reads_tracks_of_a_real_file() {
        let required = std::env::var("SUBMAGICIAN_REQUIRE_FFMPEG").is_ok_and(|v| v == "1");
        let (Some(ffmpeg), Some(ffprobe)) = (audio::find_ffmpeg(None), audio::find_ffprobe(None)) else {
            assert!(!required, "ffmpeg/ffprobe not found");
            return;
        };
        let dir = std::env::temp_dir().join(format!("submagician-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let srt = dir.join("s.srt");
        std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:01,500\nSelam\n").unwrap();
        let video = dir.join("film.mkv");
        let ok = Command::new(&ffmpeg)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=64x64:r=5:d=2"])
            .arg("-i")
            .arg(&srt)
            .arg("-i")
            .arg(&srt)
            .args(["-map", "0", "-map", "1", "-map", "2", "-c:v", "mpeg4", "-c:s", "srt"])
            .args(["-metadata:s:s:0", "language=tur", "-metadata:s:s:1", "language=eng"])
            .arg(&video)
            .status()
            .unwrap()
            .success();
        assert!(ok, "could not build the test video");
        assert_eq!(embedded_languages(&ffprobe, &video).unwrap(), vec!["tr", "en"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
