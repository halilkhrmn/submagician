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

/// Finds ffmpeg: the configured path, next to SubMagician's executable, the one Settings
/// downloaded (Windows), then on `PATH`.
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
    let installed = crate::tools::tools_dir().map(|d| d.join(&name));
    if let Some(p) = beside.into_iter().chain(installed).find(|p| p.is_file()) {
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

/// Pieces shorter than this are not worth a process of their own.
const MIN_PIECE_MS: i64 = 60_000;
/// At most this many ffmpeg processes decode at once.
const MAX_PIECES: usize = 8;

/// A part of the audio: start and length in milliseconds.
pub type Range = (i64, i64);

/// Length of `video` in milliseconds, from the header ffmpeg prints (no decoding).
pub fn duration_ms(ffmpeg: &Path, video: &Path) -> Result<Option<i64>> {
    let output = command(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-i"])
        .arg(video)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| if e.kind() == ErrorKind::NotFound { Error::NoFfmpeg } else { Error::Io(e) })?;
    Ok(String::from_utf8_lossy(&output.stderr).lines().find_map(parse_duration))
}

/// How many decoders to run at once on this computer.
fn parallelism() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, MAX_PIECES)
}

/// Speech spans (milliseconds) of the first audio track of `video`. Long videos are cut into
/// pieces decoded at the same time, one ffmpeg per CPU core. `progress` gets 0.0..=1.0 when the
/// duration is known. Checks `cancel` while decoding.
pub fn extract_speech(
    ffmpeg: &Path,
    video: &Path,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(f32) + Send),
) -> Result<Vec<Span>> {
    let duration = duration_ms(ffmpeg, video)?;
    let pieces = match duration {
        Some(total) => split(total, parallelism()),
        None => vec![(0, 0)],
    };
    let frames = decode_ranges(ffmpeg, video, &pieces, cancel, progress)?;
    // Every piece but the last is cut to its exact length, so the frames line up in time.
    let mut voiced = Vec::new();
    for (i, (piece, mut v)) in pieces.iter().zip(frames).enumerate() {
        if i + 1 < pieces.len() {
            v.resize((piece.1 / FRAME_MS) as usize, false);
        }
        voiced.extend(v);
    }
    let spans = frames_to_spans(&voiced, FRAME_MS, BRIDGE_MS, MIN_SPEECH_MS);
    if spans.is_empty() { Err(Error::NoSpeech) } else { Ok(spans) }
}

/// Speech spans inside each of `windows` (absolute times), decoded at the same time: a quick
/// look at parts of a long video.
pub fn sample_speech(
    ffmpeg: &Path,
    video: &Path,
    windows: &[Range],
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(f32) + Send),
) -> Result<Vec<Vec<Span>>> {
    let frames = decode_ranges(ffmpeg, video, windows, cancel, progress)?;
    Ok(windows
        .iter()
        .zip(frames)
        .map(|(w, v)| {
            frames_to_spans(&v, FRAME_MS, BRIDGE_MS, MIN_SPEECH_MS)
                .into_iter()
                .map(|(s, e)| (s + w.0, e + w.0))
                .collect()
        })
        .collect())
}

/// `total` ms cut into at most `n` equal pieces of at least [`MIN_PIECE_MS`].
fn split(total: i64, n: usize) -> Vec<Range> {
    let n = (total / MIN_PIECE_MS).clamp(1, n as i64);
    // Whole frames per piece, so pieces join without a gap.
    let len = (total / n / FRAME_MS + 1) * FRAME_MS;
    (0..n).map(|i| (i * len, if i + 1 == n { 0 } else { len })).collect()
}

