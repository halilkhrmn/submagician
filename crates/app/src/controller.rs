//! Glue between the window and the core: UI callbacks start work on the tokio runtime, and the
//! work reports back through `upgrade_in_event_loop`.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, VecModel, Weak};
use submagician_core::engine::{Engine, Saved};
use submagician_core::media::{self, MediaFile};
use submagician_core::provider::opensubtitles::{self, Credentials, OpenSubtitles};
use submagician_core::provider::{Candidate, SearchQuery};
use submagician_core::{Error, score};
use tokio::runtime::Handle;

use crate::settings::Settings;
use crate::{AppWindow, CandidateRow, FileRow};

// File states, as in Texts.file-state.
const WAITING: i32 = 0;
const SEARCHING: i32 = 1;
const FOUND: i32 = 2;
const NOTHING: i32 = 3;
const DOWNLOADING: i32 = 4;
const SAVED: i32 = 5;
const FAILED: i32 = 6;
const HAS_SUBTITLE: i32 = 7;

// Status bar kinds, as in Texts.status.
const ST_SCANNING: i32 = 1;
const ST_SCANNED: i32 = 2;
const ST_SEARCHING: i32 = 3;
const ST_DOWNLOADING: i32 = 4;
const ST_FINISHED: i32 = 5;
const ST_ERROR: i32 = 6;
const ST_STOPPED: i32 = 7;
const ST_QUOTA: i32 = 8;
const ST_SAVED_ONE: i32 = 9;
const ST_SETTINGS_SAVED: i32 = 10;
const ST_SEARCH_FINISHED: i32 = 11;

/// Pause between videos in batch runs, to stay well inside provider rate limits.
const BATCH_PAUSE: Duration = Duration::from_millis(250);

struct Item {
    media: MediaFile,
    query: Option<SearchQuery>,
    candidates: Vec<Candidate>,
    searched: bool,
    state: i32,
    detail: String,
}

impl Item {
    fn new(media: MediaFile) -> Item {
        Item { media, query: None, candidates: Vec::new(), searched: false, state: WAITING, detail: String::new() }
    }
}

struct Shared {
    settings: Mutex<Settings>,
    engine: Mutex<Arc<Engine>>,
    root: Mutex<Option<PathBuf>>,
    items: Mutex<Vec<Item>>,
    /// Bumped whenever the file list is replaced, so late results of old work are dropped.
    generation: AtomicU64,
    cancel: AtomicBool,
}

#[derive(Clone)]
pub struct Controller {
    shared: Arc<Shared>,
    ui: Weak<AppWindow>,
    rt: Handle,
}

/// Errors after which a batch run cannot go on.
fn is_fatal(e: &Error) -> bool {
    matches!(e, Error::NotConfigured { .. } | Error::Auth { .. } | Error::Quota { .. })
}

fn build_engine(s: &Settings) -> Arc<Engine> {
    let credentials =
        Credentials { username: s.opensubtitles_username.clone(), password: s.opensubtitles_password.clone() };
    let os = OpenSubtitles::new(Some(s.opensubtitles_api_key.clone()), Some(credentials));
    Arc::new(Engine::new(vec![Arc::new(os)]))
}

impl Controller {
    pub fn start(ui: &AppWindow, settings: Settings, rt: Handle) {
        let last_folder = settings.last_folder.clone();
        apply_settings(ui, &settings);
        ui.set_version(env!("CARGO_PKG_VERSION").into());
        ui.set_has_builtin_key(opensubtitles::BUILT_IN_KEY.is_some());

        let c = Controller {
            shared: Arc::new(Shared {
                engine: Mutex::new(build_engine(&settings)),
                settings: Mutex::new(settings),
                root: Mutex::new(None),
                items: Mutex::new(Vec::new()),
                generation: AtomicU64::new(0),
                cancel: AtomicBool::new(false),
            }),
            ui: ui.as_weak(),
            rt,
        };

        ui.on_choose_folder({
            let c = c.clone();
            move || c.choose_folder()
        });
        ui.on_rescan({
            let c = c.clone();
            move || {
                if let Some(root) = c.shared.root.lock().unwrap().clone() {
                    c.open_folder(root);
                }
            }
        });
        ui.on_search_all({
            let c = c.clone();
            move || c.run_batch(false)
        });
        ui.on_download_all({
            let c = c.clone();
            move || c.run_batch(true)
        });
        ui.on_stop({
            let c = c.clone();
            move || c.shared.cancel.store(true, Ordering::SeqCst)
        });
        ui.on_file_selected({
            let c = c.clone();
            move |i| c.file_selected(i as usize, false)
        });
        ui.on_search_file({
            let c = c.clone();
            move |i| {
                if i >= 0 {
                    c.file_selected(i as usize, true);
                }
            }
        });
        ui.on_download_candidate({
            let c = c.clone();
            move |ci| c.download_candidate(ci)
        });
        ui.on_save_settings({
            let c = c.clone();
            move || c.save_settings()
        });

        if let Some(folder) = last_folder.filter(|f| f.is_dir()) {
            c.open_folder(folder);
        }
    }

    fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::SeqCst)
    }

    fn engine(&self) -> Arc<Engine> {
        self.shared.engine.lock().unwrap().clone()
    }

    fn languages(&self) -> Vec<String> {
        self.shared.settings.lock().unwrap().language_codes()
    }

    /// Runs `work` with the window marked busy. Call from the UI thread.
    fn run_busy(&self, work: impl Future<Output = ()> + Send + 'static) {
        let Some(ui) = self.ui.upgrade() else { return };
        if ui.get_busy() {
            return;
        }
        ui.set_busy(true);
        ui.set_progress(0.0);
        self.shared.cancel.store(false, Ordering::SeqCst);
        let weak = self.ui.clone();
        self.rt.spawn(async move {
            work.await;
            let _ = weak.upgrade_in_event_loop(|ui| {
                ui.set_busy(false);
                ui.set_candidates_loading(false);
                ui.set_progress(0.0);
            });
        });
    }

    fn status(&self, kind: i32, a: i32, b: i32, detail: impl Into<String>) {
        let detail = detail.into();
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            ui.set_status_kind(kind);
            ui.set_status_a(a);
            ui.set_status_b(b);
            ui.set_status_detail(detail.into());
        });
    }

    fn progress(&self, value: f32) {
        let _ = self.ui.upgrade_in_event_loop(move |ui| ui.set_progress(value));
    }

    fn choose_folder(&self) {
        let start = self.shared.root.lock().unwrap().clone();
        let mut dialog = rfd::FileDialog::new();
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        if let Some(folder) = dialog.pick_folder() {
            self.open_folder(folder);
        }
    }

    fn open_folder(&self, folder: PathBuf) {
        let recursive = {
            let mut s = self.shared.settings.lock().unwrap();
            s.last_folder = Some(folder.clone());
            if let Err(e) = s.save() {
                log::warn!("settings not saved: {e}");
            }
            s.recursive
        };
        let epoch = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self.shared.root.lock().unwrap() = Some(folder.clone());
        if let Some(ui) = self.ui.upgrade() {
            ui.set_folder(folder.display().to_string().into());
            ui.set_files(ModelRc::default());
            ui.set_candidates(ModelRc::default());
            ui.set_selected_file(-1);
            ui.set_selected_candidate(-1);
        }
        self.status(ST_SCANNING, 0, 0, "");
        let c = self.clone();
        self.run_busy(async move {
            let scan_dir = folder.clone();
            let found =
                tokio::task::spawn_blocking(move || media::scan(&scan_dir, recursive)).await.unwrap_or_default();
            if c.generation() != epoch {
                return;
            }
            let rows: Vec<FileRow> = {
                let mut items = c.shared.items.lock().unwrap();
                *items = found.into_iter().map(Item::new).collect();
                items.iter().map(|it| file_row(&folder, it)).collect()
            };
            let count = rows.len() as i32;
            let _ = c.ui.upgrade_in_event_loop(move |ui| ui.set_files(ModelRc::new(VecModel::from(rows))));
            c.status(ST_SCANNED, count, 0, "");
        });
    }

    /// Shows the candidates of file `index`, searching first if needed (or if `force`).
    fn file_selected(&self, index: usize, force: bool) {
        let searched = {
            let items = self.shared.items.lock().unwrap();
            match items.get(index) {
                Some(it) => it.searched,
                None => return,
            }
        };
        let epoch = self.generation();
        self.show_candidates(epoch, index);
        let busy = self.ui.upgrade().is_some_and(|ui| ui.get_busy());
        if (searched && !force) || busy {
            return;
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_candidates_loading(true);
            ui.set_candidates(ModelRc::default());
        }
        let c = self.clone();
        self.run_busy(async move {
            if let Err(e) = c.search_item(epoch, index).await {
                c.status(ST_ERROR, 0, 0, e.to_string());
            }
        });
    }

    fn download_candidate(&self, candidate: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        let (Ok(index), Ok(candidate)) = (usize::try_from(ui.get_selected_file()), usize::try_from(candidate)) else {
            return;
        };
        let epoch = self.generation();
        let c = self.clone();
        self.run_busy(async move {
            match c.fetch_item(epoch, index, Some(candidate)).await {
                Ok(Some(saved)) => {
                    let name = saved.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    c.status(ST_SAVED_ONE, saved.remaining.map_or(-1, |r| r as i32), 0, name);
                }
                Ok(None) => {}
                Err(Error::Quota { message, .. }) => c.status(ST_QUOTA, 0, 0, message),
                Err(e) => c.status(ST_ERROR, 0, 0, e.to_string()),
            }
        });
    }

    fn run_batch(&self, download: bool) {
        let epoch = self.generation();
        let c = self.clone();
        self.run_busy(async move { c.batch(epoch, download).await });
    }

    async fn batch(&self, epoch: u64, download: bool) {
        let total = self.shared.items.lock().unwrap().len();
        let (languages, skip_existing) = {
            let s = self.shared.settings.lock().unwrap();
            (s.language_codes(), s.skip_existing)
        };
        let (mut saved, mut missing, mut with_subs, mut searched) = (0, 0, 0, 0);
        for i in 0..total {
            if self.generation() != epoch {
                return;
            }
            if self.shared.cancel.load(Ordering::SeqCst) {
                self.status(ST_STOPPED, 0, 0, "");
                return;
            }
            self.status(if download { ST_DOWNLOADING } else { ST_SEARCHING }, i as i32 + 1, total as i32, "");
            self.progress(i as f32 / total as f32);

            let (has_first, was_searched) = {
                let items = self.shared.items.lock().unwrap();
                let it = &items[i];
                (it.media.has_language(&languages[0]), it.searched)
            };
            if skip_existing && has_first {
                self.set_state(epoch, i, HAS_SUBTITLE, "");
                continue;
            }
            if !was_searched {
                searched += 1;
                match self.search_item(epoch, i).await {
                    Ok(count) if count > 0 => with_subs += 1,
                    Ok(_) => {}
                    Err(e) => return self.stop_with(e),
                }
                tokio::time::sleep(BATCH_PAUSE).await;
            }
            if download {
                match self.fetch_item(epoch, i, None).await {
                    Ok(Some(_)) => saved += 1,
                    Ok(None) => missing += 1,
                    Err(e) if is_fatal(&e) => return self.stop_with(e),
                    Err(_) => missing += 1,
                }
                tokio::time::sleep(BATCH_PAUSE).await;
            }
        }
        self.progress(1.0);
        if download {
            self.status(ST_FINISHED, saved, missing, "");
        } else {
            self.status(ST_SEARCH_FINISHED, with_subs, searched, "");
        }
    }

    fn stop_with(&self, e: Error) {
        match e {
            Error::Quota { message, .. } => self.status(ST_QUOTA, 0, 0, message),
            e => self.status(ST_ERROR, 0, 0, e.to_string()),
        }
    }

    /// Searches every provider for file `index`. Returns how many candidates were found, or
    /// the error when nothing was found and the reason is one a batch cannot continue past.
    async fn search_item(&self, epoch: u64, index: usize) -> Result<usize, Error> {
        let languages = self.languages();
        let Some(media) = self.shared.items.lock().unwrap().get(index).map(|it| it.media.clone()) else {
            return Ok(0);
        };
        self.set_state(epoch, index, SEARCHING, "");
        let query = tokio::task::spawn_blocking(move || Engine::query_for(&media, &languages))
            .await
            .map_err(|e| Error::Parse(e.to_string()))?;
        let outcome = self.engine().search(&query).await;
        let count = outcome.candidates.len();
        let fatal = if count == 0 { outcome.errors.into_iter().find(is_fatal) } else { None };
        let (state, detail) = match (&fatal, count) {
            (Some(e), _) => (FAILED, e.to_string()),
            (None, 0) => (NOTHING, String::new()),
            (None, n) => (FOUND, n.to_string()),
        };
        {
            let mut items = self.shared.items.lock().unwrap();
            if self.generation() != epoch {
                return Ok(0);
            }
            let it = &mut items[index];
            it.query = Some(query);
            it.candidates = outcome.candidates;
            it.searched = fatal.is_none();
        }
        self.set_state(epoch, index, state, detail);
        self.show_candidates(epoch, index);
        match fatal {
            Some(e) => Err(e),
            None => Ok(count),
        }
    }

    /// Downloads candidate `candidate` of file `index`, or the best one when `None`.
    async fn fetch_item(&self, epoch: u64, index: usize, candidate: Option<usize>) -> Result<Option<Saved>, Error> {
        let languages = self.languages();
        let picked = {
            let items = self.shared.items.lock().unwrap();
            let Some(it) = items.get(index) else { return Ok(None) };
            let ci = candidate.or_else(|| score::best(&it.candidates, &languages));
            match (ci.and_then(|ci| it.candidates.get(ci)), &it.query) {
                (Some(cand), Some(query)) => Some((it.media.clone(), query.clone(), cand.clone())),
                _ => None,
            }
        };
        let Some((media, query, cand)) = picked else { return Ok(None) };
        self.set_state(epoch, index, DOWNLOADING, "");
        match self.engine().fetch(&media, &query, &cand).await {
            Ok(saved) => {
                let existing = media::existing_subtitles(&media.path);
                {
                    let mut items = self.shared.items.lock().unwrap();
                    if self.generation() == epoch {
                        items[index].media.existing = existing;
                    }
                }
                let name = saved.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                log::info!("saved {} (was {})", saved.path.display(), saved.source_encoding);
                self.set_state(epoch, index, SAVED, name);
                Ok(Some(saved))
            }
            Err(e) => {
                self.set_state(epoch, index, FAILED, e.to_string());
                Err(e)
            }
        }
    }

    fn set_state(&self, epoch: u64, index: usize, state: i32, detail: impl Into<String>) {
        let root = self.shared.root.lock().unwrap().clone().unwrap_or_default();
        let row = {
            let mut items = self.shared.items.lock().unwrap();
            if self.generation() != epoch {
                return;
            }
            let Some(it) = items.get_mut(index) else { return };
            it.state = state;
            it.detail = detail.into();
            file_row(&root, it)
        };
        let shared = self.shared.clone();
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            let files = ui.get_files();
            if shared.generation.load(Ordering::SeqCst) == epoch && index < files.row_count() {
                files.set_row_data(index, row);
            }
        });
    }

    fn show_candidates(&self, epoch: u64, index: usize) {
        let rows: Vec<CandidateRow> = {
            let items = self.shared.items.lock().unwrap();
            match items.get(index) {
                Some(it) => it.candidates.iter().map(candidate_row).collect(),
                None => return,
            }
        };
        let shared = self.shared.clone();
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            if shared.generation.load(Ordering::SeqCst) == epoch && ui.get_selected_file() == index as i32 {
                ui.set_candidates(ModelRc::new(VecModel::from(rows)));
                ui.set_selected_candidate(-1);
                ui.set_candidates_loading(false);
            }
        });
    }

    fn save_settings(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let (engine, languages_changed, normalized) = {
            let mut s = self.shared.settings.lock().unwrap();
            let before = s.language_codes();
            read_settings(&ui, &mut s);
            if let Err(e) = s.save() {
                log::warn!("settings not saved: {e}");
            }
            let codes = s.language_codes();
            (build_engine(&s), before != codes, codes.join(", "))
        };
        *self.shared.engine.lock().unwrap() = engine;
        ui.set_languages(normalized.into());
        if languages_changed {
            // Old results were for other languages.
            let mut items = self.shared.items.lock().unwrap();
            for it in items.iter_mut() {
                it.searched = false;
                it.candidates.clear();
            }
            ui.set_candidates(ModelRc::default());
        }
        self.status(ST_SETTINGS_SAVED, 0, 0, "");
    }
}

