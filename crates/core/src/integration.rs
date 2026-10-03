//! SubMagician in the file manager's right-click menu, with the entries the user picked:
//! open a folder or video in SubMagician, get subtitles for it, or sync its subtitle to the
//! audio. The last two open the window too and start the work there (`--get` / `--sync`).
//!
//! - Windows: entries under `HKCU\Software\Classes` for folders, folder backgrounds and video
//!   files (no admin rights needed).
//! - Linux: a launcher in `applications/` (so "Open with" lists SubMagician for folders and
//!   videos, and they can be dropped on its icon), a Nautilus/Nemo/Caja script and a Dolphin
//!   service menu. The Flatpak exports its own launcher, so it adds only the menus, which start
//!   it through `flatpak run`.
//!
//! The entries run `command` (`packaging::launch_command`) with the action's flag and the path.

use std::path::PathBuf;

use crate::Result;

/// Which entries the right-click menu has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MenuEntries {
    /// "Open in SubMagician" (folders and videos).
    pub open: bool,
    /// "Get subtitles" in the wanted languages (folders and videos).
    pub get: bool,
    /// "Sync subtitle to the audio" (videos).
    pub sync: bool,
}

impl Default for MenuEntries {
    fn default() -> Self {
        MenuEntries { open: true, get: true, sync: true }
    }
}

/// What a menu entry asks the app to do with the path it passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    Get,
    Sync,
}

impl Action {
    /// The command-line flag before the path (`None`: just the path).
    pub fn flag(self) -> Option<&'static str> {
        match self {
            Action::Open => None,
            Action::Get => Some("--get"),
            Action::Sync => Some("--sync"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Open => "Open in SubMagician",
            Action::Get => "Get subtitles with SubMagician",
            Action::Sync => "Sync subtitle to the audio (SubMagician)",
        }
    }

    /// Reads the app's own arguments: `[--get|--sync] <path>`.
    pub fn from_args(mut args: impl Iterator<Item = std::ffi::OsString>) -> Option<(Action, PathBuf)> {
        let first = args.next()?;
        let action = match first.to_str() {
            Some("--get") => Action::Get,
            Some("--sync") => Action::Sync,
            _ => return Some((Action::Open, PathBuf::from(first))),
        };
        args.next().map(|p| (action, PathBuf::from(p)))
    }
}

impl MenuEntries {
    /// The chosen entries, in menu order.
    pub fn chosen(self) -> Vec<Action> {
        [(self.open, Action::Open), (self.get, Action::Get), (self.sync, Action::Sync)]
            .into_iter()
            .filter_map(|(on, a)| on.then_some(a))
            .collect()
    }
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
    const SCRIPT_DIRS: [&str; 3] = ["nautilus/scripts", "nemo/scripts", "caja/scripts"];
    /// Script names of earlier versions, removed on every change.
    const OLD_SCRIPTS: [&str; 1] = ["Find subtitles with SubMagician"];
    const ALL: [Action; 3] = [Action::Open, Action::Get, Action::Sync];

    use std::path::Path;

    fn launcher(data: &Path) -> PathBuf {
        data.join("applications/submagician.desktop")
    }

    fn service_menu(data: &Path) -> PathBuf {
        data.join("kio/servicemenus/submagician.desktop")
    }

    fn data_dir() -> Option<PathBuf> {
        crate::packaging::host_dirs().map(|(_, data)| data)
    }

    /// `"text"` with the quoting rules of desktop entries (also fine for sh).
    fn quoted(text: &str) -> String {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$"))
    }

    /// The command line for `action` on `arg` (a field code like `%f`, or "" for scripts).
    fn exec(command: &[String], action: Action, arg: &str) -> String {
        let mut parts: Vec<String> = command.iter().map(|c| quoted(c)).collect();
        parts.extend(action.flag().map(String::from));
        if !arg.is_empty() {
            parts.push(arg.into());
        }
        parts.join(" ")
    }

    pub fn install(command: &[String], entries: MenuEntries) -> Result<()> {
        // The Flatpak's own launcher already offers it for folders and videos.
        let launcher = crate::packaging::flatpak_id().is_none();
        install_into(&data_dir().ok_or_else(no_home)?, command, entries, launcher)
    }

