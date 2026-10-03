//! Updating SubMagician from inside the app.
//!
//! The newest release comes from GitHub (`releases/latest`). The Windows installer and the
//! AppImage can update themselves: the file is downloaded into the data folder, checked against
//! the SHA-256 digest GitHub publishes for every release asset, then
//! - Windows: the installer runs silently (it closes the app, replaces the files and starts it
//!   again, see `installer/submagician.iss`);
//! - AppImage: the new file takes the place of the running one, which starts it once it exits.
//!
//! Copies from a package (deb) or a portable zip are left to the user: the app links to the
//! release instead.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{APP_REPO, Error, Result, VERSION};

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
    /// `sha256:<hex>`, published by GitHub for release assets.
    pub digest: Option<String>,
}

impl Release {
    /// "0.2.0" for the tag "v0.2.0".
    pub fn version(&self) -> &str {
        self.tag_name.trim_start_matches(['v', 'V'])
    }
}

/// How this copy can update itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    WindowsInstaller,
    AppImage,
}

impl Format {
    /// `None` for a package-manager copy, a portable zip or a build from source.
    pub fn current() -> Option<Format> {
        if cfg!(windows) {
            // The installer leaves its uninstaller next to the program; a portable copy has none.
            let exe = std::env::current_exe().ok()?;
            return exe.parent()?.join("unins000.exe").is_file().then_some(Format::WindowsInstaller);
        }
        if cfg!(target_os = "linux") && std::env::consts::ARCH == "x86_64" && std::env::var_os("APPIMAGE").is_some() {
            return Some(Format::AppImage);
        }
        None
    }

    /// The release asset for `version` (as named by `.github/workflows/release.yml`).
    pub fn asset_name(self, version: &str) -> String {
        let version = version.trim_start_matches(['v', 'V']);
        match self {
            Format::WindowsInstaller => format!("submagician-setup-{version}.exe"),
            Format::AppImage => format!("SubMagician-{version}-x86_64.AppImage"),
        }
    }
}

/// `a < b` for versions like "0.1.0" / "v0.2"; text after a number ends the comparison.
pub fn version_lt(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        let mut p: Vec<u64> =
            v.trim().trim_start_matches(['v', 'V']).split(['.', '-']).map_while(|s| s.parse().ok()).collect();
        while p.last() == Some(&0) {
            p.pop();
        }
        p
    }
    parts(a) < parts(b)
}

fn client() -> Result<reqwest::Client> {
    crate::net::client(Some(Duration::from_secs(30)))
}

/// The newest release when it is newer than this build. A repository without a public release
/// (404) means "no update".
pub async fn check() -> Result<Option<Release>> {
    let url = format!("https://api.github.com/repos/{APP_REPO}/releases/latest");
    let response = client()?.get(&url).header("Accept", "application/vnd.github+json").send().await?;
    match response.status().as_u16() {
        404 => return Ok(None),
        403 | 429 => {
            return Err(Error::Api {
                provider: "GitHub",
                status: response.status().as_u16(),
                message: "rate limit reached, try again later".into(),
            });
        }
        _ => {}
    }
    let release: Release = response.error_for_status()?.json().await?;
    Ok(version_lt(VERSION, &release.tag_name).then_some(release))
}

/// Where updates are downloaded.
pub fn updates_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "SubMagician").map(|d| d.data_local_dir().join("updates"))
}

/// Downloads `release` in `format` into `dir`, checked against GitHub's SHA-256 digest; a file
/// already there with the right digest is reused and older downloads are deleted. `progress`
/// gets (bytes, total).
pub async fn download(
    release: &Release,
    format: Format,
    dir: &Path,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> Result<PathBuf> {
    let name = format.asset_name(&release.tag_name);
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| Error::Parse(format!("release {} has no {name}", release.tag_name)))?;
    let expected = expected_digest(asset)?;
    std::fs::create_dir_all(dir)?;
    let file = dir.join(&name);
    if !(file.is_file() && sha256_file(&file)? == expected) {
        let part = dir.join(format!("{name}.part"));
        let mut response =
            client_without_timeout()?.get(&asset.browser_download_url).send().await?.error_for_status()?;
        let total = response.content_length().or(Some(asset.size).filter(|s| *s > 0));
        let mut out = std::fs::File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut done = 0u64;
        while let Some(chunk) = response.chunk().await? {
            if cancel.load(Ordering::Relaxed) {
                drop(out);
                let _ = std::fs::remove_file(&part);
                return Err(Error::Cancelled);
            }
            hasher.update(&chunk);
            std::io::Write::write_all(&mut out, &chunk)?;
            done += chunk.len() as u64;
            progress(done, total);
        }
        drop(out);
        let actual = hex(&hasher.finalize());
        if actual != expected {
            let _ = std::fs::remove_file(&part);
            return Err(Error::Parse(format!("the update is damaged (checksum {actual}, expected {expected})")));
        }
        std::fs::rename(&part, &file)?;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path() != file {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(file)
}

fn client_without_timeout() -> Result<reqwest::Client> {
    crate::net::client(None)
}

fn expected_digest(asset: &Asset) -> Result<String> {
    asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|d| d.len() == 64)
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| Error::Parse("the update has no SHA-256 digest; refusing to use it".into()))
}

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Starts the downloaded update. The caller quits right after: the Windows installer closes
/// and restarts the app itself; the AppImage is swapped and started once this process exits.
pub fn install(format: Format, file: &Path) -> Result<()> {
    match format {
        Format::WindowsInstaller => run_installer(file),
        Format::AppImage => {
            let target = PathBuf::from(
                std::env::var_os("APPIMAGE").ok_or_else(|| Error::Parse("this copy is not an AppImage".into()))?,
            );
            swap_appimage(file, &target)?;
            restart_when_gone(&target)
        }
    }
}

