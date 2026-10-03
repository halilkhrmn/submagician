//! SubMagician in the file manager's right-click menu, with the entries and places the user
//! picked: open a folder or video in SubMagician, get subtitles for it, or sync its subtitle to
//! the audio, on folders, on the empty area of a folder and on video files (all, or only some
//! extensions), as a "SubMagician ▸" submenu or as separate entries. "Get" and "sync" open the
//! window too and start the work there (`--get` / `--sync`).
//!
//! - Windows: keys under `HKCU\Software\Classes` (no admin rights), written with one
//!   `reg import`. Videos get the entries per extension (`SystemFileAssociations\.mkv`): the
//!   "video" perceived type is missing for extensions no installed app has claimed.
//! - Linux: a launcher in `applications/` (so "Open with" lists SubMagician for folders and
//!   videos, and they can be dropped on its icon), Nautilus/Nemo/Caja scripts (in a
//!   `SubMagician` folder for the submenu) and a Dolphin service menu. The Flatpak exports its
//!   own launcher, so it adds only the menus, which start it through `flatpak run`.
//!
//! The entries run `command` (`packaging::launch_command`) with the action's flag and the path.

use std::path::PathBuf;

use crate::Result;

/// Which entries the right-click menu has, and where.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MenuEntries {
    /// "Open in SubMagician".
    pub open: bool,
    /// "Get subtitles" in the wanted languages.
    pub get: bool,
    /// "Sync the subtitle to the audio".
    pub sync: bool,
    /// Under one "SubMagician" submenu instead of separate entries.
    pub submenu: bool,
    /// On folders.
    pub folders: bool,
    /// On the empty area of an open folder.
    pub background: bool,
    /// On video files.
    pub videos: bool,
    /// Only these video extensions ("mkv, mp4"); empty: every video extension.
    pub extensions: String,
}

impl Default for MenuEntries {
    fn default() -> Self {
        MenuEntries {
            open: true,
            get: true,
            sync: true,
            submenu: true,
            folders: true,
            background: true,
            videos: true,
            extensions: String::new(),
        }
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

    /// The entry's text on its own in the menu.
    pub fn label(self) -> &'static str {
        match self {
            Action::Open => "Open in SubMagician",
            Action::Get => "Get subtitles with SubMagician",
            Action::Sync => "Sync subtitle to the audio (SubMagician)",
        }
    }

