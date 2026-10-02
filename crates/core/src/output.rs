//! Writing the chosen subtitle next to the video, as `<video stem>.<lang>.<ext>` in UTF-8.
//! VLC, mpv, MPC-HC, Kodi, Plex and Jellyfin all pick that name up on their own.

use std::fs;
use std::path::{Path, PathBuf};

use crate::Result;

pub fn subtitle_path(video: &Path, language: &str, ext: &str) -> PathBuf {
    let stem = video.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    video.with_file_name(format!("{stem}.{language}.{ext}"))
}

/// Writes `text` as UTF-8 with a BOM (old players and TVs need it to see UTF-8; newer ones
/// ignore it). An existing file at the target is kept once as `<name>.bak`.
pub fn write_subtitle(video: &Path, language: &str, ext: &str, text: &str) -> Result<PathBuf> {
    let path = subtitle_path(video, language, ext);
    if path.exists() {
        let mut backup = path.clone().into_os_string();
        backup.push(".bak");
        let backup = PathBuf::from(backup);
        if !backup.exists() {
            fs::rename(&path, &backup)?;
        }
    }
    let mut data = Vec::with_capacity(text.len() + 3);
    data.extend_from_slice(b"\xEF\xBB\xBF");
    data.extend_from_slice(text.replace('\n', "\r\n").as_bytes());
    let tmp = path.with_extension(format!("{ext}.part"));
    fs::write(&tmp, &data)?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_next_to_video() {
        let p = subtitle_path(Path::new("/m/Film.2020.1080p.mkv"), "tr", "srt");
        assert_eq!(p, Path::new("/m/Film.2020.1080p.tr.srt"));
    }

    #[test]
    fn writes_with_bom_and_backs_up_once() {
        let dir = std::env::temp_dir().join(format!("submagician-out-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let video = dir.join("Film.mkv");
        let first = write_subtitle(&video, "tr", "srt", "1\nbir\n").unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"\xEF\xBB\xBF1\r\nbir\r\n");
        write_subtitle(&video, "tr", "srt", "2\n").unwrap();
        write_subtitle(&video, "tr", "srt", "3\n").unwrap();
        assert_eq!(fs::read(dir.join("Film.tr.srt.bak")).unwrap(), b"\xEF\xBB\xBF1\r\nbir\r\n");
        assert_eq!(fs::read(&first).unwrap(), b"\xEF\xBB\xBF3\r\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}
