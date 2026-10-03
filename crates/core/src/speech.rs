//! Subtitles from speech, with whisper.cpp: for videos no source has a subtitle for.
//!
//! Models are not shipped; the user picks one and it is downloaded once into the data folder.
//! Audio goes from ffmpeg as 16 kHz mono floats in pieces of about ten minutes, cut at the
//! quietest moment near the end of each piece so no word is split, and each piece is
//! transcribed on all CPU cores. Whisper can only translate into English: for any other wanted
//! language the speech is written down in the language it is spoken in.

use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Once};

use crate::{Error, Result, audio, lang, timing};

/// A downloadable whisper.cpp model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    pub id: &'static str,
    file: &'static str,
    /// Download size, roughly.
    pub size_mb: u32,
}

/// From fastest to most accurate. The quantized larger models give most of their accuracy at a
/// third of the size.
pub const MODELS: &[Model] = &[
    Model { id: "tiny", file: "ggml-tiny.bin", size_mb: 75 },
    Model { id: "base", file: "ggml-base.bin", size_mb: 142 },
    Model { id: "small", file: "ggml-small.bin", size_mb: 466 },
    Model { id: "medium", file: "ggml-medium-q5_0.bin", size_mb: 514 },
    Model { id: "large-v3-turbo", file: "ggml-large-v3-turbo-q5_0.bin", size_mb: 547 },
];

const DOWNLOAD_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

pub fn model(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

/// Folder the models are kept in.
pub fn models_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "SubMagician").map(|d| d.data_dir().join("models"))
}

impl Model {
    pub fn path_in(&self, dir: &Path) -> PathBuf {
        dir.join(self.file)
    }

    /// The model file, if it has been downloaded.
    pub fn installed(&self) -> Option<PathBuf> {
        models_dir().map(|d| self.path_in(&d)).filter(|p| is_model_file(p))
    }

    /// Downloads the model into `dir`. `progress` gets (bytes so far, total bytes if known).
    pub async fn download(
        &self,
        dir: &Path,
        cancel: &AtomicBool,
        progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let target = self.path_in(dir);
        let part = target.with_extension("part");
        let client = crate::net::client(None)?;
        let mut resp = client.get(format!("{DOWNLOAD_BASE}/{}", self.file)).send().await?.error_for_status()?;
        let total = resp.content_length();
        let mut file = std::fs::File::create(&part)?;
        let mut done = 0u64;
        while let Some(chunk) = resp.chunk().await? {
            if cancel.load(Ordering::Relaxed) {
                drop(file);
                let _ = std::fs::remove_file(&part);
                return Err(Error::Cancelled);
            }
            std::io::Write::write_all(&mut file, &chunk)?;
            done += chunk.len() as u64;
            progress(done, total);
        }
        drop(file);
        if !is_model_file(&part) {
            let _ = std::fs::remove_file(&part);
            return Err(Error::Parse("the downloaded file is not a whisper.cpp model".into()));
        }
        std::fs::rename(&part, &target)?;
        Ok(target)
    }
}

/// whisper.cpp (ggml) model files start with the bytes "lmgg".
fn is_model_file(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && &magic == b"lmgg"
}

/// One line of speech, in milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub start: i64,
    pub end: i64,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Transcript {
    /// Language code (see [`crate::lang`]) of the text.
    pub language: String,
    pub segments: Vec<Segment>,
}

impl Transcript {
    pub fn to_srt(&self) -> String {
        let mut out = String::new();
        for (i, s) in self.segments.iter().enumerate() {
            let (a, b) = (timing::fmt_clock(s.start, ','), timing::fmt_clock(s.end.max(s.start + 1), ','));
            out += &format!("{}\n{a} --> {b}\n{}\n\n", i + 1, s.text);
        }
        out
    }
}

const RATE: usize = 16_000;
/// About ten minutes of audio per piece…
const PIECE: usize = 10 * 60 * RATE;
/// …cut at the quietest 200 ms within its last 30 seconds.
const CUT_WINDOW: usize = 30 * RATE;
const QUIET_FRAME: usize = RATE / 5;

