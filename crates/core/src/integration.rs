//! "Find subtitles" in the file manager: right-click a folder or a video to open it in
//! SubMagician.
//!
//! - Windows: entries under `HKCU\Software\Classes` for folders, folder backgrounds and video
//!   files (no admin rights needed).
//! - Linux: a launcher in `applications/` (so "Open with" lists SubMagician for folders and
//!   videos, and they can be dropped on its icon), a Nautilus/Nemo/Caja script and a Dolphin
//!   service menu.

use std::path::{Path, PathBuf};

use crate::Result;

/// The program the menu entries start: the AppImage when running from one, else this executable.
pub fn current_program() -> Option<PathBuf> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Some(PathBuf::from(appimage));
    }
    std::env::current_exe().ok()
}

#[cfg(not(windows))]
pub use unix::{install, is_installed, uninstall};
#[cfg(windows)]
pub use windows::{install, is_installed, uninstall};

#[cfg(not(windows))]
mod unix {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const MIME: &str = "inode/directory;video/x-matroska;video/mp4;video/x-msvideo;video/quicktime;video/webm;video/mpeg;video/x-ms-wmv;video/mp2t;video/x-flv;video/3gpp;";

    struct Files {
        launcher: PathBuf,
        scripts: Vec<PathBuf>,
        service_menu: PathBuf,
    }

    fn files(data: &Path) -> Files {
        let script = "Find subtitles with SubMagician";
        Files {
            launcher: data.join("applications/submagician.desktop"),
            scripts: ["nautilus/scripts", "nemo/scripts", "caja/scripts"]
                .iter()
                .map(|d| data.join(d).join(script))
                .collect(),
            service_menu: data.join("kio/servicemenus/submagician.desktop"),
        }
    }

    fn data_dir() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| d.data_dir().to_path_buf())
    }

    /// `"path"` with the quoting rules of desktop entries.
    fn quoted(program: &Path) -> String {
        let s = program.display().to_string();
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$"))
    }

    pub fn install(program: &Path) -> Result<()> {
        install_into(&data_dir().ok_or_else(no_home)?, program)
    }

    pub fn uninstall() -> Result<()> {
        uninstall_from(&data_dir().ok_or_else(no_home)?)
    }

    pub fn is_installed() -> bool {
        data_dir().is_some_and(|d| files(&d).launcher.is_file())
    }

    fn no_home() -> crate::Error {
        crate::Error::Io(std::io::Error::other("no home folder"))
    }

    pub(super) fn install_into(data: &Path, program: &Path) -> Result<()> {
        let f = files(data);
        let exec = quoted(program);
        write(
            &f.launcher,
            &format!(
                "[Desktop Entry]\nType=Application\nName=SubMagician\nComment=Find, pick and sync subtitles\nExec={exec} %f\nIcon=video-x-generic\nTerminal=false\nCategories=AudioVideo;Video;\nMimeType={MIME}\nNoDisplay=false\n"
            ),
            false,
        )?;
        let script = format!("#!/bin/sh\n# Added by SubMagician (Settings → File manager).\nexec {exec} \"$1\"\n");
        for path in &f.scripts {
            // Only for file managers that are there: their scripts folder's parent exists.
            if path.parent().and_then(Path::parent).is_some_and(Path::is_dir) {
                write(path, &script, true)?;
            }
        }
        write(
            &f.service_menu,
            &format!(
                "[Desktop Entry]\nType=Service\nX-KDE-ServiceTypes=KonqPopupMenu/Plugin\nMimeType={MIME}\nActions=findSubtitles;\n\n[Desktop Action findSubtitles]\nName=Find subtitles with SubMagician\nIcon=video-x-generic\nExec={exec} %f\n"
            ),
            true,
        )?;
        Ok(())
    }

    pub(super) fn uninstall_from(data: &Path) -> Result<()> {
        let f = files(data);
        for path in std::iter::once(&f.launcher).chain(&f.scripts).chain(std::iter::once(&f.service_menu)) {
            match fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    }

    fn write(path: &Path, content: &str, executable: bool) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, content)?;
        if executable {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn installs_and_removes() {
            let data = std::env::temp_dir().join(format!("submagician-integration-{}", std::process::id()));
            let _ = fs::remove_dir_all(&data);
            fs::create_dir_all(data.join("nautilus")).unwrap(); // GNOME Files is "installed", Nemo is not
            let program = Path::new("/opt/Sub Magician/submagician");
            install_into(&data, program).unwrap();

            let launcher = fs::read_to_string(data.join("applications/submagician.desktop")).unwrap();
            assert!(launcher.contains("Exec=\"/opt/Sub Magician/submagician\" %f"), "{launcher}");
            assert!(launcher.contains("MimeType=inode/directory;"));
            let script = data.join("nautilus/scripts/Find subtitles with SubMagician");
            assert!(fs::read_to_string(&script).unwrap().contains("exec \"/opt/Sub Magician/submagician\" \"$1\""));
            assert_eq!(fs::metadata(&script).unwrap().permissions().mode() & 0o777, 0o755);
            assert!(!data.join("nemo").exists(), "no scripts for file managers that are not there");
            let menu = fs::read_to_string(data.join("kio/servicemenus/submagician.desktop")).unwrap();
            assert!(
                menu.contains("Actions=findSubtitles;") && menu.contains("Exec=\"/opt/Sub Magician/submagician\" %f")
            );

            uninstall_from(&data).unwrap();
            assert!(!data.join("applications/submagician.desktop").exists());
            assert!(!script.exists());
            uninstall_from(&data).unwrap(); // twice is fine
            fs::remove_dir_all(&data).unwrap();
        }

        #[test]
        fn quotes_for_desktop_entries() {
            assert_eq!(quoted(Path::new("/a b/$x\"y")), "\"/a b/\\$x\\\"y\"");
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const KEYS: [(&str, &str); 3] = [
        (r"HKCU\Software\Classes\Directory\shell\SubMagician", "%1"),
        (r"HKCU\Software\Classes\Directory\Background\shell\SubMagician", "%V"),
        (r"HKCU\Software\Classes\SystemFileAssociations\video\shell\SubMagician", "%1"),
    ];

    fn reg(args: &[&str]) -> Result<bool> {
        let status = Command::new("reg")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()?;
        Ok(status.success())
    }

    pub fn install(program: &Path) -> Result<()> {
        let exe = program.display().to_string();
        for (key, arg) in KEYS {
            let command = format!("\"{exe}\" \"{arg}\"");
            let ok = reg(&["add", key, "/ve", "/d", "Find subtitles with SubMagician", "/f"])?
                && reg(&["add", key, "/v", "Icon", "/d", &exe, "/f"])?
                && reg(&["add", &format!(r"{key}\command"), "/ve", "/d", &command, "/f"])?;
            if !ok {
                return Err(crate::Error::Io(std::io::Error::other(format!("could not write {key}"))));
            }
        }
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        for (key, _) in KEYS {
            // Fails when the key is not there, which is fine.
            let _ = reg(&["delete", key, "/f"])?;
        }
        Ok(())
    }

    pub fn is_installed() -> bool {
        reg(&["query", KEYS[0].0]).unwrap_or(false)
    }
}
