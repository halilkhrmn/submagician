//! Whisper on real audio: the synthetic-speech video of `sync_audio.rs`, written down with the
//! model in `SUBMAGICIAN_WHISPER_MODEL` (CI uses ggml-tiny). Skipped without that variable or
//! without ffmpeg/flite.
#![cfg(feature = "whisper")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use submagician_core::{audio, speech};

const SENTENCES: &[(i64, &str)] = &[
    (1_500, "Good morning, captain."),
    (6_000, "The engines are ready and the crew is waiting."),
    (13_500, "We leave at dawn."),
    (17_000, "Nobody told me about the storm."),
    (25_500, "Then we sail around it, as always."),
    (38_500, "Load the last boxes and close the doors."),
    (46_000, "Goodbye, old harbor."),
];

#[test]
fn writes_down_synthetic_speech() {
    let Some(model) = std::env::var_os("SUBMAGICIAN_WHISPER_MODEL").filter(|v| !v.is_empty()).map(PathBuf::from) else {
        return;
    };
    let Some(ffmpeg) = audio::find_ffmpeg(None) else { panic!("ffmpeg is needed for the whisper test") };
    let dir = std::env::temp_dir().join(format!("submagician-whisper-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let video = dir.join("film.mkv");
    let mut cmd = Command::new(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono:d=52"]);
    let mut filter = String::new();
    for (i, (at, text)) in SENTENCES.iter().enumerate() {
        cmd.args(["-f", "lavfi", "-i", &format!("flite=text='{text}'")]);
        filter += &format!("[{}:a]aresample=16000,adelay={at}:all=1[s{i}];", i + 1);
    }
    for i in 0..SENTENCES.len() {
        filter += &format!("[s{i}]");
    }
    filter += &format!("[0:a]amix=inputs={}:duration=longest:normalize=0[a]", SENTENCES.len() + 1);
    let ok =
        cmd.args(["-filter_complex", &filter, "-map", "[a]", "-c:a", "aac", "-t", "52"]).arg(&video).status().unwrap();
    assert!(ok.success(), "could not build the test video (ffmpeg without flite?)");

    let seen = Arc::new(Mutex::new(Vec::new()));
    let progress = {
        let seen = seen.clone();
        Arc::new(move |p: f32| seen.lock().unwrap().push(p))
    };
    let t = speech::transcribe(&model, &ffmpeg, &video, None, Arc::new(AtomicBool::new(false)), progress).unwrap();
    let text = t.segments.iter().map(|s| s.text.to_lowercase()).collect::<Vec<_>>().join(" ");
    eprintln!("{} [{}]: {text}", t.segments.len(), t.language);
    assert_eq!(t.language, "en");
    let words = ["captain", "engines", "crew", "dawn", "storm", "sail", "boxes", "doors", "harbor"];
    let found = words.iter().filter(|w| text.contains(*w)).count();
    assert!(found >= 5, "only {found} of the key words in {text:?}");
    for seg in &t.segments {
        let nearest = SENTENCES.iter().map(|(at, _)| (seg.start - at).abs()).min().unwrap();
        assert!(nearest <= 800, "line {:?} starts {nearest} ms away from any sentence", seg);
    }
    assert_eq!(seen.lock().unwrap().last().copied(), Some(1.0));
    let srt = t.to_srt();
    assert!(srt.starts_with("1\n00:00:0"), "{srt}");
    std::fs::remove_dir_all(&dir).unwrap();
}
