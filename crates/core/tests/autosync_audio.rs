//! End to end on a 20-minute video with synthetic speech: a subtitle with a wrong frame rate and
//! offset is fixed from the quick look, one with a cut scene needs the full (parallel) read, and
//! the speech of that read is kept for the next sync. Needs ffmpeg with flite; skipped otherwise
//! unless `SUBMAGICIAN_REQUIRE_FFMPEG=1` (set in CI).

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use submagician_core::autosync::{self, Method};
use submagician_core::{audio, timing::Document};

const SENTENCES: &[&str] = &[
    "Good morning, captain.",
    "The engines are ready and the crew is waiting.",
    "We leave at dawn.",
    "Nobody told me about the storm.",
    "Then we sail around it, as always.",
    "Is that a joke?",
];

/// Units of different lengths with sentences at different places, so the film never repeats
/// with one period: (length ms, [(start ms, sentence)]).
fn units() -> Vec<(i64, Vec<(i64, usize)>)> {
    vec![
        (47_000, vec![(2_000, 0), (9_000, 1), (21_000, 2), (30_500, 3), (40_000, 5)]),
        (53_000, vec![(1_000, 4), (12_000, 3), (19_500, 0), (33_000, 1), (45_000, 2)]),
        (61_000, vec![(4_000, 5), (10_000, 2), (24_000, 4), (37_000, 0), (48_000, 1), (55_000, 3)]),
    ]
}
const ORDER: &[usize] = &[0, 1, 2, 1, 0, 2, 2, 0, 1, 0, 2, 1, 1, 2, 0, 0, 1, 2, 2, 1, 0, 1, 2];

fn clock(ms: i64) -> String {
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Builds the film; returns its sentence starts, or `None` when ffmpeg cannot synthesize speech.
fn build(ffmpeg: &Path, dir: &Path) -> Option<Vec<(i64, i64)>> {
    let mut list = String::new();
    for (u, (len, lines)) in units().iter().enumerate() {
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi"]);
        cmd.args(["-i", &format!("anullsrc=r=16000:cl=mono:d={}", len / 1000)]);
        let mut filter = String::new();
        for (i, (at, s)) in lines.iter().enumerate() {
            cmd.args(["-f", "lavfi", "-i", &format!("flite=text='{}'", SENTENCES[*s])]);
            filter += &format!("[{}:a]aresample=16000,adelay={at}:all=1[s{i}];", i + 1);
        }
        // The silence goes first: amix's "first" input sets the length.
        filter += "[0:a]";
        for i in 0..lines.len() {
            filter += &format!("[s{i}]");
        }
        filter += &format!("amix=inputs={}:duration=first:normalize=0[a]", lines.len() + 1);
        let unit = dir.join(format!("unit{u}.wav"));
        if !cmd.args(["-filter_complex", &filter, "-map", "[a]"]).arg(&unit).status().ok()?.success() {
            return None;
        }
    }
    let mut truth = Vec::new();
    let mut t = 0;
    for &u in ORDER {
        list += &format!("file 'unit{u}.wav'\n");
        let (len, lines) = &units()[u];
        // A line lasts as long as its sentence takes to say, roughly.
        truth.extend(lines.iter().map(|(at, s)| (t + at, t + at + 600 + SENTENCES[*s].len() as i64 * 60)));
        t += len;
    }
    std::fs::write(dir.join("list.txt"), list).unwrap();
    let ok = Command::new(ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i"])
        .arg(dir.join("list.txt"))
        .args(["-c:a", "aac"])
        .arg(dir.join("film.mkv"))
        .status()
        .ok()?
        .success();
    ok.then_some(truth)
}

fn write_srt(path: &Path, cues: &[(i64, i64)]) {
    let mut srt = String::new();
    for (i, (s, e)) in cues.iter().enumerate() {
        srt += &format!("{}\n{} --> {}\nline {i}\n\n", i + 1, clock(*s), clock(*e));
    }
    std::fs::write(path, srt).unwrap();
}

fn max_error(path: &Path, truth: &[(i64, i64)]) -> i64 {
    let doc = Document::parse(&String::from_utf8_lossy(&std::fs::read(path).unwrap())).unwrap();
    doc.cues.iter().zip(truth).map(|(c, t)| (c.start - t.0).abs()).max().unwrap()
}

#[test]
fn quick_look_then_full_read() {
    let required = std::env::var("SUBMAGICIAN_REQUIRE_FFMPEG").is_ok_and(|v| v == "1");
    let Some(ffmpeg) = audio::find_ffmpeg(None) else {
        assert!(!required, "ffmpeg not found");
        return;
    };
    let dir = std::env::temp_dir().join(format!("submagician-autosync-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let Some(truth) = build(&ffmpeg, &dir) else {
        assert!(!required, "could not build the test video (ffmpeg without flite?)");
        return;
    };
    let video = dir.join("film.mkv");
    autosync::forget(&video);
    let no = AtomicBool::new(false);

    // 25 fps subtitle on a 23.976 fps film, 3 s late: one shift fits everywhere.
    let sub = dir.join("film.en.srt");
    let wrong: Vec<(i64, i64)> = truth
        .iter()
        .map(|&(s, e)| ((s as f64 * 25.0 / 23.976) as i64 + 3_000, (e as f64 * 25.0 / 23.976) as i64 + 3_000))
        .collect();
    write_srt(&sub, &wrong);
    let started = std::time::Instant::now();
    let (report, method) = autosync::sync_to_audio(&sub, &video, &ffmpeg, &no, &mut |_| {}).unwrap();
    eprintln!("quick: {report:?} in {:?}", started.elapsed());
    assert_eq!(method, Method::Quick, "{report:?}");
    assert!(report.applied && (report.ratio - 23.976 / 25.0).abs() < 1e-9, "{report:?}");
    let err = max_error(&sub, &truth);
    assert!(err < 400, "max error {err} ms");

    // 15 s added in the middle of the subtitle's release: needs splits, so the whole audio.
    let middle = truth[truth.len() / 2].0;
    let cut: Vec<(i64, i64)> =
        truth.iter().map(|&(s, e)| if s >= middle { (s + 15_000, e + 15_000) } else { (s, e) }).collect();
    write_srt(&sub, &cut);
    let started = std::time::Instant::now();
    let (report, method) = autosync::sync_to_audio(&sub, &video, &ffmpeg, &no, &mut |_| {}).unwrap();
    eprintln!("full: {report:?} in {:?}", started.elapsed());
    assert_eq!(method, Method::Full, "{report:?}");
    assert!(report.applied && report.splits >= 1, "{report:?}");
    let err = max_error(&sub, &truth);
    assert!(err < 600, "max error {err} ms");
    assert!(autosync::cached(&video).is_some(), "speech kept for next time");

    // Same film again: the kept speech, no decoding.
    write_srt(&sub, &cut);
    let (_, method) = autosync::sync_to_audio(&sub, &video, &ffmpeg, &no, &mut |_| {}).unwrap();
    assert_eq!(method, Method::Cached);
    autosync::forget(&video);
    std::fs::remove_dir_all(&dir).unwrap();
}
