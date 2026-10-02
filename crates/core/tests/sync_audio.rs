//! End to end: a video with synthetic speech at known times, a subtitle that is 4 s late, and
//! `sync_file` must bring every line back to its sentence. Needs ffmpeg with flite; skipped
//! otherwise unless `SUBMAGICIAN_REQUIRE_FFMPEG=1` (set in CI).

use std::process::Command;
use std::sync::atomic::AtomicBool;

use submagician_core::{audio, sync, timing::Document};

const SENTENCES: &[(i64, &str)] = &[
    (1_500, "Good morning, captain."),
    (6_000, "The engines are ready and the crew is waiting."),
    (13_500, "We leave at dawn."),
    (17_000, "Nobody told me about the storm."),
    (25_500, "Then we sail around it, as always."),
    (31_000, "Is that a joke?"),
    (38_500, "Load the last boxes and close the doors."),
    (46_000, "Goodbye, old harbor."),
];
const LATE_MS: i64 = 4_000;

#[test]
fn syncs_a_late_subtitle_to_the_audio() {
    let required = std::env::var("SUBMAGICIAN_REQUIRE_FFMPEG").is_ok_and(|v| v == "1");
    let Some(ffmpeg) = audio::find_ffmpeg(None) else {
        assert!(!required, "ffmpeg not found");
        return;
    };
    let dir = std::env::temp_dir().join(format!("submagician-e2e-{}", std::process::id()));
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
    let status =
        cmd.args(["-filter_complex", &filter, "-map", "[a]", "-c:a", "aac", "-t", "52"]).arg(&video).status().unwrap();
    if !status.success() {
        assert!(!required, "could not build the test video (ffmpeg without flite?)");
        return;
    }

    // The subtitle: one line per sentence, 2 s long, all 4 s late.
    let mut srt = String::new();
    for (i, (at, text)) in SENTENCES.iter().enumerate() {
        let (s, e) = (at + LATE_MS, at + LATE_MS + 2_000);
        let clock = |ms: i64| format!("00:{:02}:{:02},{:03}", ms / 60_000, ms / 1000 % 60, ms % 1000);
        srt += &format!("{}\n{} --> {}\n{text}\n\n", i + 1, clock(s), clock(e));
    }
    let sub = dir.join("film.en.srt");
    std::fs::write(&sub, &srt).unwrap();

    let speech = audio::extract_speech(&ffmpeg, &video, &AtomicBool::new(false), &mut |_| {}).unwrap();
    let report = sync::sync_file(&sub, &speech).unwrap();
    assert!(report.applied, "{report:?}");
    assert!((report.offset_ms + LATE_MS).abs() < 500, "{report:?}");

    let synced = Document::parse(&String::from_utf8(std::fs::read(&sub).unwrap()).unwrap()).unwrap();
    for (cue, (at, _)) in synced.cues.iter().zip(SENTENCES) {
        assert!((cue.start - at).abs() < 500, "line at {} should start near {at}: {report:?}", cue.start);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