fn apply_settings(ui: &AppWindow, s: &Settings) {
    ui.set_languages(s.language_codes().join(", ").into());
    ui.set_recursive(s.recursive);
    ui.set_skip_existing(s.skip_existing);
    ui.set_ui_language(match s.ui_language.as_str() {
        "en" => 1,
        "tr" => 2,
        _ => 0,
    });
    ui.set_os_username(s.opensubtitles_username.clone().into());
    ui.set_os_password(s.opensubtitles_password.clone().into());
    ui.set_os_api_key(s.opensubtitles_api_key.clone().into());
}

fn read_settings(ui: &AppWindow, s: &mut Settings) {
    s.languages = ui.get_languages().into();
    s.recursive = ui.get_recursive();
    s.skip_existing = ui.get_skip_existing();
    s.ui_language = match ui.get_ui_language() {
        1 => "en",
        2 => "tr",
        _ => "auto",
    }
    .into();
    s.opensubtitles_username = ui.get_os_username().trim().into();
    s.opensubtitles_password = ui.get_os_password().into();
    s.opensubtitles_api_key = ui.get_os_api_key().trim().into();
}

fn file_row(root: &Path, it: &Item) -> FileRow {
    let folder =
        it.media.path.parent().map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string()).unwrap_or_default();
    let mut langs: Vec<&str> = it.media.existing.iter().map(|s| s.language.unwrap_or("?")).collect();
    langs.dedup();
    FileRow {
        name: it.media.file_name().into(),
        folder: folder.into(),
        existing: langs.join(", ").into(),
        state: it.state,
        detail: it.detail.clone().into(),
    }
}

fn candidate_row(c: &Candidate) -> CandidateRow {
    let fps =
        c.fps.map(|f| format!("{f:.3}").trim_end_matches('0').trim_end_matches('.').to_owned()).unwrap_or_default();
    CandidateRow {
        score: c.score,
        language: c.language.clone().into(),
        release: c.release.clone().into(),
        provider: c.provider.into(),
        downloads: c.downloads.min(i32::MAX as u64) as i32,
        hash_match: c.hash_match,
        trusted: c.trusted,
        machine: c.machine_translated,
        hearing_impaired: c.hearing_impaired,
        fps: fps.into(),
    }
}
