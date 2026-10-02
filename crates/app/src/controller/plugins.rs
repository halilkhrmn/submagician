//! Player plugins page: which players are here and the plugin in each.

use slint::{ComponentHandle, ModelRc, VecModel};
use submagician_core::players::{self, Kind, Status};

use super::{Controller, ST_ERROR, ST_PLUGIN_INSTALLED, ST_PLUGIN_REMOVED};
use crate::{AppState, PlayerRow};

impl Controller {
    pub(super) fn wire_plugins(&self, state: &AppState) {
        // Installed plugins get the current command (after an update, or a moved AppImage).
        let auto = self.shared.settings.lock().unwrap().player_auto;
        let refreshed = players::refresh(auto);
        if refreshed > 0 {
            log::info!("refreshed {refreshed} player plugins");
        }
        self.show_players();
        state.on_refresh_players({
            let c = self.clone();
            move || c.show_players()
        });
        state.on_install_player({
            let c = self.clone();
            move |i| c.player_action(i, true)
        });
        state.on_remove_player({
            let c = self.clone();
            move |i| c.player_action(i, false)
        });
        state.on_set_player_auto({
            let c = self.clone();
            move |on| {
                {
                    let mut s = c.shared.settings.lock().unwrap();
                    s.player_auto = on;
                    if let Err(e) = s.save() {
                        log::warn!("settings not saved: {e}");
                    }
                }
                players::refresh(on);
            }
        });
    }

    fn show_players(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let rows: Vec<PlayerRow> = players::detect()
            .iter()
            .map(|p| PlayerRow {
                name: p.kind.name().into(),
                status: match p.status {
                    Status::NotFound => 0,
                    Status::Sandboxed(_) => 1,
                    Status::Available => 2,
                    Status::Installed => 3,
                },
                detail: match &p.status {
                    Status::Sandboxed(sandbox) => (*sandbox).into(),
                    _ => p.script.display().to_string().into(),
                },
                kind: match p.kind {
                    Kind::Mpv => 0,
                    Kind::MpvNet => 1,
                    Kind::Vlc => 2,
                },
            })
            .collect();
        let state = ui.global::<AppState>();
        state.set_players(ModelRc::new(VecModel::from(rows)));
        state.set_cli_found(players::cli_command().is_some());
        state.set_player_auto(self.shared.settings.lock().unwrap().player_auto);
    }

    fn player_action(&self, index: i32, install: bool) {
        let Some(player) = usize::try_from(index).ok().and_then(|i| players::detect().into_iter().nth(i)) else {
            return;
        };
        let name = player.kind.name();
        let result = if install {
            let auto = self.shared.settings.lock().unwrap().player_auto;
            match players::cli_command() {
                Some(cli) => player.install(&cli, auto),
                None => Err(submagician_core::Error::Io(std::io::Error::other("submagician-cli not found"))),
            }
        } else {
            player.uninstall()
        };
        match result {
            Ok(()) => {
                log::info!(
                    "{} the {name} plugin at {}",
                    if install { "installed" } else { "removed" },
                    player.script.display()
                );
                self.status(if install { ST_PLUGIN_INSTALLED } else { ST_PLUGIN_REMOVED }, 0, 0, name);
            }
            Err(e) => {
                log::error!("{name} plugin: {e}");
                self.status(ST_ERROR, 0, 0, e.to_string());
            }
        }
        self.show_players();
    }
}
