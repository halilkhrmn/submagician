//! What is inside a video file, through ffprobe: its subtitle tracks (so a video that already
//! carries the wanted language is not given a second subtitle), and taking a text track out as a
//! file that can then be synced to the audio.

use std::path::Path;
use std::process::Stdio;

use serde::Deserialize;

use crate::{Error, Result, audio, lang};

/// A subtitle track inside a video.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Stream index in the file (for `-map 0:<index>`).
    pub index: u32,
    pub language: Option<&'static str>,
    /// ffmpeg's codec name: subrip, ass, mov_text, hdmv_pgs_subtitle, …
    pub codec: String,
    pub title: String,
    /// Shows only the lines in other languages (signs, foreign speech).
    pub forced: bool,
}

impl Track {
    /// Text tracks can be taken out and synced; picture tracks (Blu-ray PGS, DVD) cannot
    /// without OCR.
    pub fn is_text(&self) -> bool {
        matches!(self.codec.as_str(), "subrip" | "srt" | "ass" | "ssa" | "mov_text" | "webvtt" | "text")
    }

    /// File extension for the track taken out.
    pub fn extension(&self) -> &'static str {
        if matches!(self.codec.as_str(), "ass" | "ssa") { "ass" } else { "srt" }
    }
}

/// Subtitle tracks of `video`, in file order.
pub fn subtitle_tracks(ffprobe: &Path, video: &Path) -> Result<Vec<Track>> {
    let output = audio::command(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "s",
            "-show_entries",
            "stream=index,codec_name:stream_tags=language,title:stream_disposition=forced",
            "-of",
            "json",
        ])
        .arg(video)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { Error::NoFfmpeg } else { Error::Io(e) })?;
    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or_default().to_owned();
        return Err(Error::Ffmpeg(msg));
    }
    parse(&output.stdout)
}

/// Language codes of the subtitle tracks in `video`, in track order, without repeats. Tracks
/// without a (known) language tag are left out.
pub fn embedded_languages(ffprobe: &Path, video: &Path) -> Result<Vec<&'static str>> {
    Ok(languages(&subtitle_tracks(ffprobe, video)?))
}

fn languages(tracks: &[Track]) -> Vec<&'static str> {
    let mut out = Vec::new();
    for code in tracks.iter().filter_map(|t| t.language) {
        if !out.contains(&code) {
            out.push(code);
        }
    }
    out
}

/// The track to take out for `languages` (most wanted first): a full text track (not forced)
/// in the first language that has one.
pub fn pick<'a>(tracks: &'a [Track], languages: &[String]) -> Option<&'a Track> {
    languages.iter().find_map(|code| {
        let mut in_language = tracks.iter().filter(|t| t.language == Some(code.as_str()) && t.is_text());
        let all: Vec<&Track> = in_language.by_ref().collect();
        all.iter().find(|t| !t.forced && !t.title.to_lowercase().contains("forced")).or(all.first()).copied()
    })
}

/// The text of `track` in `video`, as SRT (or ASS for ASS tracks). Reads through the whole file.
pub fn extract(ffmpeg: &Path, video: &Path, track: &Track) -> Result<String> {
    if !track.is_text() {
        return Err(Error::Parse(format!("the {} track is pictures, not text", track.codec)));
    }
    let mut cmd = audio::command(ffmpeg);
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(video)
        .args(["-map", &format!("0:{}", track.index)]);
    if track.extension() == "ass" {
        cmd.args(["-c:s", "copy", "-f", "ass"]);
    } else {
        cmd.args(["-c:s", "srt", "-f", "srt"]);
    }
    let output = cmd
        .arg("pipe:1")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { Error::NoFfmpeg } else { Error::Io(e) })?;
    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or_default().to_owned();
        return Err(Error::Ffmpeg(msg));
    }
    // ffmpeg writes subtitles as UTF-8.
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse(json: &[u8]) -> Result<Vec<Track>> {
    let probe: Probe = serde_json::from_slice(json).map_err(|e| Error::Parse(format!("ffprobe: {e}")))?;
    Ok(probe
        .streams
        .into_iter()
        .map(|s| {
            let tags = s.tags.unwrap_or_default();
            Track {
                index: s.index,
                language: tags.language.as_deref().and_then(lang::find).map(|l| l.code),
                codec: s.codec_name.unwrap_or_default(),
                title: tags.title.unwrap_or_default(),
                forced: s.disposition.is_some_and(|d| d.forced == 1),
            }
        })
        .collect())
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
}

#[derive(Deserialize)]
struct Stream {
    #[serde(default)]
    index: u32,
    codec_name: Option<String>,
    tags: Option<Tags>,
    disposition: Option<Disposition>,
}

#[derive(Deserialize, Default)]
struct Tags {
    language: Option<String>,
    title: Option<String>,
}

#[derive(Deserialize)]
struct Disposition {
    #[serde(default)]
    forced: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn parses_ffprobe_json() {
        let json = br#"{"programs":[],"streams":[{"index":2,"tags":{"language":"tur"}},{"index":3,"tags":{"language":"eng","title":"SDH"}},{"index":4},{"index":5,"tags":{"language":"und"}},{"index":6,"tags":{"language":"tur"}}]}"#;
        assert_eq!(languages(&parse(json).unwrap()), vec!["tr", "en"]);
        assert!(parse(br#"{"streams":[]}"#).unwrap().is_empty());
        assert!(parse(b"nope").is_err());

        let json = br#"{"streams":[
            {"index":2,"codec_name":"hdmv_pgs_subtitle","tags":{"language":"tur"},"disposition":{"forced":0}},
            {"index":3,"codec_name":"subrip","tags":{"language":"tur","title":"Forced"},"disposition":{"forced":0}},
            {"index":4,"codec_name":"subrip","tags":{"language":"tur"},"disposition":{"forced":0}},
            {"index":5,"codec_name":"ass","tags":{"language":"eng"},"disposition":{"forced":0}}]}"#;
        let tracks = parse(json).unwrap();
        let want = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(pick(&tracks, &want(&["tr", "en"])).map(|t| t.index), Some(4), "text, not forced");
        let en = pick(&tracks, &want(&["de", "en"])).unwrap();
        assert_eq!((en.index, en.extension()), (5, "ass"));
        assert_eq!(pick(&tracks[..1], &want(&["tr"])), None, "pictures only");
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
        let tracks = subtitle_tracks(&ffprobe, &video).unwrap();
        let text = extract(&ffmpeg, &video, &tracks[1]).unwrap();
        assert!(text.contains("00:00:00,500 --> 00:00:01,500") && text.contains("Selam"), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