    /// The entry's text inside the "SubMagician" submenu.
    pub fn short_label(self) -> &'static str {
        match self {
            Action::Open => "Open",
            Action::Get => "Get subtitles",
            Action::Sync => "Sync subtitle to the audio",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Action::Open => "open",
            Action::Get => "get",
            Action::Sync => "sync",
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
    pub fn chosen(&self) -> Vec<Action> {
        [(self.open, Action::Open), (self.get, Action::Get), (self.sync, Action::Sync)]
            .into_iter()
            .filter_map(|(on, a)| on.then_some(a))
            .collect()
    }

    /// The video extensions the entries are offered for (none when videos are off). Unknown
    /// extensions in the setting are left out.
    pub fn video_extensions(&self) -> Vec<&'static str> {
        if !self.videos {
            return Vec::new();
        }
        let wanted: Vec<String> = self
            .extensions
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .map(|e| e.trim().trim_start_matches("*.").trim_start_matches('.').to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        crate::media::VIDEO_EXTS
            .iter()
            .copied()
            .filter(|e| wanted.is_empty() || wanted.iter().any(|w| w == e))
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
    use std::path::Path;

    const ALL: [Action; 3] = [Action::Open, Action::Get, Action::Sync];
    const SCRIPT_DIRS: [&str; 3] = ["nautilus/scripts", "nemo/scripts", "caja/scripts"];
    /// Script names of earlier versions, removed on every change.
    const OLD_SCRIPTS: [&str; 1] = ["Find subtitles with SubMagician"];
    /// The submenu: a folder of scripts.
    const SUBMENU: &str = "SubMagician";

    fn mime(ext: &str) -> &'static str {
        match ext {
            "mkv" => "video/x-matroska",
            "mp4" | "m4v" => "video/mp4",
            "avi" | "divx" => "video/x-msvideo",
            "mov" => "video/quicktime",
            "wmv" => "video/x-ms-wmv",
            "mpg" | "mpeg" | "vob" => "video/mpeg",
            "ts" | "m2ts" => "video/mp2t",
            "webm" => "video/webm",
            "flv" => "video/x-flv",
            "ogm" => "video/x-ogm+ogg",
            "3gp" => "video/3gpp",
            _ => "",
        }
    }

    /// The MIME types for `entries`: folders and the chosen video kinds.
    fn mime_types(entries: &MenuEntries) -> String {
        let mut types: Vec<&str> = Vec::new();
        if entries.folders || entries.background {
            types.push("inode/directory");
        }
        for ext in entries.video_extensions() {
            let m = mime(ext);
            if !m.is_empty() && !types.contains(&m) {
                types.push(m);
            }
        }
        types.iter().map(|t| format!("{t};")).collect()
    }

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

    pub fn install(command: &[String], entries: &MenuEntries) -> Result<()> {
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
                || SCRIPT_DIRS.iter().any(|dir| {
                    d.join(dir).join(SUBMENU).is_dir() || ALL.iter().any(|a| d.join(dir).join(a.label()).is_file())
                })
        })
    }

    fn no_home() -> crate::Error {
        crate::Error::Io(std::io::Error::other("no home folder"))
    }

    pub(super) fn install_into(
        data: &Path,
        command: &[String],
        entries: &MenuEntries,
        with_launcher: bool,
    ) -> Result<()> {
        // Start clean, so entries switched off disappear.
        uninstall_from(data)?;
        if with_launcher {
            write(
                &launcher(data),
                &format!(
                    "[Desktop Entry]\nType=Application\nName=SubMagician\nComment=Find, pick and sync subtitles\nExec={}\nIcon=video-x-generic\nTerminal=false\nCategories=AudioVideo;Video;\nMimeType={}\nNoDisplay=false\n",
                    exec(command, Action::Open, "%f"),
                    mime_types(&MenuEntries::default())
                ),
                false,
            )?;
        }
        let chosen = entries.chosen();
        let mimes = mime_types(entries);
        if chosen.is_empty() || mimes.is_empty() {
            return Ok(());
        }
        for dir in SCRIPT_DIRS {
            // Only for file managers that are there: their scripts folder's parent exists.
            if !data.join(dir).parent().is_some_and(Path::is_dir) {
                continue;
            }
            let folder = if entries.submenu { data.join(dir).join(SUBMENU) } else { data.join(dir) };
            for action in &chosen {
                let name = if entries.submenu { action.short_label() } else { action.label() };
                // Nothing selected (the folder's empty area): the open folder, the script's
                // working directory.
                let script = format!(
                    "#!/bin/sh\n# Added by SubMagician (Settings → Right-click menu).\nexec {} \"${{1:-$PWD}}\"\n",
                    exec(command, *action, "")
                );
                write(&folder.join(name), &script, true)?;
            }
        }
        let ids: Vec<&str> = chosen.iter().map(|a| a.id()).collect();
        let mut menu = format!(
            "[Desktop Entry]\nType=Service\nX-KDE-ServiceTypes=KonqPopupMenu/Plugin\nMimeType={mimes}\nActions={};\n",
            ids.join(";")
        );
        if entries.submenu {
            menu += "X-KDE-Submenu=SubMagician\n";
        }
        for action in &chosen {
            let name = if entries.submenu { action.short_label() } else { action.label() };
            menu += &format!(
                "\n[Desktop Action {}]\nName={name}\nIcon=video-x-generic\nExec={}\n",
                action.id(),
                exec(command, *action, "%f")
            );
        }
        write(&service_menu(data), &menu, true)?;
        Ok(())
    }

    pub(super) fn uninstall_from(data: &Path) -> Result<()> {
        let mut paths = vec![launcher(data), service_menu(data)];
        for dir in SCRIPT_DIRS {
            paths.extend(ALL.iter().map(|a| data.join(dir).join(a.label())));
            paths.extend(ALL.iter().map(|a| data.join(dir).join(SUBMENU).join(a.short_label())));
            paths.extend(OLD_SCRIPTS.iter().map(|n| data.join(dir).join(n)));
        }
        for path in paths {
            match fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        for dir in SCRIPT_DIRS {
            // Only when empty: someone may keep their own scripts there.
            let _ = fs::remove_dir(data.join(dir).join(SUBMENU));
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
            let flat = MenuEntries { submenu: false, ..MenuEntries::default() };
            install_into(&data, &program, &flat, true).unwrap();

            let launcher = fs::read_to_string(data.join("applications/submagician.desktop")).unwrap();
            assert!(launcher.contains("Exec=\"/opt/Sub Magician/submagician\" %f"), "{launcher}");
            assert!(launcher.contains("MimeType=inode/directory;video/x-matroska;"), "{launcher}");
            let open = data.join("nautilus/scripts/Open in SubMagician");
            assert!(
                fs::read_to_string(&open).unwrap().contains("exec \"/opt/Sub Magician/submagician\" \"${1:-$PWD}\"")
            );
            assert_eq!(fs::metadata(&open).unwrap().permissions().mode() & 0o777, 0o755);
            let get = fs::read_to_string(data.join("nautilus/scripts/Get subtitles with SubMagician")).unwrap();
            assert!(get.contains("exec \"/opt/Sub Magician/submagician\" --get \"${1:-$PWD}\""), "{get}");
            assert!(!data.join("nemo").exists(), "no scripts for file managers that are not there");
            let menu = fs::read_to_string(data.join("kio/servicemenus/submagician.desktop")).unwrap();
            assert!(menu.contains("Actions=open;get;sync;") && !menu.contains("X-KDE-Submenu"), "{menu}");
            assert!(menu.contains("Exec=\"/opt/Sub Magician/submagician\" --sync %f"), "{menu}");

            // The submenu, only Open, only on .mkv files.
            let some = MenuEntries {
                get: false,
                sync: false,
                folders: false,
                background: false,
                extensions: ".MKV, foo".into(),
                ..MenuEntries::default()
            };
            install_into(&data, &program, &some, true).unwrap();
            assert!(!open.exists());
            assert!(data.join("nautilus/scripts/SubMagician/Open").exists());
            assert!(!data.join("nautilus/scripts/SubMagician/Get subtitles").exists());
            let menu = fs::read_to_string(data.join("kio/servicemenus/submagician.desktop")).unwrap();
            assert!(
                menu.contains("MimeType=video/x-matroska;\n") && menu.contains("X-KDE-Submenu=SubMagician"),
                "{menu}"
            );
            assert!(menu.contains("Name=Open\n"), "{menu}");

            uninstall_from(&data).unwrap();
            assert!(!data.join("applications/submagician.desktop").exists());
            assert!(!data.join("nautilus/scripts/SubMagician").exists());
            uninstall_from(&data).unwrap(); // twice is fine

            // Flatpak: no launcher of its own, the menus run `flatpak run`.
            let flatpak: Vec<String> = ["flatpak", "run", "io.github.halilkhrmn.SubMagician"].map(String::from).into();
            install_into(&data, &flatpak, &flat, false).unwrap();
            assert!(!data.join("applications/submagician.desktop").exists());
            let get = fs::read_to_string(data.join("nautilus/scripts/Get subtitles with SubMagician")).unwrap();
            assert!(get.contains("exec \"flatpak\" \"run\" \"io.github.halilkhrmn.SubMagician\" --get"), "{get}");
            fs::remove_dir_all(&data).unwrap();
        }

        #[test]
        fn quotes_for_desktop_entries() {
            assert_eq!(quoted("/a b/$x\"y"), "\"/a b/\\$x\\\"y\"");
        }
    }
}

