//! Ranking candidates: which subtitle is most likely made for *this* file.
//!
//! A hash match is the strongest signal, but uploaders sometimes attach subtitles to the wrong
//! hash, so the release name still counts: same release group and source usually means the
//! same cut and frame rate.

use crate::name::{self, ParsedName};
use crate::provider::Candidate;

/// Score below which a candidate is treated as wrong (another episode).
pub const REJECT: i32 = -500;

pub fn score(video_name: &str, video: &ParsedName, c: &Candidate) -> i32 {
    let release = if c.release.is_empty() { c.file_name.as_deref().unwrap_or_default() } else { &c.release };
    let rel = name::parse(release);
    let mut s = 0;

    if c.hash_match {
        s += 100;
    }
    if video.is_episode {
        let season = c.season.or(rel.season);
        let episode = c.episode.or(rel.episode);
        if differs(video.season, season) || differs(video.episode, episode) {
            return REJECT * 2;
        }
    }
    if same(&video.release_group, &rel.release_group) {
        s += 40;
    }
    match (&video.source, &rel.source) {
        (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => s += 25,
        (Some(_), Some(_)) => s -= 10,
        _ => {}
    }
    if same(&video.streaming_service, &rel.streaming_service) {
        s += 10;
    }
    if same(&video.screen_size, &rel.screen_size) {
        s += 5;
    }
    s += (similarity(video_name, release) * 30.0).round() as i32;

    if c.trusted {
        s += 5;
    }
    if c.machine_translated {
        s -= 40;
    }
    if c.hearing_impaired {
        s -= 3;
    }
    s += ((c.downloads as f64 + 1.0).log10() * 4.0).round() as i32;
    s += c.rating.clamp(0.0, 10.0).round() as i32;
    s
}

fn differs(a: Option<i32>, b: Option<i32>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a != b)
}

fn same(a: &Option<String>, b: &Option<String>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a.eq_ignore_ascii_case(b))
}

/// Jaccard similarity of the two names' tokens, 0.0 ..= 1.0.
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (name::tokens(a), name::tokens(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    a.intersection(&b).count() as f64 / a.union(&b).count() as f64
}

/// Scores `candidates` and sorts them: wanted languages in order, then score, best first.
pub fn rank(video_name: &str, video: &ParsedName, languages: &[String], candidates: &mut [Candidate]) {
    for c in candidates.iter_mut() {
        c.score = score(video_name, video, c);
    }
    let lang_rank = |c: &Candidate| languages.iter().position(|l| *l == c.language).unwrap_or(usize::MAX);
    candidates.sort_by(|a, b| {
        lang_rank(a).cmp(&lang_rank(b)).then(b.score.cmp(&a.score)).then(b.downloads.cmp(&a.downloads))
    });
}

/// The best usable candidate in the most wanted language that has one.
pub fn best(candidates: &[Candidate], languages: &[String]) -> Option<usize> {
    languages.iter().find_map(|lang| {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.language == *lang && c.score > REJECT)
            .max_by_key(|(_, c)| c.score)
            .map(|(i, _)| i)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(lang: &str, release: &str) -> Candidate {
        Candidate { language: lang.into(), release: release.into(), ..Default::default() }
    }

    #[test]
    fn same_release_beats_popular_other_release() {
        let file = "Inception.2010.1080p.BluRay.x264-SPARKS.mkv";
        let video = name::parse(file);
        let langs = vec!["tr".to_string()];
        let mut list = vec![
            Candidate { downloads: 90_000, ..cand("tr", "Inception.2010.720p.WEB-DL.x264-OTHER") },
            cand("tr", "Inception.2010.1080p.BluRay.x264-SPARKS"),
        ];
        rank(file, &video, &langs, &mut list);
        assert_eq!(list[0].release, "Inception.2010.1080p.BluRay.x264-SPARKS");
        assert_eq!(best(&list, &langs), Some(0));
    }

    #[test]
    fn hash_match_wins_and_machine_translation_loses() {
        let file = "Film.2020.mkv";
        let video = name::parse(file);
        let a = Candidate { hash_match: true, ..cand("tr", "whatever") };
        let b = Candidate { machine_translated: true, ..cand("tr", "Film.2020") };
        assert!(score(file, &video, &a) > score(file, &video, &b));
    }

    #[test]
    fn wrong_episode_is_rejected() {
        let file = "Show.S02E05.720p.HDTV.mkv";
        let video = name::parse(file);
        let langs = vec!["tr".to_string()];
        let mut list = vec![cand("tr", "Show.S02E06.720p.HDTV"), Candidate { episode: Some(4), ..cand("tr", "Show") }];
        rank(file, &video, &langs, &mut list);
        assert!(list.iter().all(|c| c.score <= REJECT));
        assert_eq!(best(&list, &langs), None);
    }

    #[test]
    fn falls_back_to_next_language() {
        let file = "Film.2020.mkv";
        let video = name::parse(file);
        let langs = vec!["tr".to_string(), "en".to_string()];
        let mut list = vec![cand("en", "Film.2020"), cand("de", "Film.2020")];
        rank(file, &video, &langs, &mut list);
        assert_eq!(list[0].language, "en");
        assert_eq!(best(&list, &langs), Some(0));
    }
}
