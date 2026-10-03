//! Logs and "Report a problem".

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::ComponentHandle;

use submagician_core::report::Report;
use submagician_core::update::Format;
use submagician_core::{APP_REPO, SUPPORT_EMAIL, applog, audio, players, speech};

use super::{Controller, ST_ERROR, ST_REPORT_SAVED};
use crate::AppState;

/// The open report and where its full text was saved.
type Open = Rc<RefCell<Option<(Report, Option<PathBuf>)>>>;

impl Controller {
    pub(super) fn wire_support(&self, state: &AppState) {
        let dir = applog::global().map(|b| b.dir().to_path_buf()).or_else(applog::logs_dir);
        state.set_log_dir(dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default().into());
        state.set_support_email(SUPPORT_EMAIL.into());

        state.on_open_log_folder({
            let c = self.clone();
            move || {
                if let Some(dir) = &dir {
                    let _ = std::fs::create_dir_all(dir);
                    if let Err(e) = opener::open(dir) {
                        c.status(ST_ERROR, 0, 0, e.to_string());
                    }
                }
            }
        });
        state.on_open_link({
            let c = self.clone();
            move |which| {
                let url = match which.as_str() {
                    "releases" => format!("https://github.com/{APP_REPO}/releases"),
                    "license" => format!("https://github.com/{APP_REPO}/blob/main/LICENSE"),
                    _ => format!("https://github.com/{APP_REPO}"),
                };
                c.open_url(&url);
            }
        });

        let open: Open = Rc::default();
        state.on_report_problem({
            let (c, open) = (self.clone(), open.clone());
            move || {
                let report = c.collect_report();
                let saved = c.save_report(&report, "");
                if let Some(ui) = c.ui.upgrade() {
                    let state = ui.global::<AppState>();
                    state.set_report_text(report.text("").into());
                    state.set_report_saved_path(
                        saved.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into(),
                    );
                    state.set_report_note("".into());
                    state.set_report_visible(true);
                }
                *open.borrow_mut() = Some((report, saved));
            }
        });
        state.on_report_github({
            let (c, open) = (self.clone(), open.clone());
            move || c.send_report(&open, false)
        });
        state.on_report_email({
            let (c, open) = (self.clone(), open.clone());
            move || c.send_report(&open, true)
        });
        state.on_report_show_file({
            let c = self.clone();
            move || {
                let saved = open.borrow().as_ref().and_then(|(_, p)| p.clone());
                if let Some(path) = saved
                    && let Err(e) = opener::reveal(&path)
                {
                    c.status(ST_ERROR, 0, 0, e.to_string());
                }
            }
        });
    }

    pub(super) fn open_url(&self, url: &str) {
        if let Err(e) = opener::open(url) {
            self.status(ST_ERROR, 0, 0, e.to_string());
        }
    }

    /// The report: what kind of copy this is, the settings that matter for problems (no
    /// passwords or keys) and the recent log.
    fn collect_report(&self) -> Report {
        let s = self.shared.settings.lock().unwrap().clone();
        let kind = match Format::current() {
            Some(Format::WindowsInstaller) => "installer",
            Some(Format::AppImage) => "AppImage",
            None if cfg!(windows) => "portable",
            None => "package or build",
        };
        let sources: Vec<&str> =
            [(s.use_opensubtitles, "OpenSubtitles"), (s.use_subdl, "SubDL"), (s.use_addic7ed, "Addic7ed")]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, name)| *name)
                .collect();
        let ffmpeg = audio::find_ffmpeg(Some(&PathBuf::from(&s.ffmpeg_path)));
        let model = speech::model(&s.whisper_model).and_then(|m| m.installed());
        let plugins: Vec<&str> = players::detect()
            .iter()
            .filter(|p| p.status == players::Status::Installed)
            .map(|p| p.kind.name())
            .collect();
        let videos = self.shared.items.lock().unwrap().len();
        let details = [
            ("Copy", kind.to_owned()),
            ("Languages", s.language_codes().join(", ")),
            ("Sources", sources.join(", ")),
            ("OpenSubtitles login", if s.opensubtitles_username.is_empty() { "no" } else { "yes" }.to_owned()),
            (
                "Sync to audio",
                format!("{} (ffmpeg {})", on_off(s.auto_sync), if ffmpeg.is_some() { "found" } else { "missing" }),
            ),
            (
                "Whisper",
                format!("{} ({})", s.whisper_model, if model.is_some() { "downloaded" } else { "not downloaded" }),
            ),
            ("Videos open", videos.to_string()),
            ("Watch folder", on_off(s.watch).to_owned()),
            ("Player plugins", if plugins.is_empty() { "none".to_owned() } else { plugins.join(", ") }),
            ("Detailed logs", on_off(s.detailed_logs).to_owned()),
        ];
        let log = applog::global().map(|b| b.recent()).unwrap_or_default();
        Report::new(&details, log)
    }

    fn save_report(&self, report: &Report, note: &str) -> Option<PathBuf> {
        let dir = applog::global().map(|b| b.dir().to_path_buf()).or_else(applog::logs_dir)?;
        match report.save(&dir, note) {
            Ok(path) => {
                self.status(ST_REPORT_SAVED, 0, 0, path.display().to_string());
                Some(path)
            }
            Err(e) => {
                log::warn!("report not saved: {e}");
                None
            }
        }
    }

    /// Saves the report again with the note, then opens a GitHub issue or an e-mail.
    fn send_report(&self, open: &Open, email: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        let note = ui.global::<AppState>().get_report_note().to_string();
        let mut open = open.borrow_mut();
        let Some((report, saved)) = open.as_mut() else { return };
        if !note.trim().is_empty() {
            if let Some(old) = saved.take() {
                let _ = std::fs::remove_file(old);
            }
            *saved = self.save_report(report, &note);
            ui.global::<AppState>()
                .set_report_saved_path(saved.as_ref().map(|p| p.display().to_string()).unwrap_or_default().into());
        }
        let url =
            if email { report.mailto_url(&note, saved.as_deref()) } else { report.github_url(&note, saved.as_deref()) };
        self.open_url(&url);
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}