#[cfg(any(windows, test))]
mod registry {
    use super::*;

    /// Where entries can go: the key under `HKCU\Software\Classes` and the placeholder for the
    /// clicked item.
    pub(super) fn places(entries: &MenuEntries) -> Vec<(String, &'static str)> {
        let mut out = Vec::new();
        if entries.folders {
            out.push((r"Directory\shell".to_owned(), "%1"));
        }
        if entries.background {
            out.push((r"Directory\Background\shell".to_owned(), "%V"));
        }
        for ext in entries.video_extensions() {
            out.push((format!(r"SystemFileAssociations\.{ext}\shell"), "%1"));
        }
        out
    }

    /// Every key any version may have written, for removal.
    pub(super) fn all_keys() -> Vec<String> {
        let mut parents = vec![
            r"Directory\shell".to_owned(),
            r"Directory\Background\shell".to_owned(),
            r"SystemFileAssociations\video\shell".to_owned(),
        ];
        parents.extend(crate::media::VIDEO_EXTS.iter().map(|e| format!(r"SystemFileAssociations\.{e}\shell")));
        let mut out = Vec::new();
        for parent in parents {
            for name in ["SubMagician", "SubMagicianGet", "SubMagicianSync"] {
                out.push(format!(r"HKEY_CURRENT_USER\Software\Classes\{parent}\{name}"));
            }
        }
        out
    }

