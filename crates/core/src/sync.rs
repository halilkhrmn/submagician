//! Fitting subtitle timings to a reference: the speech in the video's audio, or another subtitle
//! that is already in sync. Alignment is done by alass (offsets, splits for cut/added scenes);
//! a frame-rate mismatch (23.976 vs 25 …) is found first by trying the common ratios.

use std::path::Path;

use alass_core::{NoProgressHandler, TimeDelta, TimePoint, TimeSpan};

use crate::timing::Document;
use crate::{Result, media, output, text};

/// A stretch of time in milliseconds, `start < end`.
pub type Span = (i64, i64);

/// Frame-rate ratios to try: none, PAL speed-up/slow-down, 24 vs 23.976 and 25 vs 24.
const RATIOS: &[f64] = &[1.0, 25.0 / 23.976, 23.976 / 25.0, 24.0 / 23.976, 23.976 / 24.0, 25.0 / 24.0, 24.0 / 25.0];

/// Split penalty passed to alass; its documentation suggests 7 for normal use.
const SPLIT_PENALTY: f64 = 7.0;
/// Speed optimization passed to alass (higher is faster, less exact).
const SPEED: f64 = 1.0;
/// A sync must raise the speech overlap by this much to be kept.
const MIN_GAIN: f32 = 0.02;

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Shift of the first line, in milliseconds.
    pub offset_ms: i64,
    /// Time scale applied for a frame-rate mismatch (1.0 = none).
    pub ratio: f64,
    /// Number of places where the shift changes (cut or added scenes).
    pub splits: usize,
    /// Share of subtitle time that falls on reference speech/lines, before and after.
    pub overlap_before: f32,
    pub overlap_after: f32,
    /// `false` when the result was not better than the original, so nothing was changed.
    pub applied: bool,
}

impl Report {
    /// "+4.00 s" and, for a frame-rate fix, "· 25 → 23.976 fps" (numbers only, no words).
    pub fn summary(&self) -> String {
        let mut out = format!("{:+.2} s", self.offset_ms as f64 / 1000.0);
        if (self.ratio - 1.0).abs() > 1e-6 {
            const RATES: [f64; 3] = [23.976, 24.0, 25.0];
            let pair = RATES
                .iter()
                .flat_map(|a| RATES.iter().map(move |b| (*a, *b)))
                .find(|(a, b)| (a / b - self.ratio).abs() < 1e-6);
            out += &match pair {
                Some((a, b)) => format!(" · {a} → {b} fps"),
                None => format!(" · ×{:.4}", self.ratio),
            };
        }
        out
    }
}

/// Aligns `doc` to `reference` in place. Leaves `doc` untouched (and `applied: false`) when the
/// alignment does not fit the reference better than the original timing.
pub fn align(doc: &mut Document, reference: &[Span]) -> Report {
    let original: Vec<Span> = doc.cues.iter().map(|c| (c.start, c.end.max(c.start + 1))).collect();
    let reference = merge(reference.to_vec());
    let before = overlap(&original, &reference);
    let ref_spans: Vec<TimeSpan> = reference.iter().map(|&(s, e)| span(s, e)).collect();

    // 1. Frame rate: the ratio whose best single shift fits the reference best.
    let ratio = RATIOS
        .iter()
        .copied()
        .map(|r| {
            let scaled: Vec<TimeSpan> = original.iter().map(|&(s, e)| scaled_span(s, e, r)).collect();
            let (_, score) =
                alass_core::align_nosplit(&ref_spans, &scaled, alass_core::overlap_scoring, NoProgressHandler);
            (r, score)
        })
        .fold((1.0, f64::MIN), |best, cur| if cur.1 > best.1 * 1.01 { cur } else { best })
        .0;

    // 2. Offsets, with splits where scenes were cut or added.
    let scaled: Vec<TimeSpan> = original.iter().map(|&(s, e)| scaled_span(s, e, ratio)).collect();
    let (deltas, _) = alass_core::align(
        &ref_spans,
        &scaled,
        SPLIT_PENALTY,
        Some(SPEED),
        alass_core::standard_scoring,
        NoProgressHandler,
    );
    let deltas: Vec<i64> = deltas.iter().map(TimeDelta::as_i64).collect();

    let synced: Vec<Span> =
        original.iter().zip(&deltas).map(|(&(s, e), d)| (scale(s, ratio) + d, scale(e, ratio) + d)).collect();
    let after = overlap(&synced, &reference);
    let splits = deltas.windows(2).filter(|w| w[0] != w[1]).count();
    let offset_ms = synced.first().zip(original.first()).map_or(0, |(a, b)| a.0 - b.0);
    let applied = after >= before + MIN_GAIN;
    if applied {
        doc.map_times(|i, t| scale(t, ratio) + deltas[i]);
    }
    Report { offset_ms, ratio, splits, overlap_before: before, overlap_after: after, applied }
}