    pub fn uninstall() -> Result<()> {
        uninstall_from(&data_dir().ok_or_else(no_home)?)
    }

    pub fn is_installed() -> bool {
        data_dir().is_some_and(|d| {
            launcher(&d).is_file()
                || service_menu(&d).is_file()
                || SCRIPT_DIRS.iter().any(|dir| ALL.iter().any(|a| d.join(dir).join(a.label()).is_file()))
        })
    }

    fn no_home() -> crate::Error {
        crate::Error::Io(std::io::Error::other("no home folder"))
    }

    pub(super) fn install_into(
        data: &Path,
        command: &[String],
        entries: MenuEntries,
        with_launcher: bool,
    ) -> Result<()> {
        // Start clean, so entries switched off disappear.
        uninstall_from(data)?;
        if with_launcher {
            write(
                &launcher(data),
                &format!(
                    "[Desktop Entry]\nType=Application\nName=SubMagician\nComment=Find, pick and sync subtitles\nExec={}\nIcon=video-x-generic\nTerminal=false\nCategories=AudioVideo;Video;\nMimeType={MIME}\nNoDisplay=false\n",
                    exec(command, Action::Open, "%f")
                ),
                false,
            )?;
        }
        let chosen = entries.chosen();
        for dir in SCRIPT_DIRS {
            // Only for file managers that are there: their scripts folder's parent exists.
            if !data.join(dir).parent().is_some_and(Path::is_dir) {
                continue;
            }
            for action in &chosen {
                let script = format!(
                    "#!/bin/sh\n# Added by SubMagician (Settings → Right-click menu).\nexec {} \"$1\"\n",
                    exec(command, *action, "")
                );
                write(&data.join(dir).join(action.label()), &script, true)?;
            }
        }
        if !chosen.is_empty() {
            let ids: Vec<String> = chosen.iter().map(|a| format!("{a:?}").to_lowercase()).collect();
            let mut menu = format!(
                "[Desktop Entry]\nType=Service\nX-KDE-ServiceTypes=KonqPopupMenu/Plugin\nMimeType={MIME}\nActions={};\n",
                ids.join(";")
            );
            for (action, id) in chosen.iter().zip(&ids) {
                menu += &format!(
                    "\n[Desktop Action {id}]\nName={}\nIcon=video-x-generic\nExec={}\n",
                    action.label(),
                    exec(command, *action, "%f")
                );
            }
            write(&service_menu(data), &menu, true)?;
        }
        Ok(())
    }

