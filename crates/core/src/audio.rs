//! Speech in a video's audio: ffmpeg decodes the first audio track to 8 kHz mono PCM, WebRTC's
//! voice activity detector marks each 10 ms frame, and the voiced frames become spans.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use webrtc_vad::{SampleRate, Vad, VadMode};

use crate::sync::{Span, frames_to_spans};
use crate::{Error, Result};

const RATE: usize = 8000;
const FRAME_MS: i64 = 10;
const FRAME_SAMPLES: usize = RATE / 1000 * FRAME_MS as usize;
/// Pauses shorter than this inside a sentence are joined.
const BRIDGE_MS: i64 = 200;
/// Voiced bits shorter than this are noise.
const MIN_SPEECH_MS: i64 = 100;

fn exe_name(tool: &str) -> String {
    if cfg!(windows) { format!("{tool}.exe") } else { tool.to_owned() }
}

/// Finds ffmpeg: the configured path, next to SubMagician's executable, then on `PATH`.
pub fn find_ffmpeg(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured.filter(|p| !p.as_os_str().is_empty()) {
        return p.is_file().then(|| p.to_path_buf());
    }
    find_tool("ffmpeg")
}

/// Finds ffprobe: next to the configured ffmpeg, next to SubMagician, then on `PATH`.
pub fn find_ffprobe(configured_ffmpeg: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = configured_ffmpeg.filter(|p| !p.as_os_str().is_empty()).and_then(Path::parent) {
        let p = dir.join(exe_name("ffprobe"));
        if p.is_file() {
            return Some(p);
        }
    }
    find_tool("ffprobe")
}

fn find_tool(tool: &str) -> Option<PathBuf> {
    let name = exe_name(tool);
    let beside = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(&name)));
    if let Some(p) = beside.filter(|p| p.is_file()) {
        return Some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(&name)).find(|p| p.is_file())
}

