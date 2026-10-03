//! How this copy of SubMagician was installed: the command other programs (file manager, player
//! plugins) use to start it, and whether updates come from a package manager.

use std::path::PathBuf;

/// The Flatpak application ID (packaging/flatpak/).
pub const FLATPAK_APP_ID: &str = "io.github.halilkhrmn.SubMagician";

/// Who updates this copy, when not SubMagician itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    /// Flatpak (Flathub or a bundle): `flatpak update`, or the software center.
    Flatpak,
    /// A package from a repository (Fedora COPR): dnf or the software center.
    System,
}

/// Who built this copy for a package repository (`SUBMAGICIAN_PACKAGER` at build time, e.g.
/// "fedora-copr"); shown in reports.
pub const PACKAGER: Option<&str> = option_env!("SUBMAGICIAN_PACKAGER");

/// Running inside the Flatpak sandbox: its app ID.
pub fn flatpak_id() -> Option<String> {
    std::env::var("FLATPAK_ID").ok().filter(|id| !id.is_empty())
}

/// The package manager that owns this copy, if any.
pub fn manager() -> Option<Manager> {
    if flatpak_id().is_some() {
        return Some(Manager::Flatpak);
    }
    // Set by the builds a package repository updates (the Fedora COPR spec), not by the .deb on
    // the release page, which apt does not update.
    if PACKAGER.is_some() && std::env::var_os("APPIMAGE").is_none() {
        return Some(Manager::System);
    }
    None
}

/// The user's config and data folders as other programs see them (`~/.config`,
/// `~/.local/share`): inside the Flatpak sandbox the XDG folders point into `~/.var/app`, where
/// players and file managers do not look.
pub fn host_dirs() -> Option<(PathBuf, PathBuf)> {
    let base = directories::BaseDirs::new()?;
    if flatpak_id().is_some() {
        let home = base.home_dir();
        return Some((home.join(".config"), home.join(".local/share")));
    }
    Some((base.config_dir().to_path_buf(), base.data_dir().to_path_buf()))
}

/// The command that starts the app from outside: `flatpak run <id>` in the sandbox (its own paths
/// mean nothing outside it), the AppImage when running from one, else this executable.
pub fn launch_command() -> Option<Vec<String>> {
    if let Some(id) = flatpak_id() {
        return Some(vec!["flatpak".into(), "run".into(), id]);
    }
    let program = match std::env::var_os("APPIMAGE") {
        Some(appimage) => PathBuf::from(appimage),
        None => std::env::current_exe().ok()?,
    };
    Some(vec![program.display().to_string()])
}
