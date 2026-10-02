//! Player plugins: a script for mpv (and mpv.net) and an extension for VLC that run
//! `submagician-cli --player` for the playing video and load the subtitle it saves. The scripts
//! live in `plugins/`; installing fills in the command and the auto setting and writes them into
//! the player's user folder (no admin rights needed).
//!
//! Players in a sandbox (Flatpak, Snap) cannot start programs outside it, so they are shown but
//! cannot get the plugin.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{Result, VERSION};

const MPV_SCRIPT: &str = include_str!("../../../plugins/submagician.lua");
const VLC_SCRIPT: &str = include_str!("../../../plugins/submagician_vlc.lua");
const FILE_NAME: &str = "submagician.lua";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mpv,
    MpvNet,
    Vlc,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Mpv => "mpv",
            Kind::MpvNet => "mpv.net",
            Kind::Vlc => "VLC",
        }
    }

    fn template(self) -> &'static str {
        match self {
            Kind::Mpv | Kind::MpvNet => MPV_SCRIPT,
            Kind::Vlc => VLC_SCRIPT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The player is not on this computer.
    NotFound,
    /// Installed from Flatpak or Snap: it cannot start SubMagician.
    Sandboxed(&'static str),
    Available,
    Installed,
}

#[derive(Debug, Clone)]
pub struct Player {
    pub kind: Kind,
    pub status: Status,
    /// Where the plugin goes (or is).
    pub script: PathBuf,
}

impl Player {
    fn new(kind: Kind, found: bool, sandbox: Option<&'static str>, script: PathBuf) -> Player {
        let status = if script.is_file() {
            Status::Installed
        } else if found {
            Status::Available
        } else if let Some(sandbox) = sandbox {
            Status::Sandboxed(sandbox)
        } else {
            Status::NotFound
        };
        Player { kind, status, script }
    }

    pub fn install(&self, cli: &[String], auto: bool) -> Result<()> {
        if let Some(dir) = self.script.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&self.script, render(self.kind, cli, auto))?;
        Ok(())
    }

    pub fn uninstall(&self) -> Result<()> {
        match fs::remove_file(&self.script) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

/// The script for `kind` with the command and the auto setting filled in.
pub fn render(kind: Kind, cli: &[String], auto: bool) -> String {
    let cli = cli.iter().map(|a| lua_string(a)).collect::<Vec<_>>().join(", ");
    kind.template()
        .replace("@@CLI@@", &cli)
        .replace("@@AUTO@@", if auto { "true" } else { "false" })
        .replace("@@VERSION@@", VERSION)
}

/// A Lua long string that cannot be closed by the text inside it.
fn lua_string(text: &str) -> String {
    let mut level = 0;
    while text.contains(&format!("]{}]", "=".repeat(level))) {
        level += 1;
    }
    let eq = "=".repeat(level);
    format!("[{eq}[{text}]{eq}]")
}

/// The command the plugins run: `submagician-cli` next to the app, or the AppImage with
/// `--cli` (its AppRun starts the tool inside). `None` when the tool is not there.
pub fn cli_command() -> Option<Vec<String>> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Some(vec![appimage.to_string_lossy().into_owned(), "--cli".into()]);
    }
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) { "submagician-cli.exe" } else { "submagician-cli" };
    let cli = exe.parent()?.join(name);
    cli.is_file().then(|| vec![plain_path(&cli)])
}

/// The path as text players can pass to a shell. On Windows VLC starts the tool through
/// `cmd.exe` in the ANSI code page, so a path with other letters gets its 8.3 short form.
fn plain_path(path: &Path) -> String {
    let text = path.display().to_string();
    #[cfg(windows)]
    if !text.is_ascii()
        && let Some(short) = short_path(path)
    {
        return short;
    }
    text
}

#[cfg(windows)]
fn short_path(path: &Path) -> Option<String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buf = vec![0u16; 1024];
    // SAFETY: `wide` is NUL-terminated and `buf` has the length passed.
    let n = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    let short = std::ffi::OsString::from_wide(&buf[..n]).to_string_lossy().into_owned();
    short.is_ascii().then_some(short)
}

/// Every supported player for this system, found or not.
pub fn detect() -> Vec<Player> {
    let home = directories::BaseDirs::new();
    let Some(home) = home.as_ref() else { return Vec::new() };
    detect_in(&Env {
        home: home.home_dir().to_path_buf(),
        config: home.config_dir().to_path_buf(),
        data: home.data_dir().to_path_buf(),
        path: std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default(),
        program_files: ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
            .collect(),
    })
}

/// Where to look; separate so tests can use a made-up home.
struct Env {
    home: PathBuf,
    /// `~/.config`, `%APPDATA%`
    config: PathBuf,
    /// `~/.local/share`, `%APPDATA%`
    data: PathBuf,
    path: Vec<PathBuf>,
    /// Windows install roots (`Program Files`, `%LOCALAPPDATA%`).
    program_files: Vec<PathBuf>,
}

impl Env {
    fn which(&self, name: &str) -> Option<PathBuf> {
        let name = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
        self.path.iter().map(|d| d.join(&name)).find(|p| p.is_file())
    }

    fn flatpak(&self, id: &str) -> bool {
        self.home.join(".var/app").join(id).is_dir() || Path::new("/var/lib/flatpak/app").join(id).is_dir()
    }

    fn installed_under(&self, rel: &str) -> bool {
        self.program_files.iter().any(|d| d.join(rel).is_file())
    }
}

