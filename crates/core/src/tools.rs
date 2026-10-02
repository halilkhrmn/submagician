//! ffmpeg for Windows, on request: Windows has no package manager that every user has, so
//! Settings can download the "essentials" build from gyan.dev (the build ffmpeg.org links to),
//! check it against the published SHA-256, and keep only ffmpeg.exe and ffprobe.exe.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::{Error, Result};

const ZIP_URL: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";
const SHA_URL: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256";

/// Where downloaded tools live (searched by [`crate::audio::find_ffmpeg`]).
pub fn tools_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "SubMagician").map(|d| d.data_dir().join("tools"))
}

/// Downloads ffmpeg and ffprobe for Windows into `dir`. `progress` gets (bytes, total).
pub async fn download_ffmpeg(
    dir: &Path,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let client = reqwest::Client::builder().user_agent(crate::provider::user_agent()).build()?;
    let expected = client.get(SHA_URL).send().await?.error_for_status()?.text().await?;
    let expected = expected.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    if expected.len() != 64 {
        return Err(Error::Parse("no checksum for the ffmpeg download".into()));
    }
    let zip_path = dir.join("ffmpeg.zip.part");
    let mut resp = client.get(ZIP_URL).send().await?.error_for_status()?;
    let total = resp.content_length();
    let mut file = std::fs::File::create(&zip_path)?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    while let Some(chunk) = resp.chunk().await? {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&zip_path);
            return Err(Error::Cancelled);
        }
        hasher.update(&chunk);
        std::io::Write::write_all(&mut file, &chunk)?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    drop(file);
    let actual = hex(&hasher.finalize());
    let result = if actual != expected {
        Err(Error::Parse(format!("ffmpeg download damaged (checksum {actual}, expected {expected})")))
    } else {
        extract_ffmpeg(&zip_path, dir)
    };
    let _ = std::fs::remove_file(&zip_path);
    result
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Copies `…/bin/ffmpeg.exe` and `…/bin/ffprobe.exe` out of the zip into `dir`.
fn extract_ffmpeg(zip_path: &Path, dir: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(zip_path)?).map_err(|e| Error::Archive(e.to_string()))?;
    let mut found = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| Error::Archive(e.to_string()))?;
        let name = entry.name().replace('\\', "/");
        let Some(file) = ["ffmpeg.exe", "ffprobe.exe"].into_iter().find(|f| name.ends_with(&format!("/bin/{f}")))
        else {
            continue;
        };
        let part = dir.join(format!("{file}.part"));
        let mut out = std::fs::File::create(&part)?;
        std::io::copy(&mut entry.by_ref().take(500 * 1024 * 1024), &mut out)?;
        drop(out);
        std::fs::rename(&part, dir.join(file))?;
        found += 1;
    }
    if found == 2 { Ok(()) } else { Err(Error::Archive("ffmpeg.exe/ffprobe.exe not in the download".into())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn takes_only_the_two_programs() {
        let dir = std::env::temp_dir().join(format!("submagician-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("f.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            for (name, body) in [
                ("ffmpeg-9.0.2-essentials_build/bin/ffmpeg.exe", "MZ ffmpeg"),
                ("ffmpeg-9.0.2-essentials_build/bin/ffprobe.exe", "MZ ffprobe"),
                ("ffmpeg-9.0.2-essentials_build/bin/ffplay.exe", "MZ ffplay"),
                ("ffmpeg-9.0.2-essentials_build/doc/ffmpeg.html", "<html>"),
            ] {
                w.start_file(name, o).unwrap();
                w.write_all(body.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        extract_ffmpeg(&zip_path, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("ffmpeg.exe")).unwrap(), "MZ ffmpeg");
        assert_eq!(std::fs::read_to_string(dir.join("ffprobe.exe")).unwrap(), "MZ ffprobe");
        assert!(!dir.join("ffplay.exe").exists());

        let empty = dir.join("e.zip");
        zip::ZipWriter::new(std::fs::File::create(&empty).unwrap()).finish().unwrap();
        assert!(extract_ffmpeg(&empty, &dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hashes_as_hex() {
        assert_eq!(hex(&Sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    /// Live: downloads the real build (about 110 MB) and checks it; only with
    /// `SUBMAGICIAN_LIVE_FFMPEG=1`.
    #[tokio::test]
    async fn live_download() {
        if !std::env::var("SUBMAGICIAN_LIVE_FFMPEG").is_ok_and(|v| v == "1") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("submagician-ffmpeg-dl-{}", std::process::id()));
        let mut last = 0;
        download_ffmpeg(&dir, &AtomicBool::new(false), &mut |d, _| last = d).await.unwrap();
        assert!(last > 50_000_000);
        for f in ["ffmpeg.exe", "ffprobe.exe"] {
            let head = std::fs::read(dir.join(f)).unwrap();
            assert_eq!(&head[..2], b"MZ", "{f} is a Windows program");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
