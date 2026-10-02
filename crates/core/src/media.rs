//! Finding videos in a folder and the subtitles already sitting next to them.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::lang;

pub const VIDEO_EXTS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "wmv", "mpg", "mpeg", "ts", "m2ts", "webm", "flv", "ogm", "divx", "vob", "3gp",
];
pub const SUBTITLE_EXTS: &[&str] = &["srt", "ass", "ssa", "sub", "vtt", "smi"];

pub fn is_video_ext(ext: &str) -> bool {
    VIDEO_EXTS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

pub fn is_subtitle_ext(ext: &str) -> bool {
    SUBTITLE_EXTS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

#[derive(Debug, Clone)]
pub struct ExistingSubtitle {
    pub path: PathBuf,
    /// Language code from the file name (`movie.tr.srt`), if it has one.
    pub language: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub struct MediaFile {
    pub path: PathBuf,
    pub size: u64,
    pub existing: Vec<ExistingSubtitle>,
}

impl MediaFile {
    pub fn file_name(&self) -> String {
        self.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub fn has_language(&self, code: &str) -> bool {
        self.existing.iter().any(|s| s.language == Some(code))
    }
}

/// Lists the videos under `dir` (sorted by path), skipping samples and trailers.
pub fn scan(dir: &Path, recursive: bool) -> Vec<MediaFile> {
    let walker = WalkDir::new(dir).max_depth(if recursive { usize::MAX } else { 1 }).follow_links(true);
    let mut files: Vec<MediaFile> = walker
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().is_some_and(|x| is_video_ext(&x.to_string_lossy())))
        .filter_map(|e| {
            let size = e.metadata().ok()?.len();
            if is_extra(e.path(), size) {
                return None;
            }
            Some(MediaFile { path: e.path().to_path_buf(), size, existing: existing_subtitles(e.path()) })
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

fn is_extra(path: &Path, size: u64) -> bool {
    const SMALL: u64 = 300 * 1024 * 1024;
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let in_extra_dir = path
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|d| matches!(d.to_string_lossy().to_lowercase().as_str(), "sample" | "samples" | "trailers"));
    size < SMALL && (in_extra_dir || stem == "sample" || stem.ends_with("-sample") || stem.ends_with(".sample"))
}

/// Subtitles next to `video` named `<stem>.<ext>` or `<stem>.<tags>.<ext>`.
pub fn existing_subtitles(video: &Path) -> Vec<ExistingSubtitle> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem()) else {
        return Vec::new();
    };
    let stem = stem.to_string_lossy().to_lowercase();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut subs: Vec<ExistingSubtitle> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_string_lossy().into_owned();
            if !is_subtitle_ext(&ext) {
                return None;
            }
            let name = path.file_stem()?.to_string_lossy().to_lowercase();
            let rest = name.strip_prefix(&stem)?;
            if !(rest.is_empty() || rest.starts_with('.') || rest.starts_with('_') || rest.starts_with('-')) {
                return None;
            }
            let language =
                rest.split(['.', '_', '-']).filter(|t| !t.is_empty()).find_map(|t| lang::find(t)).map(|l| l.code);
            Some(ExistingSubtitle { path, language })
        })
        .collect();
    subs.sort_by(|a, b| a.path.cmp(&b.path));
    subs
}

/// Language tag in a subtitle file name: `Film.tr.srt`, `Film.eng.forced.srt` → code.
pub fn language_of(subtitle: &Path) -> Option<&'static str> {
    let stem = subtitle.file_stem()?.to_string_lossy().to_lowercase();
    stem.rsplit(['.', '_', '-']).take(3).find_map(|t| lang::find(t)).map(|l| l.code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path, size: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(path).unwrap().set_len(size).unwrap();
    }

    #[test]
    fn scans_videos_and_their_subtitles() {
        let dir = std::env::temp_dir().join(format!("submagician-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        touch(&dir.join("Film.2020.mkv"), 1000);
        touch(&dir.join("Film.2020.tr.srt"), 10);
        touch(&dir.join("Film.2020.eng.forced.srt"), 10);
        touch(&dir.join("Film.2020.srt"), 10);
        touch(&dir.join("Film.20201.srt"), 10); // a different file's subtitle
        touch(&dir.join("notes.txt"), 10);
        touch(&dir.join("Sample/film-sample.mkv"), 1000);
        touch(&dir.join("Show/Show.S01E01.mp4"), 1000);

        let flat = scan(&dir, false);
        assert_eq!(flat.len(), 1);
        let film = &flat[0];
        let langs: Vec<_> = film.existing.iter().map(|s| s.language).collect();
        assert_eq!(film.existing.len(), 3, "{:?}", film.existing);
        assert!(langs.contains(&Some("tr")) && langs.contains(&Some("en")) && langs.contains(&None));
        assert!(film.has_language("tr"));

        assert_eq!(language_of(Path::new("a/Film.2020.eng.forced.srt")), Some("en"));
        assert_eq!(language_of(Path::new("a/Film.2020.srt")), None);

        let all = scan(&dir, true);
        assert_eq!(all.len(), 2, "samples are skipped");
        fs::remove_dir_all(&dir).unwrap();
    }
}