    fn reg_string(text: &str) -> String {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }

    /// A .reg file that removes every old entry and writes the chosen ones.
    pub(super) fn reg_file(exe: &str, entries: &MenuEntries) -> String {
        let mut out = String::from("Windows Registry Editor Version 5.00\r\n\r\n");
        for key in all_keys() {
            out += &format!("[-{key}]\r\n\r\n");
        }
        let chosen = entries.chosen();
        if chosen.is_empty() {
            return out;
        }
        let command = |action: Action, arg: &str| match action.flag() {
            Some(flag) => format!("\"{exe}\" {flag} \"{arg}\""),
            None => format!("\"{exe}\" \"{arg}\""),
        };
        for (place, arg) in places(entries) {
            let root = format!(r"HKEY_CURRENT_USER\Software\Classes\{place}");
            if entries.submenu {
                // A cascading menu: MUIVerb + an empty SubCommands, the items under "shell".
                let menu = format!(r"{root}\SubMagician");
                out += &format!(
                    "[{menu}]\r\n\"MUIVerb\"=\"SubMagician\"\r\n\"Icon\"={}\r\n\"SubCommands\"=\"\"\r\n\r\n",
                    reg_string(exe)
                );
                for (i, action) in chosen.iter().enumerate() {
                    let item = format!(r"{menu}\shell\{}{}", i + 1, action.id());
                    out += &format!("[{item}]\r\n\"MUIVerb\"={}\r\n\r\n", reg_string(action.short_label()));
                    out += &format!("[{item}\\command]\r\n@={}\r\n\r\n", reg_string(&command(*action, arg)));
                }
            } else {
                for action in &chosen {
                    let name = match action {
                        Action::Open => "SubMagician",
                        Action::Get => "SubMagicianGet",
                        Action::Sync => "SubMagicianSync",
                    };
                    let key = format!(r"{root}\{name}");
                    out +=
                        &format!("[{key}]\r\n@={}\r\n\"Icon\"={}\r\n\r\n", reg_string(action.label()), reg_string(exe));
                    out += &format!("[{key}\\command]\r\n@={}\r\n\r\n", reg_string(&command(*action, arg)));
                }
            }
        }
        out
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

    /// Imports `content` as a .reg file (UTF-16 with a byte order mark, as regedit writes them).
    fn import(content: &str) -> Result<()> {
        let path = std::env::temp_dir().join(format!("submagician-menu-{}.reg", std::process::id()));
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(content.encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(&path, bytes)?;
        let ok = reg(&["import", &path.display().to_string()]);
        let _ = std::fs::remove_file(&path);
        if ok? {
            Ok(())
        } else {
            Err(crate::Error::Io(std::io::Error::other("could not change the right-click menu (reg import failed)")))
        }
    }

    pub fn install(command: &[String], entries: &MenuEntries) -> Result<()> {
        let exe = command.first().cloned().unwrap_or_default();
        import(&registry::reg_file(&exe, entries))
    }

    pub fn uninstall() -> Result<()> {
        import(&registry::reg_file("", &MenuEntries { open: false, get: false, sync: false, ..MenuEntries::default() }))
    }

    pub fn is_installed() -> bool {
        [
            r"Directory\shell",
            r"Directory\Background\shell",
            r"SystemFileAssociations\.mkv\shell",
            r"SystemFileAssociations\.mp4\shell",
        ]
        .iter()
        .any(|parent| {
            ["SubMagician", "SubMagicianGet", "SubMagicianSync"]
                .iter()
                .any(|name| reg(&["query", &format!(r"HKCU\Software\Classes\{parent}\{name}")]).unwrap_or(false))
        })
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
        let some = MenuEntries { open: false, ..MenuEntries::default() };
        assert_eq!(some.chosen(), vec![Action::Get, Action::Sync]);
    }

    #[test]
    fn picks_the_video_extensions() {
        let all = MenuEntries::default();
        assert_eq!(all.video_extensions().len(), crate::media::VIDEO_EXTS.len());
        let some = MenuEntries { extensions: "*.MKV; .mp4 avi, nope".into(), ..MenuEntries::default() };
        assert_eq!(some.video_extensions(), vec!["mkv", "mp4", "avi"]);
        let none = MenuEntries { videos: false, ..MenuEntries::default() };
        assert!(none.video_extensions().is_empty());
    }

    #[test]
    fn writes_the_windows_registry_file() {
        let exe = r"C:\Users\Ali\AppData\Local\Programs\SubMagician\submagician.exe";
        let reg = registry::reg_file(exe, &MenuEntries { extensions: "mkv".into(), ..MenuEntries::default() });
        // Everything old goes first, the per-extension video key included.
        assert!(reg.contains(r"[-HKEY_CURRENT_USER\Software\Classes\SystemFileAssociations\video\shell\SubMagician]"));
        assert!(
            reg.contains(r"[-HKEY_CURRENT_USER\Software\Classes\SystemFileAssociations\.avi\shell\SubMagicianGet]")
        );
        // The submenu on .mkv files, folders and the folder background.
        let mkv = r"HKEY_CURRENT_USER\Software\Classes\SystemFileAssociations\.mkv\shell\SubMagician";
        assert!(reg.contains(&format!("[{mkv}]\r\n\"MUIVerb\"=\"SubMagician\"")), "{reg}");
        assert!(reg.contains(&format!("[{mkv}\\shell\\2get\\command]\r\n@=\"\\\"C:\\\\Users\\\\Ali")), "{reg}");
        assert!(reg.contains("--get \\\"%1\\\"\""), "{reg}");
        assert!(reg.contains(r"Directory\Background\shell\SubMagician\shell\1open\command]"));
        assert!(reg.contains("\\\"%V\\\"\""), "{reg}");
        assert!(!reg.contains(r"[HKEY_CURRENT_USER\Software\Classes\SystemFileAssociations\.mp4"));
        // Separate entries without the submenu.
        let flat = registry::reg_file(exe, &MenuEntries { submenu: false, ..MenuEntries::default() });
        assert!(flat.contains(r"[HKEY_CURRENT_USER\Software\Classes\Directory\shell\SubMagicianSync]"), "{flat}");
        assert!(!flat.contains("SubCommands"));
        // Uninstall: only removals.
        let off =
            registry::reg_file(exe, &MenuEntries { open: false, get: false, sync: false, ..MenuEntries::default() });
        assert!(off.lines().filter(|l| l.starts_with('[')).all(|l| l.starts_with("[-")));
    }
}