#[cfg(windows)]
fn run_installer(file: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let log = file.with_extension("log");
    std::process::Command::new(file)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-", "/CLOSEAPPLICATIONS", "/RELAUNCH=1"])
        .arg(format!("/LOG={}", log.display()))
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()?;
    Ok(())
}

#[cfg(not(windows))]
fn run_installer(_file: &Path) -> Result<()> {
    Err(Error::Parse("the installer runs only on Windows".into()))
}

/// Copies `new` over `target` through a temporary file in the same folder, so a failed copy
/// leaves the old AppImage working. The running copy keeps working: its mount still refers to
/// the old file.
pub fn swap_appimage(new: &Path, target: &Path) -> Result<()> {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = target.with_file_name(format!(".{name}.update"));
    let copy = || -> std::io::Result<()> {
        std::fs::copy(new, &tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&tmp, target)
    };
    copy().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::Io(e)
    })
}

/// Starts `program` once this process has exited (a second copy would otherwise find this
/// one still running).
fn restart_when_gone(program: &Path) -> Result<()> {
    std::process::Command::new("sh")
        .args(["-c", r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec "$0""#])
        .arg(program)
        .arg(std::process::id().to_string())
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(version_lt("0.1.0", "v0.2.0"));
        assert!(version_lt("0.1.9", "0.1.10"));
        assert!(!version_lt("0.2.0", "0.2"));
        assert!(!version_lt("1.0.0", "0.9.9"));
        assert!(version_lt("0.1", "0.1.1"));
        assert!(!version_lt("0.1.0", "garbage"));
    }

    #[test]
    fn names_assets() {
        assert_eq!(Format::WindowsInstaller.asset_name("v0.2.0"), "submagician-setup-0.2.0.exe");
        assert_eq!(Format::AppImage.asset_name("0.2.0"), "SubMagician-0.2.0-x86_64.AppImage");
    }

    #[test]
    fn reads_release_json() {
        let json = r#"{"tag_name":"v0.2.0","html_url":"https://github.com/x/y/releases/tag/v0.2.0",
            "assets":[{"name":"SubMagician-0.2.0-x86_64.AppImage","browser_download_url":"https://e/a",
            "size":10,"digest":"sha256:ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789"},
            {"name":"old","browser_download_url":"https://e/b","digest":null}]}"#;
        let r: Release = serde_json::from_str(json).unwrap();
        assert_eq!(r.version(), "0.2.0");
        assert_eq!(
            expected_digest(&r.assets[0]).unwrap(),
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
        assert!(expected_digest(&r.assets[1]).is_err(), "no digest, no update");
    }

    #[test]
    fn reuses_a_verified_download_and_removes_old_ones() {
        let dir = std::env::temp_dir().join(format!("submagician-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let name = Format::AppImage.asset_name("9.9.9");
        std::fs::write(dir.join(&name), b"new build").unwrap();
        std::fs::write(dir.join("SubMagician-0.0.1-x86_64.AppImage"), b"old").unwrap();
        let digest = sha256_file(&dir.join(&name)).unwrap();
        let release = Release {
            tag_name: "v9.9.9".into(),
            html_url: String::new(),
            assets: vec![Asset {
                name: name.clone(),
                // Never fetched: the file on disk already has this digest.
                browser_download_url: "https://invalid.invalid/x".into(),
                size: 9,
                digest: Some(format!("sha256:{digest}")),
            }],
        };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let file =
            rt.block_on(download(&release, Format::AppImage, &dir, &AtomicBool::new(false), &mut |_, _| {})).unwrap();
        assert_eq!(file, dir.join(&name));
        assert!(!dir.join("SubMagician-0.0.1-x86_64.AppImage").exists());

        let target = dir.join("SubMagician.AppImage");
        std::fs::write(&target, b"running build").unwrap();
        swap_appimage(&file, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new build");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o755);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