    pub(super) fn uninstall_from(data: &Path) -> Result<()> {
        let mut paths = vec![launcher(data), service_menu(data)];
        for dir in SCRIPT_DIRS {
            paths.extend(ALL.iter().map(|a| data.join(dir).join(a.label())));
            paths.extend(OLD_SCRIPTS.iter().map(|n| data.join(dir).join(n)));
        }
        for path in paths {
            match fs::remove_file(&path) {
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
            let program = vec!["/opt/Sub Magician/submagician".to_owned()];
            install_into(&data, &program, MenuEntries::default(), true).unwrap();

            let launcher = fs::read_to_string(data.join("applications/submagician.desktop")).unwrap();
            assert!(launcher.contains("Exec=\"/opt/Sub Magician/submagician\" %f"), "{launcher}");
            assert!(launcher.contains("MimeType=inode/directory;"));
            let open = data.join("nautilus/scripts/Open in SubMagician");
            assert!(fs::read_to_string(&open).unwrap().contains("exec \"/opt/Sub Magician/submagician\" \"$1\""));
            assert_eq!(fs::metadata(&open).unwrap().permissions().mode() & 0o777, 0o755);
            let get = fs::read_to_string(data.join("nautilus/scripts/Get subtitles with SubMagician")).unwrap();
            assert!(get.contains("exec \"/opt/Sub Magician/submagician\" --get \"$1\""), "{get}");
            assert!(!data.join("nemo").exists(), "no scripts for file managers that are not there");
            let menu = fs::read_to_string(data.join("kio/servicemenus/submagician.desktop")).unwrap();
            assert!(menu.contains("Actions=open;get;sync;"), "{menu}");
            assert!(menu.contains("Exec=\"/opt/Sub Magician/submagician\" --sync %f"), "{menu}");

            // Switching an entry off removes it.
            install_into(&data, &program, MenuEntries { open: true, get: false, sync: false }, true).unwrap();
            assert!(open.exists());
            assert!(!data.join("nautilus/scripts/Get subtitles with SubMagician").exists());
            let menu = fs::read_to_string(data.join("kio/servicemenus/submagician.desktop")).unwrap();
            assert!(menu.contains("Actions=open;") && !menu.contains("--get"), "{menu}");

            uninstall_from(&data).unwrap();
            assert!(!data.join("applications/submagician.desktop").exists());
            assert!(!open.exists());
            uninstall_from(&data).unwrap(); // twice is fine

            // Flatpak: no launcher of its own, the menus run `flatpak run`.
            let flatpak: Vec<String> = ["flatpak", "run", "io.github.halilkhrmn.SubMagician"].map(String::from).into();
            install_into(&data, &flatpak, MenuEntries::default(), false).unwrap();
            assert!(!data.join("applications/submagician.desktop").exists());
            let get = fs::read_to_string(data.join("nautilus/scripts/Get subtitles with SubMagician")).unwrap();
            assert!(
                get.contains("exec \"flatpak\" \"run\" \"io.github.halilkhrmn.SubMagician\" --get \"$1\""),
                "{get}"
            );
            fs::remove_dir_all(&data).unwrap();
        }

        #[test]
        fn quotes_for_desktop_entries() {
            assert_eq!(quoted("/a b/$x\"y"), "\"/a b/\\$x\\\"y\"");
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    /// Where the entries go and the placeholder for the clicked item.
    const PLACES: [(&str, &str); 3] = [
        (r"HKCU\Software\Classes\Directory\shell", "%1"),
        (r"HKCU\Software\Classes\Directory\Background\shell", "%V"),
        (r"HKCU\Software\Classes\SystemFileAssociations\video\shell", "%1"),
    ];
    const ALL: [Action; 3] = [Action::Open, Action::Get, Action::Sync];

    fn key_name(action: Action) -> &'static str {
        match action {
            Action::Open => "SubMagician",
            Action::Get => "SubMagicianGet",
            Action::Sync => "SubMagicianSync",
        }
    }

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

    pub fn install(command: &[String], entries: MenuEntries) -> Result<()> {
        uninstall()?;
        let exe = command.first().cloned().unwrap_or_default();
        for action in entries.chosen() {
            for (place, arg) in PLACES {
                let key = format!(r"{place}\{}", key_name(action));
                let command = match action.flag() {
                    Some(flag) => format!("\"{exe}\" {flag} \"{arg}\""),
                    None => format!("\"{exe}\" \"{arg}\""),
                };
                let ok = reg(&["add", &key, "/ve", "/d", action.label(), "/f"])?
                    && reg(&["add", &key, "/v", "Icon", "/d", &exe, "/f"])?
                    && reg(&["add", &format!(r"{key}\command"), "/ve", "/d", &command, "/f"])?;
                if !ok {
                    return Err(crate::Error::Io(std::io::Error::other(format!("could not write {key}"))));
                }
            }
        }
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        for action in ALL {
            for (place, _) in PLACES {
                // Fails when the key is not there, which is fine.
                let _ = reg(&["delete", &format!(r"{place}\{}", key_name(action)), "/f"])?;
            }
        }
        Ok(())
    }

    pub fn is_installed() -> bool {
        ALL.iter().any(|a| reg(&["query", &format!(r"{}\{}", PLACES[0].0, key_name(*a))]).unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_app_arguments() {
        let args = |a: &[&str]| Action::from_args(a.iter().map(std::ffi::OsString::from));
        assert_eq!(args(&[]), None);
        assert_eq!(args(&["/v/a.mkv"]), Some((Action::Open, PathBuf::from("/v/a.mkv"))));
        assert_eq!(args(&["--get", "/v"]), Some((Action::Get, PathBuf::from("/v"))));
        assert_eq!(args(&["--sync", "/v/a.mkv"]), Some((Action::Sync, PathBuf::from("/v/a.mkv"))));
        assert_eq!(args(&["--sync"]), None);
        assert_eq!(MenuEntries { open: false, get: true, sync: true }.chosen(), vec![Action::Get, Action::Sync]);
    }
}