/// The cue spans of an in-sync subtitle, for use as a reference.
pub fn reference_from(doc: &Document) -> Vec<Span> {
    doc.cues.iter().filter(|c| c.end > c.start).map(|c| (c.start, c.end)).collect()
}

fn load(path: &Path) -> Result<Document> {
    let bytes = std::fs::read(path)?;
    Document::parse(&text::decode(&bytes, media::language_of(path)).text)
}

/// Syncs the subtitle file at `path` to `reference` and rewrites it when the result is better.
pub fn sync_file(path: &Path, reference: &[Span]) -> Result<Report> {
    let mut doc = load(path)?;
    let report = align(&mut doc, reference);
    if report.applied {
        output::rewrite(path, &doc.render())?;
    }
    Ok(report)
}

/// Moves every line of the subtitle file at `path` by `ms` milliseconds.
pub fn shift_file(path: &Path, ms: i64) -> Result<()> {
    let mut doc = load(path)?;
    doc.shift(ms);
    output::rewrite(path, &doc.render())
}

/// Lines of an in-sync subtitle file, as a reference for [`sync_file`].
pub fn reference_from_file(path: &Path) -> Result<Vec<Span>> {
    Ok(reference_from(&load(path)?))
}

fn scale(t: i64, ratio: f64) -> i64 {
    (t as f64 * ratio).round() as i64
}

fn span(s: i64, e: i64) -> TimeSpan {
    TimeSpan::new_safe(TimePoint::from(s), TimePoint::from(e))
}

fn scaled_span(s: i64, e: i64, ratio: f64) -> TimeSpan {
    span(scale(s, ratio), scale(e, ratio).max(scale(s, ratio) + 1))
}