/// The instructions whisper.cpp is built with (see `.cargo/config.toml`). Without this check an
/// older processor crashes the process with an illegal instruction.
pub fn cpu_supported() -> Result<()> {
    #[cfg(target_arch = "x86_64")]
    {
        let missing: Vec<&str> = [
            ("AVX", std::arch::is_x86_feature_detected!("avx")),
            ("AVX2", std::arch::is_x86_feature_detected!("avx2")),
            ("FMA", std::arch::is_x86_feature_detected!("fma")),
            ("F16C", std::arch::is_x86_feature_detected!("f16c")),
        ]
        .into_iter()
        .filter_map(|(name, ok)| (!ok).then_some(name))
        .collect();
        if !missing.is_empty() {
            return Err(Error::Other(format!(
                "speech recognition needs a processor with {} (most made since 2013); this one does not have it",
                missing.join(", ")
            )));
        }
    }
    Ok(())
}

/// Writes down the speech in the first audio track of `video`. With `target == Some("en")` any
/// language is translated into English; otherwise the text stays in the spoken language, which
/// is detected. `progress` gets 0.0..=1.0.
pub fn transcribe(
    model: &Path,
    ffmpeg: &Path,
    video: &Path,
    target: Option<&str>,
    cancel: Arc<AtomicBool>,
    progress: Arc<dyn Fn(f32) + Send + Sync>,
) -> Result<Transcript> {
    use whisper_rs::{WhisperContext, WhisperContextParameters};

    static LOGGING: Once = Once::new();
    LOGGING.call_once(whisper_rs::install_logging_hooks);
    cpu_supported()?;

    let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .map_err(|e| Error::Parse(format!("whisper model: {e}")))?;
    let translate = target == Some("en");

    let mut child = audio::command(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-nostats", "-loglevel", "info", "-i"])
        .arg(video)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-acodec", "pcm_f32le"])
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
            let mut last = String::new();
            for line in BufReader::new(stderr).lines().map_while(std::result::Result::ok) {
                if let Some(ms) = audio::parse_duration(&line) {
                    duration_ms.store(ms, Ordering::Relaxed);
                }
                if !line.trim().is_empty() {
                    last = line;
                }
            }
            last
        })
    };

    let mut reader = BufReader::with_capacity(1 << 20, child.stdout.take().expect("piped stdout"));
    let mut buffer: Vec<f32> = Vec::with_capacity(PIECE + CUT_WINDOW);
    let mut offset_samples = 0usize;
    let mut language: Option<String> = None;
    let mut segments = Vec::new();
    let mut bytes = vec![0u8; 1 << 16];
    let mut eof = false;
    while !eof {
        // Fill up to one piece.
        while buffer.len() < PIECE {
            let n = match reader.read(&mut bytes) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => n - n % 4,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    let _ = child.kill();
                    return Err(e.into());
                }
            };
            buffer.extend(bytes[..n].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Cancelled);
            }
        }
        if buffer.len() < RATE / 2 {
            break; // nothing worth transcribing left
        }
        let cut = if eof { buffer.len() } else { quietest_cut(&buffer) };
        let total = duration_ms.load(Ordering::Relaxed);
        let piece_start_ms = (offset_samples * 1000 / RATE) as i64;
        let piece_ms = (cut * 1000 / RATE) as i64;
        let report = {
            let progress = progress.clone();
            move |pct: i32| {
                if total > 0 {
                    progress(((piece_start_ms + piece_ms * i64::from(pct) / 100) as f32 / total as f32).min(1.0));
                }
            }
        };
        let piece = transcribe_piece(&ctx, &buffer[..cut], language.as_deref(), translate, cancel.clone(), report)?;
        if language.is_none() {
            language = Some(piece.language);
        }
        segments.extend(piece.segments.into_iter().map(|s| Segment {
            start: s.start + piece_start_ms,
            end: s.end + piece_start_ms,
            text: s.text,
        }));
        offset_samples += cut;
        buffer.drain(..cut);
    }
    let status = child.wait()?;
    let last = stderr_thread.join().unwrap_or_default();
    if !status.success() {
        return Err(Error::Ffmpeg(last));
    }
    progress(1.0);
    if segments.is_empty() {
        return Err(Error::NoSpeech);
    }
    let spoken = language.unwrap_or_else(|| "en".into());
    let language = if translate { "en".to_owned() } else { lang::find(&spoken).map_or(spoken, |l| l.code.to_owned()) };
    Ok(Transcript { language, segments })
}

