//! Providers hand out plain subtitle files or zip archives; this gets the subtitle files out.

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
        return extract_with_tool(&bytes);
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

/// Archive tools that read RAR and 7z, in order of preference. Windows 10+ ships `tar.exe`,
/// which is bsdtar (libarchive) and reads both; GNU tar does not, so `tar` is only used when it
/// says it is bsdtar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    BsdTar,
    SevenZip,
    UnRar,
}

fn find_tool() -> Option<(PathBuf, Tool)> {
    let path = std::env::var_os("PATH")?;
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    let find = |name: &str| {
        let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
        dirs.iter().map(|d| d.join(&file)).find(|p| p.is_file())
    };
    if let Some(p) = find("bsdtar") {
        return Some((p, Tool::BsdTar));
    }
    if let Some(p) = find("tar").filter(|p| is_bsdtar(p)) {
        return Some((p, Tool::BsdTar));
    }
    for (name, tool) in [("7z", Tool::SevenZip), ("7za", Tool::SevenZip), ("unrar", Tool::UnRar)] {
        if let Some(p) = find(name) {
            return Some((p, tool));
        }
    }
    None
}

fn is_bsdtar(tar: &Path) -> bool {
    quiet(tar).arg("--version").output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("bsdtar"))
}

fn quiet(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    cmd
}

/// Unpacks a RAR or 7z archive with an external tool into a temporary folder and reads the
/// subtitle files from it.
fn extract_with_tool(bytes: &[u8]) -> Result<Vec<SubtitleFile>> {
    let Some((tool_path, tool)) = find_tool() else {
        return Err(Error::Archive("RAR/7z archive: install 7-Zip or bsdtar to open it".into()));
    };
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let dir = std::env::temp_dir().join(format!("submagician-{}-{stamp}", std::process::id()));
    let out = dir.join("out");
    fs::create_dir_all(&out)?;
    let result = (|| {
        let archive = dir.join("archive");
        fs::write(&archive, bytes)?;
        let mut cmd = quiet(&tool_path);
        match tool {
            Tool::BsdTar => cmd.arg("-x").arg("-f").arg(&archive).arg("-C").arg(&out),
            Tool::SevenZip => cmd.args(["x", "-y", "-bd"]).arg(format!("-o{}", out.display())).arg(&archive),
            Tool::UnRar => cmd.args(["x", "-y", "-inul"]).arg(&archive).arg(&out),
        };
        let output = cmd.output()?;
        if !output.status.success() {
            let msg = String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or_default().to_owned();
            return Err(Error::Archive(format!("{} failed: {msg}", tool_path.display())));
        }
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(&out).into_iter().filter_map(std::result::Result::ok) {
            let path = entry.path();
            let is_sub = path.extension().is_some_and(|e| is_subtitle_ext(&e.to_string_lossy()));
            if entry.file_type().is_file() && is_sub && entry.metadata().is_ok_and(|m| m.len() <= MAX_ENTRY) {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                files.push(SubtitleFile { name, bytes: fs::read(path)? });
            }
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        if files.is_empty() { Err(Error::NoSubtitleInArchive) } else { Ok(files) }
    })();
    let _ = fs::remove_dir_all(&dir);
    result
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

    /// A 7z archive (bsdtar can write those; nothing free writes RAR) goes through the same
    /// external-tool path as RAR. Needs bsdtar; skipped otherwise unless
    /// `SUBMAGICIAN_REQUIRE_BSDTAR=1` (set in CI on Linux).
    #[test]
    fn unpacks_7z_with_external_tool() {
        let required = std::env::var("SUBMAGICIAN_REQUIRE_BSDTAR").is_ok_and(|v| v == "1");
        if !matches!(find_tool(), Some((_, Tool::BsdTar | Tool::SevenZip))) {
            assert!(!required, "bsdtar not found");
            return;
        }
        let dir = std::env::temp_dir().join(format!("submagician-7z-{}", std::process::id()));
        let src = dir.join("src");
        fs::create_dir_all(src.join("Subs")).unwrap();
        fs::write(src.join("Subs/Film.tr.srt"), "1\n00:00:01,000 --> 00:00:02,000\nSelam\n").unwrap();
        fs::write(src.join("info.nfo"), "x").unwrap();
        let archive = dir.join("a.7z");
        let ok = Command::new("bsdtar")
            .args(["--format", "7zip", "-c", "-f"])
            .arg(&archive)
            .arg("-C")
            .arg(&src)
            .args(["Subs", "info.nfo"])
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            assert!(!required, "bsdtar could not write 7z");
            return;
        }
        let files = extract("a.7z", fs::read(&archive).unwrap()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "Film.tr.srt");
        assert!(String::from_utf8_lossy(&files[0].bytes).contains("Selam"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