fn detect_in(env: &Env) -> Vec<Player> {
    let mut out = Vec::new();
    if cfg!(windows) {
        let mpv = env.which("mpv");
        // A portable mpv reads `portable_config` next to it instead of %APPDATA%\mpv.
        let portable = mpv.as_ref().and_then(|p| p.parent()).map(|d| d.join("portable_config")).filter(|d| d.is_dir());
        let mpv_dir = portable.unwrap_or_else(|| env.config.join("mpv"));
        let found = mpv.is_some() || env.config.join("mpv").is_dir() || env.home.join("scoop/apps/mpv").is_dir();
        out.push(Player::new(Kind::Mpv, found, None, mpv_dir.join("scripts").join(FILE_NAME)));

        let found = env.config.join("mpv.net").is_dir()
            || env.installed_under(r"mpv.net\mpvnet.exe")
            || env.installed_under(r"Programs\mpv.net\mpvnet.exe")
            || env.which("mpvnet").is_some();
        out.push(Player::new(Kind::MpvNet, found, None, env.config.join("mpv.net/scripts").join(FILE_NAME)));

        let found = env.installed_under(r"VideoLAN\VLC\vlc.exe") || env.which("vlc").is_some();
        out.push(Player::new(Kind::Vlc, found, None, env.config.join("vlc/lua/extensions").join(FILE_NAME)));
    } else {
        let mpv = env.which("mpv");
        let snap = mpv.as_ref().is_some_and(|p| p.starts_with("/snap"));
        let sandbox = if snap {
            Some("Snap")
        } else if env.flatpak("io.mpv.Mpv") {
            Some("Flatpak")
        } else {
            None
        };
        out.push(Player::new(
            Kind::Mpv,
            mpv.is_some() && !snap,
            sandbox,
            env.config.join("mpv/scripts").join(FILE_NAME),
        ));

        let vlc = env.which("vlc");
        let snap = vlc.as_ref().is_some_and(|p| p.starts_with("/snap"));
        let sandbox = if snap {
            Some("Snap")
        } else if env.flatpak("org.videolan.VLC") {
            Some("Flatpak")
        } else {
            None
        };
        out.push(Player::new(
            Kind::Vlc,
            vlc.is_some() && !snap,
            sandbox,
            env.data.join("vlc/lua/extensions").join(FILE_NAME),
        ));
    }
    out
}

/// Rewrites installed plugins with the current command (after an update, or when the AppImage
/// moved). Returns how many were refreshed.
pub fn refresh(auto: bool) -> usize {
    let Some(cli) = cli_command() else { return 0 };
    detect()
        .iter()
        .filter(|p| p.status == Status::Installed)
        .filter(|p| fs::read_to_string(&p.script).ok().as_deref() != Some(render(p.kind, &cli, auto).as_str()))
        .filter(|p| p.install(&cli, auto).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_the_scripts() {
        let cli = vec!["/opt/Sub Magician/SubMagician.AppImage".to_owned(), "--cli".to_owned()];
        let mpv = render(Kind::Mpv, &cli, true);
        assert!(mpv.contains("local CLI = { [[/opt/Sub Magician/SubMagician.AppImage]], [[--cli]] }"), "{mpv}");
        assert!(mpv.contains("local opts = { auto = true }"));
        let vlc = render(Kind::Vlc, &cli, false);
        assert!(vlc.contains("local AUTO = false") && vlc.contains(&format!("version = \"{VERSION}\"")));
        assert!(!vlc.contains("@@") && !mpv.contains("@@"));
        assert_eq!(lua_string("a]]b]=]c"), "[==[a]]b]=]c]==]");
    }

    #[test]
    fn detects_and_installs() {
        let root = std::env::temp_dir().join(format!("submagician-players-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let exe = |n: &str| if cfg!(windows) { format!("{n}.exe") } else { n.to_owned() };
        fs::write(bin.join(exe("mpv")), "").unwrap();
        let env = Env {
            home: root.join("home"),
            config: root.join("config"),
            data: root.join("data"),
            path: vec![bin.clone()],
            program_files: Vec::new(),
        };
        let players = detect_in(&env);
        let mpv = players.iter().find(|p| p.kind == Kind::Mpv).unwrap();
        assert_eq!(mpv.status, Status::Available);
        let vlc = players.iter().find(|p| p.kind == Kind::Vlc).unwrap();
        assert_eq!(vlc.status, Status::NotFound);

        mpv.install(&["/usr/bin/submagician-cli".into()], false).unwrap();
        assert!(fs::read_to_string(&mpv.script).unwrap().contains("[[/usr/bin/submagician-cli]]"));
        let again = detect_in(&env);
        assert_eq!(again.iter().find(|p| p.kind == Kind::Mpv).unwrap().status, Status::Installed);
        mpv.uninstall().unwrap();
        mpv.uninstall().unwrap(); // twice is fine
        assert!(!mpv.script.exists());

        if !cfg!(windows) {
            fs::create_dir_all(env.home.join(".var/app/org.videolan.VLC")).unwrap();
            let vlc = detect_in(&env).into_iter().find(|p| p.kind == Kind::Vlc).unwrap();
            assert_eq!(vlc.status, Status::Sandboxed("Flatpak"));
            assert_eq!(vlc.script, env.data.join("vlc/lua/extensions/submagician.lua"));
        }
        fs::remove_dir_all(&root).unwrap();
    }
}