/// Voice decisions per 10 ms frame for each range, `parallelism()` ffmpeg processes at a time.
/// A range `(start, 0)` reads to the end.
fn decode_ranges(
    ffmpeg: &Path,
    video: &Path,
    ranges: &[Range],
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(f32) + Send),
) -> Result<Vec<Vec<bool>>> {
    let done_frames = AtomicI64::new(0);
    // Known total frames; a range to the end learns its length from ffmpeg.
    let known: i64 = ranges.iter().map(|r| r.1 / FRAME_MS).sum();
    let open_end = AtomicI64::new(0);
    let progress = std::sync::Mutex::new(progress);
    let report = || {
        let total = known + open_end.load(Ordering::Relaxed);
        if total > 0 {
            let p = (done_frames.load(Ordering::Relaxed) as f32 / total as f32).min(1.0);
            (progress.lock().unwrap())(p);
        }
    };
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut results: Vec<Option<Result<Vec<bool>>>> = (0..ranges.len()).map(|_| None).collect();
    let slots: Vec<std::sync::Mutex<&mut Option<Result<Vec<bool>>>>> =
        results.iter_mut().map(std::sync::Mutex::new).collect();
    std::thread::scope(|scope| {
        for _ in 0..parallelism().min(ranges.len()) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some(&range) = ranges.get(i) else { break };
                    let result = decode_range(ffmpeg, video, range, cancel, &mut |frames, total| {
                        done_frames.fetch_add(frames, Ordering::Relaxed);
                        if range.1 == 0 && total > 0 {
                            open_end.store((total - range.0) / FRAME_MS, Ordering::Relaxed);
                        }
                        report();
                    });
                    let failed = result.is_err();
                    **slots[i].lock().unwrap() = Some(result);
                    if failed {
                        // The others stop at their next check.
                        break;
                    }
                }
            });
        }
    });
    drop(slots);
    let mut out = Vec::with_capacity(ranges.len());
    for r in results {
        match r {
            Some(r) => out.push(r?),
            // Not started because another range failed or Stop was pressed.
            None if cancel.load(Ordering::Relaxed) => return Err(Error::Cancelled),
            None => return Err(Error::Ffmpeg("decoding stopped".into())),
        }
    }
    (progress.lock().unwrap())(1.0);
    Ok(out)
}

/// Voice decisions for one range. `on_frames` gets (frames since the last call, the video's
/// duration in ms or 0 while unknown).
fn decode_range(
    ffmpeg: &Path,
    video: &Path,
    (start, len): Range,
    cancel: &AtomicBool,
    on_frames: &mut dyn FnMut(i64, i64),
) -> Result<Vec<bool>> {
    let mut cmd = command(ffmpeg);
    cmd.args(["-nostdin", "-hide_banner", "-nostats", "-loglevel", "info"]);
    // Before -i: ffmpeg seeks in the file and then decodes exactly from `start`.
    if start > 0 {
        cmd.arg("-ss").arg(format!("{:.3}", start as f64 / 1000.0));
    }
    if len > 0 {
        cmd.arg("-t").arg(format!("{:.3}", len as f64 / 1000.0));
    }
    let mut child = cmd
        .arg("-i")
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
                let _ = child.wait();
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
            on_frames(1000, duration_ms.load(Ordering::Relaxed));
        }
    }
    on_frames((voiced.len() % 1000) as i64, duration_ms.load(Ordering::Relaxed));
    let status = child.wait()?;
    let tail = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        let last = tail.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
        return Err(Error::Ffmpeg(last));
    }
    Ok(voiced)
}

/// `  Duration: 01:23:45.67, start: …` → milliseconds.
pub(crate) fn parse_duration(line: &str) -> Option<i64> {
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
    fn splits_into_pieces() {
        assert_eq!(split(30_000, 8), vec![(0, 0)], "short: one piece");
        assert_eq!(
            split(7_200_000, 4),
            vec![(0, 1_800_010), (1_800_010, 1_800_010), (3_600_020, 1_800_010), (5_400_030, 0)]
        );
        let p = split(200_000, 8);
        assert_eq!(p.len(), 3, "at least a minute each: {p:?}");
        assert!(p.windows(2).all(|w| w[0].0 + w[0].1 == w[1].0));
    }

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