pub(crate) fn command(ffmpeg: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Speech spans (milliseconds) of the first audio track of `video`. `progress` gets 0.0..=1.0
/// when the duration is known. Checks `cancel` while decoding.
pub fn extract_speech(
    ffmpeg: &Path,
    video: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<Span>> {
    let mut child = command(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-nostats", "-loglevel", "info", "-i"])
        .arg(video)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-ac", "1", "-ar", "8000", "-f", "s16le", "-acodec", "pcm_s16le"])
        .arg("pipe:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| if e.kind() == ErrorKind::NotFound { Error::NoFfmpeg } else { Error::Io(e) })?;

    let duration_ms = Arc::new(AtomicI64::new(0));
    let stderr = child.stderr.take().expect("piped stderr");
    let stderr_thread = {
        let duration_ms = duration_ms.clone();
        std::thread::spawn(move || {
            let mut tail = VecDeque::new();
            for line in BufReader::new(stderr).lines().map_while(std::result::Result::ok) {
                if let Some(ms) = parse_duration(&line) {
                    duration_ms.store(ms, Ordering::Relaxed);
                }
                if tail.len() == 8 {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
            tail
        })
    };

    let mut vad = Vad::new_with_rate_and_mode(SampleRate::Rate8kHz, VadMode::Aggressive);
    let mut reader = BufReader::with_capacity(64 * 1024, child.stdout.take().expect("piped stdout"));
    let mut bytes = [0u8; FRAME_SAMPLES * 2];
    let mut samples = [0i16; FRAME_SAMPLES];
    let mut voiced = Vec::new();
    loop {
        match reader.read_exact(&mut bytes) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => {
                let _ = child.kill();
                return Err(e.into());
            }
        }
        for (s, b) in samples.iter_mut().zip(bytes.as_chunks::<2>().0) {
            *s = i16::from_le_bytes(*b);
        }
        voiced.push(vad.is_voice_segment(&samples).unwrap_or(false));
        if voiced.len() % 1000 == 0 {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Cancelled);
            }
            let total = duration_ms.load(Ordering::Relaxed);
            if total > 0 {
                progress((voiced.len() as i64 * FRAME_MS) as f32 / total as f32);
            }
        }
    }
    let status = child.wait()?;
    let tail = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        let last = tail.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
        return Err(Error::Ffmpeg(last));
    }
    progress(1.0);
    let spans = frames_to_spans(&voiced, FRAME_MS, BRIDGE_MS, MIN_SPEECH_MS);
    if spans.is_empty() { Err(Error::NoSpeech) } else { Ok(spans) }
}

/// `  Duration: 01:23:45.67, start: …` → milliseconds.
fn parse_duration(line: &str) -> Option<i64> {
    let rest = line.trim_start().strip_prefix("Duration:")?.trim_start();
    let clock = rest.split(',').next()?;
    let (hms, frac) = clock.split_once('.').unwrap_or((clock, "0"));
    let p: Vec<i64> = hms.split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let [h, m, s] = p.as_slice() else { return None };
    let cs: i64 = frac.get(..2).unwrap_or(frac).parse().ok()?;
    Some(((h * 60 + m) * 60 + s) * 1000 + cs * 10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{merge, overlap};

    #[test]
    fn parses_ffmpeg_duration() {
        assert_eq!(parse_duration("  Duration: 01:02:03.45, start: 0.000000, bitrate: 1 kb/s"), Some(3_723_450));
        assert_eq!(parse_duration("  Duration: N/A, bitrate: N/A"), None);
        assert_eq!(parse_duration("Stream #0:0"), None);
    }

    #[test]
    fn missing_ffmpeg_is_reported() {
        let r =
            extract_speech(Path::new("/nonexistent/ffmpeg"), Path::new("x.mkv"), &AtomicBool::new(false), &mut |_| {});
        assert!(matches!(r, Err(Error::NoFfmpeg)));
        assert_eq!(find_ffmpeg(Some(Path::new("/nonexistent/ffmpeg"))), None);
    }

    /// Builds a video whose audio has synthetic speech (ffmpeg's flite) at known times and checks
    /// the detected speech lands there. Needs ffmpeg with flite; skipped otherwise unless
    /// `SUBMAGICIAN_REQUIRE_FFMPEG=1` (set in CI).
    #[test]
    fn detects_synthetic_speech() {
        let required = std::env::var("SUBMAGICIAN_REQUIRE_FFMPEG").is_ok_and(|v| v == "1");
        let Some(ffmpeg) = find_ffmpeg(None) else {
            assert!(!required, "ffmpeg not found");
            eprintln!("skipped: no ffmpeg");
            return;
        };
        let dir = std::env::temp_dir().join(format!("submagician-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("speech.mkv");
        // Speech starting at 2 s and 9 s in 16 s of silence; plus a black video track.
        let filter = "[1:a]adelay=2000:all=1[a1];[2:a]adelay=9000:all=1[a2];[0:a][a1][a2]amix=inputs=3:duration=first:normalize=0,aresample=16000[a]";
        let status = Command::new(&ffmpeg)
            .args(["-y", "-hide_banner", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono:d=16"])
            .args([
                "-f",
                "lavfi",
                "-i",
                "flite=text='The quick brown fox jumps over the lazy dog. Subtitles are magic.'",
            ])
            .args(["-f", "lavfi", "-i", "flite=text='Where are we going tonight, my friend? Nobody knows.'"])
            .args(["-f", "lavfi", "-i", "color=c=black:s=64x64:r=10:d=16"])
            .args([
                "-filter_complex",
                filter,
                "-map",
                "3:v",
                "-map",
                "[a]",
                "-c:v",
                "mpeg4",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&video)
            .status()
            .unwrap();
        if !status.success() {
            assert!(!required, "could not build the test video (ffmpeg without flite?)");
            eprintln!("skipped: ffmpeg cannot synthesize speech");
            return;
        }
        let mut last = 0.0;
        let spans = extract_speech(&ffmpeg, &video, &AtomicBool::new(false), &mut |p| last = p).unwrap();
        assert_eq!(last, 1.0);
        let speech = merge(spans.clone());
        // Each sentence takes roughly 2.5–4 s; check the middle part of each is found, and silence is not.
        assert!(overlap(&[(2_500, 4_000), (9_500, 11_000)], &speech) > 0.8, "{spans:?}");
        assert!(overlap(&[(0, 1_800), (14_500, 16_000)], &speech) < 0.1, "{spans:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