/// The quietest 200 ms in the last 30 s of `buffer`, as a sample index.
fn quietest_cut(buffer: &[f32]) -> usize {
    let from = buffer.len().saturating_sub(CUT_WINDOW);
    (from..buffer.len().saturating_sub(QUIET_FRAME))
        .step_by(QUIET_FRAME / 2)
        .min_by(|&a, &b| {
            let energy = |i: usize| buffer[i..i + QUIET_FRAME].iter().map(|s| s * s).sum::<f32>();
            energy(a).total_cmp(&energy(b))
        })
        .map_or(buffer.len(), |i| i + QUIET_FRAME / 2)
}

struct Piece {
    /// Whisper's code of the spoken language ("tr", "en", …).
    language: String,
    segments: Vec<Segment>,
}

fn transcribe_piece(
    ctx: &whisper_rs::WhisperContext,
    samples: &[f32],
    language: Option<&str>,
    translate: bool,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(i32) + 'static,
) -> Result<Piece> {
    use whisper_rs::{FullParams, SamplingStrategy};
    let whisper_err = |e: whisper_rs::WhisperError| Error::Parse(format!("whisper: {e}"));
    let mut state = ctx.create_state().map_err(whisper_err)?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16) as i32;
    params.set_n_threads(threads);
    params.set_translate(translate);
    params.set_language(Some(language.unwrap_or("auto")));
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    params.set_progress_callback_safe(progress);
    // whisper-rs 0.16's set_abort_callback_safe calls its closure through the wrong type, so the
    // raw callback is used: `cancel` outlives `state.full` below, which is the only user.
    unsafe extern "C" fn should_abort(user_data: *mut std::ffi::c_void) -> bool {
        // SAFETY: user_data is the `&AtomicBool` set below, alive for the whole `full` call.
        unsafe { (*(user_data as *const AtomicBool)).load(Ordering::Relaxed) }
    }
    // SAFETY: see `should_abort`.
    unsafe {
        params.set_abort_callback(Some(should_abort));
        params.set_abort_callback_user_data(Arc::as_ptr(&cancel) as *mut std::ffi::c_void);
    }
    state.full(params, samples).map_err(whisper_err)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let language = whisper_rs::get_lang_str(state.full_lang_id_from_state()).unwrap_or("en").to_owned();
    let mut segments = Vec::new();
    for i in 0..state.full_n_segments() {
        let Some(seg) = state.get_segment(i) else { continue };
        let text = seg.to_str_lossy().map(|t| t.trim().to_owned()).unwrap_or_default();
        if is_noise(&text) {
            continue;
        }
        segments.push(Segment { start: seg.start_timestamp() * 10, end: seg.end_timestamp() * 10, text });
    }
    tighten(&mut segments, samples);
    Ok(Piece { language, segments })
}

