//! Providers hand out plain subtitle files or zip archives; this gets the subtitle files out.

use std::io::{Cursor, Read};

use crate::media::is_subtitle_ext;
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct SubtitleFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

const MAX_ENTRY: u64 = 20 * 1024 * 1024;

/// Returns the subtitle files in `bytes`: the entries of a zip, or `bytes` itself.
pub fn extract(name: &str, bytes: Vec<u8>) -> Result<Vec<SubtitleFile>> {
    if bytes.starts_with(b"PK\x03\x04") {
        return unzip(&bytes);
    }
    if bytes.starts_with(b"Rar!") || bytes.starts_with(b"7z\xBC\xAF") {
        return Err(Error::Archive("RAR and 7z archives are not supported yet".into()));
    }
    Ok(vec![SubtitleFile { name: name.to_owned(), bytes }])
}

fn unzip(bytes: &[u8]) -> Result<Vec<SubtitleFile>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Archive(e.to_string()))?;
    let mut files = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| Error::Archive(e.to_string()))?;
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().rsplit(['/', '\\']).next().unwrap_or_default().to_owned();
        let is_sub = name.rsplit_once('.').is_some_and(|(_, ext)| is_subtitle_ext(ext));
        if !is_sub || entry.size() > MAX_ENTRY {
            continue;
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.by_ref().take(MAX_ENTRY).read_to_end(&mut data)?;
        files.push(SubtitleFile { name, bytes: data });
    }
    if files.is_empty() { Err(Error::NoSubtitleInArchive) } else { Ok(files) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn plain_file_passes_through() {
        let files = extract("a.srt", b"1\n".to_vec()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "a.srt");
    }

    #[test]
    fn unzips_only_subtitles() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("Show.S01E01.srt", opts).unwrap();
            w.write_all(b"1\n00:00:01,000 --> 00:00:02,000\nhi\n").unwrap();
            w.start_file("dir/Show.S01E02.srt", opts).unwrap();
            w.write_all(b"x").unwrap();
            w.start_file("readme.nfo", opts).unwrap();
            w.write_all(b"ads").unwrap();
            w.finish().unwrap();
        }
        let files = extract("x.zip", buf.into_inner()).unwrap();
        let names: Vec<_> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Show.S01E01.srt", "Show.S01E02.srt"]);
    }
}
