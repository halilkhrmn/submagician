//! What a file or release name says: title, year, episode, source, release group.

use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedName {
    pub title: Option<String>,
    pub year: Option<i32>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub episode_title: Option<String>,
    /// "Blu-ray", "Web", "HDTV", … as normalized by hunch.
    pub source: Option<String>,
    pub screen_size: Option<String>,
    pub release_group: Option<String>,
    pub streaming_service: Option<String>,
    pub is_episode: bool,
}

pub fn parse(name: &str) -> ParsedName {
    let r = hunch::hunch(name);
    let own = |v: Option<&str>| v.map(str::to_owned);
    ParsedName {
        title: own(r.title()),
        year: r.year(),
        season: r.season(),
        episode: r.episode(),
        episode_title: own(r.episode_title()),
        source: own(r.source()),
        screen_size: own(r.screen_size()),
        release_group: own(r.release_group()),
        streaming_service: own(r.streaming_service()),
        is_episode: r.is_episode() || r.episode().is_some(),
    }
}

/// Parses a video path. When the file name has no usable title (`movie.mkv`, `CD1.avi`) the
/// parent folder name usually has it, so its title and year fill the gaps.
pub fn parse_path(path: &Path) -> ParsedName {
    let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let mut parsed = parse(&file);
    let weak_title = parsed.title.as_deref().is_none_or(|t| t.chars().count() < 3 || is_generic(t));
    if (weak_title || parsed.year.is_none())
        && let Some(dir) = path.parent().and_then(|p| p.file_name())
    {
        let from_dir = parse(&dir.to_string_lossy());
        if weak_title && from_dir.title.is_some() {
            parsed.title = from_dir.title;
        }
        if parsed.year.is_none() && !parsed.is_episode {
            parsed.year = from_dir.year;
        }
        if parsed.release_group.is_none() {
            parsed.release_group = from_dir.release_group;
        }
        if parsed.source.is_none() {
            parsed.source = from_dir.source;
        }
    }
    parsed
}

fn is_generic(title: &str) -> bool {
    let t = title.to_ascii_lowercase();
    ["movie", "film", "video", "cd", "disc", "sample"].iter().any(|g| t == *g || t.starts_with(&format!("{g} ")))
}

/// Lowercase alphanumeric tokens of a release name, for similarity scoring.
pub fn tokens(name: &str) -> BTreeSet<String> {
    let stem = strip_known_extension(name);
    stem.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_lowercase).collect()
}

fn strip_known_extension(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext)) if crate::media::is_video_ext(ext) || crate::media::is_subtitle_ext(ext) => stem,
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_movie() {
        let p = parse("Inception.2010.1080p.BluRay.x264-SPARKS.mkv");
        assert_eq!(p.title.as_deref(), Some("Inception"));
        assert_eq!(p.year, Some(2010));
        assert_eq!(p.release_group.as_deref(), Some("SPARKS"));
        assert_eq!(p.source.as_deref(), Some("Blu-ray"));
        assert!(!p.is_episode);
    }

    #[test]
    fn parses_episode() {
        let p = parse("The.Office.US.S03E07.720p.WEB-DL.mkv");
        assert_eq!(p.season, Some(3));
        assert_eq!(p.episode, Some(7));
        assert!(p.is_episode);
    }

    #[test]
    fn falls_back_to_folder() {
        let p = parse_path(Path::new("/films/Inception.2010.1080p.BluRay.x264-SPARKS/movie.mkv"));
        assert_eq!(p.title.as_deref(), Some("Inception"));
        assert_eq!(p.year, Some(2010));
    }

    #[test]
    fn tokenizes() {
        let t = tokens("Inception.2010.1080p.BluRay.x264-SPARKS.srt");
        assert!(t.contains("sparks") && t.contains("bluray") && !t.contains("srt"));
    }
}
