//! Updates: the check at start (and "Check now"), download and install of a newer release, and
//! "What's new" after an update.

use std::sync::atomic::Ordering;

use slint::ComponentHandle;

use submagician_core::update::{self, Format};
use submagician_core::{APP_REPO, Error, whatsnew};

use super::{Controller, ST_ERROR, ST_STOPPED, ST_UP_TO_DATE, ST_UPDATE_DOWNLOAD};
use crate::AppState;

// AppState.update-state
const NONE: i32 = 0;
const CHECKING: i32 = 1;
const INSTALLABLE: i32 = 2;
const MANUAL: i32 = 3;
const DOWNLOADING: i32 = 4;
const RESTARTING: i32 = 5;
const FAILED: i32 = 6;

impl Controller {
    pub(super) fn wire_updates(&self, state: &AppState) {
        state.on_check_for_update({
            let c = self.clone();
            move || c.check_for_update(true)
        });
        state.on_install_update({
            let c = self.clone();
            move || c.install_update()
        });
        state.on_open_release({
            let c = self.clone();
            move || {
                let url = c.shared.release.lock().unwrap().as_ref().map(|r| r.html_url.clone());
                let url = url.unwrap_or_else(|| format!("https://github.com/{APP_REPO}/releases/latest"));
                c.open_url(&url);
            }
        });
        state.on_show_whats_new({
            let c = self.clone();
            move || {
                if let Some(ui) = c.ui.upgrade() {
                    let state = ui.global::<AppState>();
                    state.set_whats_new(whatsnew::current().into());
                    state.set_whats_new_visible(true);
                }
            }
        });
    }

    /// After an update (or at the first start) shows the notes of the new version once.
    pub(super) fn show_whats_new_once(&self, state: &AppState) {
        let last = self.shared.settings.lock().unwrap().last_version_seen.clone();
        if last == submagician_core::VERSION {
            return;
        }
        if let Some(notes) = whatsnew::pending(&last) {
            state.set_whats_new(notes.into());
            state.set_whats_new_visible(true);
        }
        let mut s = self.shared.settings.lock().unwrap();
        s.last_version_seen = submagician_core::VERSION.into();
        if let Err(e) = s.save() {
            log::warn!("settings not saved: {e}");
        }
    }

    /// Asks GitHub for a newer release. `manual`: the user asked, so "up to date" and failures
    /// are shown; the check at start stays quiet about them.
    pub(super) fn check_for_update(&self, manual: bool) {
        self.set_update_state(CHECKING, "", "");
        let c = self.clone();
        self.rt.spawn(async move {
            match update::check().await {
                Ok(Some(release)) => {
                    let version = release.version().to_owned();
                    log::info!("update available: {version}");
                    *c.shared.release.lock().unwrap() = Some(release);
                    let state = if Format::current().is_some() { INSTALLABLE } else { MANUAL };
                    c.set_update_state(state, &version, "");
                }
                Ok(None) => {
                    log::info!("no newer release");
                    c.set_update_state(NONE, "", "");
                    if manual {
                        c.status(ST_UP_TO_DATE, 0, 0, "");
                    }
                }
                Err(e) => {
                    log::warn!("update check failed: {e}");
                    if manual {
                        c.set_update_state(FAILED, "", &e.to_string());
                    } else {
                        c.set_update_state(NONE, "", "");
                    }
                }
            }
        });
    }

    fn set_update_state(&self, value: i32, version: &str, error: &str) {
        let (version, error) = (version.to_owned(), error.to_owned());
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            let state = ui.global::<AppState>();
            state.set_update_state(value);
            if !version.is_empty() {
                state.set_update_version(version.into());
            }
            state.set_update_error(error.into());
        });
    }

    /// Downloads the release, checks it and starts it; SubMagician closes and the new version
    /// starts by itself.
    fn install_update(&self) {
        let (Some(format), Some(release), Some(dir)) =
            (Format::current(), self.shared.release.lock().unwrap().clone(), update::updates_dir())
        else {
            return;
        };
        let version = release.version().to_owned();
        self.set_update_state(DOWNLOADING, &version, "");
        let c = self.clone();
        self.run_busy(async move {
            let reporter = c.clone();
            let mut last = -1;
            let mut progress = move |done: u64, total: Option<u64>| {
                let pct = total.filter(|t| *t > 0).map_or(0, |t| (done * 100 / t) as i32);
                if pct != last {
                    last = pct;
                    reporter.status(ST_UPDATE_DOWNLOAD, pct, 0, "");
                    reporter.progress(pct as f32 / 100.0);
                }
            };
            let result = update::download(&release, format, &dir, &c.shared.cancel, &mut progress).await;
            let started = result.and_then(|file| {
                log::info!("installing update {} from {}", release.tag_name, file.display());
                update::install(format, &file)
            });
            match started {
                Ok(()) => {
                    c.set_update_state(RESTARTING, &version, "");
                    let _ = slint::invoke_from_event_loop(|| {
                        let _ = slint::quit_event_loop();
                    });
                }
                Err(Error::Cancelled) => {
                    c.shared.cancel.store(false, Ordering::SeqCst);
                    c.set_update_state(INSTALLABLE, &version, "");
                    c.status(ST_STOPPED, 0, 0, "");
                }
                Err(e) => {
                    log::error!("update failed: {e}");
                    c.set_update_state(FAILED, &version, &e.to_string());
                    c.status(ST_ERROR, 0, 0, e.to_string());
                }
            }
        });
    }
}