/// Sorts and joins overlapping spans.
pub fn merge(mut spans: Vec<Span>) -> Vec<Span> {
    spans.retain(|(s, e)| e > s);
    spans.sort_unstable();
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for (s, e) in spans {
        match out.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// Share of the total length of `spans` that lies inside `reference` (merged, sorted).
pub fn overlap(spans: &[Span], reference: &[Span]) -> f32 {
    let total: i64 = spans.iter().map(|(s, e)| (e - s).max(0)).sum();
    if total == 0 {
        return 0.0;
    }
    let covered: i64 = spans
        .iter()
        .map(|&(s, e)| {
            let first = reference.partition_point(|r| r.1 <= s);
            reference[first..].iter().take_while(|r| r.0 < e).map(|r| (e.min(r.1) - s.max(r.0)).max(0)).sum::<i64>()
        })
        .sum();
    covered as f32 / total as f32
}

/// Turns per-frame voice decisions into speech spans. `frame_ms` is the frame length; gaps
/// shorter than `bridge_ms` are closed and spans shorter than `min_ms` dropped.
pub fn frames_to_spans(voiced: &[bool], frame_ms: i64, bridge_ms: i64, min_ms: i64) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut start = None;
    for (i, &v) in voiced.iter().chain(std::iter::once(&false)).enumerate() {
        let t = i as i64 * frame_ms;
        match (v, start) {
            (true, None) => start = Some(t),
            (false, Some(s)) => {
                match spans.last_mut() {
                    Some(last) if s - last.1 < bridge_ms => last.1 = t,
                    _ => spans.push((s, t)),
                }
                start = None;
            }
            _ => {}
        }
    }
    spans.retain(|(s, e)| e - s >= min_ms);
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A subtitle with irregular lines over ~10 minutes.
    fn sample_srt(lines: usize) -> String {
        let mut out = String::new();
        let mut t = 3_000i64;
        for i in 0..lines {
            let len = 900 + (i as i64 * 7919 % 2600);
            let gap = 400 + (i as i64 * 104_729 % 5200);
            let clock = |ms: i64| {
                format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
            };
            out += &format!("{}\n{} --> {}\nline {i}\n\n", i + 1, clock(t), clock(t + len));
            t += len + gap;
        }
        out
    }

    #[test]
    fn finds_a_plain_offset() {
        let truth = Document::parse(&sample_srt(150)).unwrap();
        let speech = reference_from(&truth);
        let mut doc = truth.clone();
        doc.shift(-2_350);
        let report = align(&mut doc, &speech);
        assert!(report.applied);
        assert_eq!(report.ratio, 1.0);
        assert!((report.offset_ms - 2_350).abs() <= 20, "{report:?}");
        assert!(report.overlap_after > 0.95 && report.overlap_before < 0.8, "{report:?}");
    }

    #[test]
    fn finds_frame_rate_and_offset() {
        let truth = Document::parse(&sample_srt(200)).unwrap();
        let speech = reference_from(&truth);
        // A 25 fps subtitle on a 23.976 fps video: times are 23.976/25 of the truth, plus 1 s.
        let mut doc = truth.clone();
        doc.map_times(|_, t| (t as f64 * 23.976 / 25.0).round() as i64 + 1_000);
        let report = align(&mut doc, &speech);
        assert!(report.applied, "{report:?}");
        assert!((report.ratio - 25.0 / 23.976).abs() < 1e-9, "{report:?}");
        assert!(report.overlap_after > 0.9, "{report:?}");
        let err = doc.cues.iter().zip(&truth.cues).map(|(a, b)| (a.start - b.start).abs()).max().unwrap();
        assert!(err <= 60, "max error {err} ms");
    }

    #[test]
    fn handles_a_cut_scene() {
        let truth = Document::parse(&sample_srt(200)).unwrap();
        let speech = reference_from(&truth);
        // The subtitle's release has 30 s more in the middle (an intro or recap).
        let mut doc = truth.clone();
        let middle = doc.cues[100].start;
        doc.map_times(|_, t| if t >= middle { t + 30_000 } else { t });
        let report = align(&mut doc, &speech);
        assert!(report.applied && report.splits >= 1, "{report:?}");
        assert!(report.overlap_after > 0.95, "{report:?}");
    }

    #[test]
    fn keeps_an_already_synced_subtitle() {
        let truth = Document::parse(&sample_srt(80)).unwrap();
        let speech = reference_from(&truth);
        let mut doc = truth.clone();
        let report = align(&mut doc, &speech);
        assert!(!report.applied);
        assert_eq!(doc.render(), truth.render());
    }

    #[test]
    fn summarizes_reports() {
        let r =
            Report { offset_ms: -3_800, ratio: 1.0, splits: 0, overlap_before: 0.2, overlap_after: 0.8, applied: true };
        assert_eq!(r.summary(), "-3.80 s");
        let r = Report { offset_ms: 1_000, ratio: 25.0 / 23.976, ..r };
        assert_eq!(r.summary(), "+1.00 s · 25 → 23.976 fps");
    }

    #[test]
    fn merges_and_measures_overlap() {
        assert_eq!(merge(vec![(5, 9), (0, 3), (2, 4), (9, 9)]), vec![(0, 4), (5, 9)]);
        let r = merge(vec![(0, 10), (20, 30)]);
        assert_eq!(overlap(&[(5, 25)], &r), 0.5);
        assert_eq!(overlap(&[(10, 20)], &r), 0.0);
    }

    #[test]
    fn frames_become_spans() {
        let v: Vec<bool> = "0011101100000111".chars().map(|c| c == '1').collect();
        // 10 ms frames, bridge gaps < 20 ms, drop spans < 20 ms
        assert_eq!(frames_to_spans(&v, 10, 20, 20), vec![(20, 80), (130, 160)]);
    }
}