/// Whisper starts a segment where its 30 s window or the previous segment ends, often well
/// before the first word. Moves each segment's start and end onto the speech inside it, found
/// with WebRTC's voice detector in 10 ms frames.
fn tighten(segments: &mut [Segment], samples: &[f32]) {
    use webrtc_vad::{SampleRate, Vad, VadMode};
    const FRAME: usize = RATE / 100;
    let mut vad = Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, VadMode::Aggressive);
    let voiced: Vec<bool> = samples
        .as_chunks::<FRAME>()
        .0
        .iter()
        .map(|f| {
            let pcm: Vec<i16> = f.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
            vad.is_voice_segment(&pcm).unwrap_or(false)
        })
        .collect();
    for seg in segments {
        let (from, to) = ((seg.start / 10).max(0) as usize, ((seg.end / 10).max(0) as usize).min(voiced.len()));
        if from >= to {
            continue;
        }
        if let (Some(first), Some(last)) =
            (voiced[from..to].iter().position(|v| *v), voiced[from..to].iter().rposition(|v| *v))
        {
            seg.start = ((from + first) * 10) as i64;
            seg.end = ((from + last + 1) * 10) as i64;
        }
    }
}

/// Empty lines and Whisper's sound tags ("[Music]", "(applause)", "[BLANK_AUDIO]").
fn is_noise(text: &str) -> bool {
    let t = text.trim_matches(|c: char| c.is_whitespace() || c == '.' || c == '-');
    t.is_empty()
        || (t.starts_with('[') && t.ends_with(']') && !t[1..].contains('['))
        || (t.starts_with('(') && t.ends_with(')') && !t[1..].contains('('))
        || t.chars().all(|c| c == '♪' || c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_models() {
        assert_eq!(model("base").unwrap().path_in(Path::new("/m")), PathBuf::from("/m/ggml-base.bin"));
        assert!(model("huge").is_none());
    }

    #[test]
    fn drops_noise_lines() {
        for t in ["", " [Music] ", "(applause)", "[BLANK_AUDIO]", "♪ ♪", " - "] {
            assert!(is_noise(t), "{t:?}");
        }
        for t in ["Hello.", "[Music] then words [x]", "(laughs) Fine."] {
            assert!(!is_noise(t), "{t:?}");
        }
    }

    #[test]
    fn cuts_at_the_quietest_moment() {
        let mut buffer = vec![0.5f32; PIECE];
        let quiet = PIECE - 10 * RATE; // 10 s before the end
        for s in &mut buffer[quiet..quiet + QUIET_FRAME] {
            *s = 0.0;
        }
        let cut = quietest_cut(&buffer);
        assert!((cut as i64 - (quiet + QUIET_FRAME / 2) as i64).abs() <= QUIET_FRAME as i64, "{cut} vs {quiet}");
    }

    #[test]
    fn tightens_segments_onto_speech() {
        // 3 s: silence, then a voice-like buzz from 1.2 s to 2.0 s, then silence.
        let samples: Vec<f32> = (0..3 * RATE)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                if (1.2..2.0).contains(&t) {
                    0.4 * ((t * 2.0 * std::f32::consts::PI * 180.0).sin()
                        + 0.5 * (t * 2.0 * std::f32::consts::PI * 720.0).sin())
                } else {
                    0.0
                }
            })
            .collect();
        let mut segs = vec![
            Segment { start: 0, end: 2_900, text: "x".into() },
            Segment { start: 2_900, end: 3_000, text: "y".into() },
        ];
        tighten(&mut segs, &samples);
        assert!((segs[0].start - 1_200).abs() <= 100, "{segs:?}");
        assert!((segs[0].end - 2_000).abs() <= 150, "{segs:?}");
        assert_eq!((segs[1].start, segs[1].end), (2_900, 3_000), "no speech: unchanged");
    }

    #[test]
    fn writes_srt() {
        let t = Transcript {
            language: "en".into(),
            segments: vec![Segment { start: 1_500, end: 3_000, text: "Good morning, captain.".into() }],
        };
        assert_eq!(t.to_srt(), "1\n00:00:01,500 --> 00:00:03,000\nGood morning, captain.\n\n");
    }

    #[test]
    fn checks_model_magic() {
        let dir = std::env::temp_dir().join(format!("submagician-model-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("good.bin"), b"lmggrest").unwrap();
        std::fs::write(dir.join("bad.bin"), b"<html>").unwrap();
        assert!(is_model_file(&dir.join("good.bin")));
        assert!(!is_model_file(&dir.join("bad.bin")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
