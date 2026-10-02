//! Glue between the window and the core: UI callbacks start work on the tokio runtime, and the
//! work reports back through `upgrade_in_event_loop`.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, VecModel, Weak};
use submagician_core::cache::SearchCache;
use submagician_core::engine::{Engine, Saved};
use submagician_core::media::{self, MediaFile};
use submagician_core::provider::opensubtitles;
use submagician_core::provider::subdl;
use submagician_core::provider::{Candidate, SearchQuery};
use submagician_core::settings::Settings;
use submagician_core::sync::{self, Report, Span};
use submagician_core::{Error, audio, integration, name, output, probe, score, speech, tools, watch};
use tokio::runtime::Handle;

use crate::{AppState, AppWindow, CandidateRow, FileRow};

mod plugins;
mod support;
mod updates;

// File states, as in Texts.file-state.
const WAITING: i32 = 0;
const SEARCHING: i32 = 1;
const FOUND: i32 = 2;
const NOTHING: i32 = 3;
const DOWNLOADING: i32 = 4;
const SAVED: i32 = 5;
const FAILED: i32 = 6;
const HAS_SUBTITLE: i32 = 7;
const SYNCING: i32 = 8;
const SYNCED: i32 = 9;
const SYNC_FAILED: i32 = 10;
const TIMING_OK: i32 = 11;
const GENERATING: i32 = 12;
const GENERATED: i32 = 13;

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
const ST_SEARCH_FINISHED: i32 = 11;
const ST_AUDIO: i32 = 12;
const ST_SYNCED: i32 = 13;
const ST_NO_FFMPEG: i32 = 14;
const ST_TIMING_OK: i32 = 15;
const ST_SHIFTED: i32 = 16;
const ST_NO_SUBTITLE: i32 = 17;
const ST_CACHE_CLEARED: i32 = 18;
const ST_MENU_ADDED: i32 = 19;
const ST_MENU_REMOVED: i32 = 20;
const ST_WATCH_NEW: i32 = 21;
const ST_RESTORED: i32 = 22;
const ST_NO_BACKUP: i32 = 23;
const ST_MODEL_DOWNLOAD: i32 = 24;
const ST_MODEL_READY: i32 = 25;
const ST_LISTENING: i32 = 26;
const ST_GENERATED: i32 = 27;
const ST_NO_MODEL: i32 = 28;
const ST_FFMPEG_DOWNLOAD: i32 = 29;
const ST_FFMPEG_READY: i32 = 30;
const ST_PLUGIN_INSTALLED: i32 = 31;
const ST_PLUGIN_REMOVED: i32 = 32;
const ST_UPDATE_DOWNLOAD: i32 = 33;
const ST_UP_TO_DATE: i32 = 34;
const ST_REPORT_SAVED: i32 = 35;

/// Typed settings are saved once the typing pauses for this long.
const SAVE_DELAY: Duration = Duration::from_millis(700);

/// A new video in a watched folder is handled once its size has not changed for this long.
const WATCH_SETTLE: Duration = Duration::from_secs(10);

/// What happened to one video in a run.
#[derive(Default)]
struct ItemDone {
    skipped: bool,
    searched: bool,
    found: bool,
    saved: bool,
}

/// Pause between videos in batch runs, to stay well inside provider rate limits.
const BATCH_PAUSE: Duration = Duration::from_millis(250);

struct Item {
    media: MediaFile,
    query: Option<SearchQuery>,
    candidates: Vec<Candidate>,
    searched: bool,
    state: i32,
    detail: String,
    /// Subtitle saved by this session, the one to sync or shift.
    subtitle: Option<PathBuf>,
    /// Subtitle tracks inside the video were read (or cannot be: no ffprobe).
    probed: bool,
    /// What to search for instead of what the file name says ("Search as").
    search_as: Option<String>,
}

impl Item {
    fn new(media: MediaFile) -> Item {
        Item {
            media,
            query: None,
            candidates: Vec::new(),
            searched: false,
            state: WAITING,
            detail: String::new(),
            subtitle: None,
            probed: false,
            search_as: None,
        }
    }
}

struct Shared {
    settings: Mutex<Settings>,
    engine: Mutex<Arc<Engine>>,
    root: Mutex<Option<PathBuf>>,
    items: Mutex<Vec<Item>>,
    /// Bumped whenever the file list is replaced, so late results of old work are dropped.
    generation: AtomicU64,
    cancel: Arc<AtomicBool>,
    /// Speech spans per video, so syncing again does not decode the audio again.
    speech: Mutex<HashMap<PathBuf, Arc<Vec<Span>>>>,
    /// Video to select once the folder scan that is running now has finished.
    select_after_scan: Mutex<Option<PathBuf>>,
    /// When videos were opened one by one (not a folder): the list to show instead of a scan.
    opened_files: Mutex<Option<Vec<PathBuf>>>,
    /// The folder watch, while it is on.
    watcher: Mutex<Option<watch::WatchHandle>>,
    watch_tx: tokio::sync::mpsc::UnboundedSender<(u64, PathBuf)>,
    /// A newer release found by the update check.
    release: Mutex<Option<submagician_core::update::Release>>,
}

/// What to sync a subtitle to.
enum Reference {
    Audio,
    Subtitle(PathBuf),
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
    Arc::new(s.engine())
}

impl Shared {
    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }
}

impl Controller {
    /// Wires the window to the core. `initial` is a folder or video given on the command line
    /// (file-manager action); without it the last folder is opened again.
    pub fn start(ui: &AppWindow, settings: Settings, rt: Handle, initial: Option<PathBuf>) -> Controller {
        let last_folder = settings.last_folder.clone();
        apply_settings(ui, &settings);
        ui.global::<AppState>().set_ffmpeg_found(ffmpeg_text(&settings).into());
        ui.global::<AppState>().set_version(submagician_core::VERSION.into());
        ui.global::<AppState>().set_has_builtin_key(opensubtitles::BUILT_IN_KEY.is_some());
        ui.global::<AppState>().set_has_subdl_key(subdl::BUILT_IN_KEY.is_some());

        let (watch_tx, watch_rx) = tokio::sync::mpsc::unbounded_channel();
        let c = Controller {
            shared: Arc::new(Shared {
                engine: Mutex::new(build_engine(&settings)),
                settings: Mutex::new(settings),
                root: Mutex::new(None),
                items: Mutex::new(Vec::new()),
                generation: AtomicU64::new(0),
                cancel: Arc::new(AtomicBool::new(false)),
                speech: Mutex::new(HashMap::new()),
                select_after_scan: Mutex::new(None),
                opened_files: Mutex::new(None),
                watcher: Mutex::new(None),
                watch_tx,
                release: Mutex::new(None),
            }),
            ui: ui.as_weak(),
            rt,
        };

        ui.global::<AppState>().on_choose_folder({
            let c = c.clone();
            move || c.choose_folder()
        });
        ui.global::<AppState>().on_choose_videos({
            let c = c.clone();
            move || c.choose_videos()
        });
        ui.global::<AppState>().on_download_ffmpeg({
            let c = c.clone();
            move || c.download_ffmpeg()
        });
        ui.global::<AppState>().set_can_download_ffmpeg(cfg!(windows));
        ui.global::<AppState>().on_rescan({
            let c = c.clone();
            move || {
                let files = c.shared.opened_files.lock().unwrap().clone();
                match (files, c.shared.root.lock().unwrap().clone()) {
                    (Some(files), _) => c.open_files(files),
                    (None, Some(root)) => c.open_folder(root),
                    _ => {}
                }
            }
        });
        ui.global::<AppState>().on_search_all({
            let c = c.clone();
            move || c.run_batch(false)
        });
        ui.global::<AppState>().on_download_all({
            let c = c.clone();
            move || c.run_batch(true)
        });
        ui.global::<AppState>().on_stop({
            let c = c.clone();
            move || c.shared.cancel.store(true, Ordering::SeqCst)
        });
        ui.global::<AppState>().on_file_selected({
            let c = c.clone();
            move |i| c.file_selected(i as usize, false)
        });
        ui.global::<AppState>().on_search_file({
            let c = c.clone();
            move |i| {
                if i >= 0 {
                    c.file_selected(i as usize, true);
                }
            }
        });
        ui.global::<AppState>().on_download_candidate({
            let c = c.clone();
            move |ci| c.download_candidate(ci)
        });
        ui.global::<AppState>().on_sync_audio({
            let c = c.clone();
            move |i| c.start_sync(i, false)
        });
        ui.global::<AppState>().on_sync_reference({
            let c = c.clone();
            move |i| c.start_sync(i, true)
        });
        ui.global::<AppState>().on_shift({
            let c = c.clone();
            move |i, ms| c.shift(i, ms)
        });
        ui.global::<AppState>().on_clear_cache({
            let c = c.clone();
            move || {
                let cleared = Settings::search_cache_dir().map(|d| SearchCache::new(d).clear());
                match cleared {
                    Some(Err(e)) => c.status(ST_ERROR, 0, 0, e.to_string()),
                    _ => c.status(ST_CACHE_CLEARED, 0, 0, ""),
                }
            }
        });
        ui.global::<AppState>().on_settings_changed({
            let c = c.clone();
            move || c.save_settings()
        });
        let save_timer = std::rc::Rc::new(slint::Timer::default());
        ui.global::<AppState>().on_settings_edited({
            let c = c.clone();
            move || {
                let c = c.clone();
                save_timer.start(slint::TimerMode::SingleShot, SAVE_DELAY, move || c.save_settings());
            }
        });

        ui.global::<AppState>().set_menu_installed(integration::is_installed());
        ui.global::<AppState>().on_set_menu({
            let c = c.clone();
            move |add| c.set_menu(add)
        });
        rt_spawn_watch_worker(&c, watch_rx);
        ui.global::<AppState>().on_toggle_watch({
            let c = c.clone();
            move |on| {
                {
                    let mut s = c.shared.settings.lock().unwrap();
                    s.watch = on;
                    if let Err(e) = s.save() {
                        log::warn!("settings not saved: {e}");
                    }
                }
                c.update_watch();
            }
        });
        ui.global::<AppState>().on_generate({
            let c = c.clone();
            move |i| {
                let Ok(index) = usize::try_from(i) else { return };
                let epoch = c.generation();
                let w = c.clone();
                c.run_busy(async move {
                    let _ = w.generate_item(epoch, index).await;
                });
            }
        });
        ui.global::<AppState>().on_download_model({
            let c = c.clone();
            move || c.download_model()
        });
        ui.global::<AppState>().on_whisper_model_changed({
            let c = c.clone();
            move |index| {
                let Some(model) = usize::try_from(index).ok().and_then(|i| speech::MODELS.get(i)) else { return };
                {
                    let mut s = c.shared.settings.lock().unwrap();
                    s.whisper_model = model.id.into();
                    if let Err(e) = s.save() {
                        log::warn!("settings not saved: {e}");
                    }
                }
                if let Some(ui) = c.ui.upgrade() {
                    ui.global::<AppState>().set_whisper_installed(model.installed().is_some());
                }
            }
        });
        ui.global::<AppState>().on_search_as({
            let c = c.clone();
            move |i, text| {
                let Ok(index) = usize::try_from(i) else { return };
                let text = text.trim().to_owned();
                if let Some(it) = c.shared.items.lock().unwrap().get_mut(index) {
                    it.search_as = (!text.is_empty()).then_some(text);
                }
                c.file_selected(index, true);
            }
        });
        ui.global::<AppState>().on_restore({
            let c = c.clone();
            move |i| c.restore(i)
        });
        ui.global::<AppState>().on_play({
            let c = c.clone();
            move |i| c.with_video(i, |p| opener::open(p).map_err(|e| e.to_string()))
        });
        ui.global::<AppState>().on_reveal({
            let c = c.clone();
            move |i| c.with_video(i, |p| opener::reveal(p).map_err(|e| e.to_string()))
        });

        let state = ui.global::<AppState>();
        c.wire_updates(&state);
        c.wire_support(&state);
        c.wire_plugins(&state);
        c.show_whats_new_once(&state);
        if c.shared.settings.lock().unwrap().check_updates {
            c.check_for_update(false);
        }

        match initial {
            Some(path) => c.open_path(path),
            None => {
                if let Some(folder) = last_folder.filter(|f| f.is_dir()) {
                    c.open_folder(folder);
                }
            }
        }
        c
    }

    /// Puts back the subtitle that the last download replaced (and the other way round).
    fn restore(&self, index: i32) {
        let Ok(index) = usize::try_from(index) else { return };
        let Some(target) = self.target_subtitle(index) else {
            self.status(ST_NO_BACKUP, 0, 0, "");
            return;
        };
        match output::restore_backup(&target) {
            Ok(true) => self.status(ST_RESTORED, 0, 0, file_name(&target)),
            Ok(false) => self.status(ST_NO_BACKUP, 0, 0, ""),
            Err(e) => self.status(ST_ERROR, 0, 0, e.to_string()),
        }
    }

    /// Windows: downloads ffmpeg and ffprobe into the tools folder (Settings → Timing).
    fn download_ffmpeg(&self) {
        let Some(dir) = tools::tools_dir() else { return };
        let c = self.clone();
        self.run_busy(async move {
            let reporter = c.clone();
            let mut last = -1;
            let mut progress = move |done: u64, total: Option<u64>| {
                let pct = total.filter(|t| *t > 0).map_or(0, |t| (done * 100 / t) as i32);
                if pct != last {
                    last = pct;
                    reporter.status(ST_FFMPEG_DOWNLOAD, pct, 0, "");
                    reporter.progress(pct as f32 / 100.0);
                }
            };
            match tools::download_ffmpeg(&dir, &c.shared.cancel, &mut progress).await {
                Ok(()) => c.status(ST_FFMPEG_READY, 0, 0, ""),
                Err(Error::Cancelled) => c.status(ST_STOPPED, 0, 0, ""),
                Err(e) => c.status(ST_ERROR, 0, 0, e.to_string()),
            }
            let found = ffmpeg_text(&c.shared.settings.lock().unwrap());
            let _ = c.ui.upgrade_in_event_loop(move |ui| ui.global::<AppState>().set_ffmpeg_found(found.into()));
        });
    }

    /// Adds or removes "Find subtitles" in the file manager.
    fn set_menu(&self, add: bool) {
        let result = if add {
            match integration::current_program() {
                Some(program) => integration::install(&program),
                None => Err(Error::Io(std::io::Error::other("cannot find SubMagician's own path"))),
            }
        } else {
            integration::uninstall()
        };
        match result {
            Ok(()) => self.status(if add { ST_MENU_ADDED } else { ST_MENU_REMOVED }, 0, 0, ""),
            Err(e) => self.status(ST_ERROR, 0, 0, e.to_string()),
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<AppState>().set_menu_installed(integration::is_installed());
        }
    }

    /// Opens a folder, or a video's folder with that video selected (command line, drop).
    pub fn open_path(&self, path: PathBuf) {
        if path.is_dir() {
            self.open_folder(path);
        } else if path.is_file() {
            self.open_files(vec![path]);
        }
    }

    /// Lists just these videos (not their whole folder) and selects the first.
    pub fn open_files(&self, files: Vec<PathBuf>) {
        let Some(folder) = files.first().and_then(|f| f.parent()).map(Path::to_path_buf) else { return };
        *self.shared.select_after_scan.lock().unwrap() = files.first().cloned();
        self.open(folder, Some(files));
    }

    fn open_folder(&self, folder: PathBuf) {
        self.open(folder, None);
    }

    /// Runs `action` on the path of video `index` and reports a failure in the status bar.
    fn with_video(&self, index: i32, action: impl FnOnce(&Path) -> Result<(), String>) {
        let path = usize::try_from(index)
            .ok()
            .and_then(|i| self.shared.items.lock().unwrap().get(i).map(|it| it.media.path.clone()));
        if let Some(path) = path
            && let Err(e) = action(&path)
        {
            self.status(ST_ERROR, 0, 0, e);
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
        if ui.global::<AppState>().get_busy() {
            return;
        }
        ui.global::<AppState>().set_busy(true);
        ui.global::<AppState>().set_progress(0.0);
        self.shared.cancel.store(false, Ordering::SeqCst);
        let weak = self.ui.clone();
        self.rt.spawn(async move {
            work.await;
            let _ = weak.upgrade_in_event_loop(|ui| {
                ui.global::<AppState>().set_busy(false);
                ui.global::<AppState>().set_candidates_loading(false);
                ui.global::<AppState>().set_progress(0.0);
            });
        });
    }

    fn status(&self, kind: i32, a: i32, b: i32, detail: impl Into<String>) {
        let detail = detail.into();
        if kind == ST_ERROR || kind == ST_NO_FFMPEG {
            log::warn!("status {kind}: {detail}");
        }
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            ui.global::<AppState>().set_status_kind(kind);
            ui.global::<AppState>().set_status_a(a);
            ui.global::<AppState>().set_status_b(b);
            ui.global::<AppState>().set_status_detail(detail.into());
        });
    }

    fn progress(&self, value: f32) {
        let _ = self.ui.upgrade_in_event_loop(move |ui| ui.global::<AppState>().set_progress(value));
    }

    /// A file dialog owned by the window, so it opens in front of it (an ownerless dialog on
    /// Windows can open behind the window, which then looks frozen).
    fn dialog(&self, start: Option<PathBuf>) -> rfd::FileDialog {
        let mut dialog = rfd::FileDialog::new();
        if let Some(ui) = self.ui.upgrade() {
            dialog = dialog.set_parent(&ui.window().window_handle());
        }
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        dialog
    }

    fn choose_folder(&self) {
        let start = self.shared.root.lock().unwrap().clone();
        if let Some(folder) = self.dialog(start).pick_folder() {
            self.open_folder(folder);
        }
    }

    fn choose_videos(&self) {
        let start = self.shared.root.lock().unwrap().clone();
        if let Some(files) = self.dialog(start).add_filter("Videos", media::VIDEO_EXTS).pick_files() {
            self.open_files(files);
        }
    }

    /// Shows the videos of `folder`, or only `files` when given.
    fn open(&self, folder: PathBuf, files: Option<Vec<PathBuf>>) {
        if self.ui.upgrade().is_some_and(|ui| ui.global::<AppState>().get_busy()) {
            return;
        }
        log::info!(
            "open {} ({})",
            folder.display(),
            files.as_ref().map_or("folder".into(), |f| format!("{} files", f.len()))
        );
        *self.shared.opened_files.lock().unwrap() = files.clone();
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
            let shown = match files.as_deref() {
                Some([one]) => one.display().to_string(),
                _ => folder.display().to_string(),
            };
            let name = match files.as_deref() {
                Some([one]) => one.file_name(),
                _ => folder.file_name(),
            };
            let name = name.map_or_else(|| shown.clone(), |n| n.to_string_lossy().into_owned());
            ui.global::<AppState>().set_folder(shown.into());
            ui.global::<AppState>().set_folder_name(name.into());
            ui.global::<AppState>().set_files(ModelRc::default());
            ui.global::<AppState>().set_candidates(ModelRc::default());
            ui.global::<AppState>().set_selected_file(-1);
            ui.global::<AppState>().set_selected_candidate(-1);
        }
        self.status(ST_SCANNING, 0, 0, "");
        let c = self.clone();
        self.run_busy(async move {
            let scan_dir = folder.clone();
            let found = tokio::task::spawn_blocking(move || match files {
                Some(files) => files.iter().filter_map(|f| media::media_file(f)).collect(),
                None => media::scan(&scan_dir, recursive),
            })
            .await
            .unwrap_or_default();
            if c.generation() != epoch {
                return;
            }
            let first = c.languages().swap_remove(0);
            let rows: Vec<FileRow> = {
                let mut items = c.shared.items.lock().unwrap();
                *items = found.into_iter().map(Item::new).collect();
                items.iter().map(|it| file_row(&folder, it, &first)).collect()
            };
            let count = rows.len() as i32;
            let select = c.shared.select_after_scan.lock().unwrap().take();
            let selected = select.and_then(|p| c.shared.items.lock().unwrap().iter().position(|it| it.media.path == p));
            let _ = c.ui.upgrade_in_event_loop(move |ui| {
                ui.global::<AppState>().set_files(ModelRc::new(VecModel::from(rows)));
                if let Some(i) = selected {
                    ui.global::<AppState>().set_selected_file(i as i32);
                }
            });
            c.status(ST_SCANNED, count, 0, "");
            let watcher = c.clone();
            let _ = c.ui.upgrade_in_event_loop(move |_| watcher.update_watch());
            // Read the subtitle tracks inside the videos in the background; the window stays usable.
            let bg = c.clone();
            tokio::spawn(async move {
                for i in 0..count as usize {
                    if bg.generation() != epoch {
                        return;
                    }
                    bg.probe_item(epoch, i).await;
                }
            });
        });
    }

    /// Reads the subtitle tracks inside video `index` once (needs ffprobe).
    async fn probe_item(&self, epoch: u64, index: usize) {
        let video = {
            let mut items = self.shared.items.lock().unwrap();
            match items.get_mut(index) {
                Some(it) if !it.probed => {
                    it.probed = true;
                    it.media.path.clone()
                }
                _ => return,
            }
        };
        let configured = PathBuf::from(&self.shared.settings.lock().unwrap().ffmpeg_path);
        let Some(ffprobe) = audio::find_ffprobe(Some(&configured)) else { return };
        let langs = tokio::task::spawn_blocking(move || probe::embedded_languages(&ffprobe, &video)).await;
        let langs = match langs {
            Ok(Ok(langs)) => langs,
            Ok(Err(e)) => {
                log::debug!("ffprobe: {e}");
                return;
            }
            Err(_) => return,
        };
        if langs.is_empty() {
            return;
        }
        {
            let mut items = self.shared.items.lock().unwrap();
            if self.generation() != epoch {
                return;
            }
            if let Some(it) = items.get_mut(index) {
                it.media.embedded = langs;
            }
        }
        self.push_row(epoch, index);
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
        let busy = self.ui.upgrade().is_some_and(|ui| ui.global::<AppState>().get_busy());
        if (searched && !force) || busy {
            return;
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<AppState>().set_candidates_loading(true);
            ui.global::<AppState>().set_candidates(ModelRc::default());
        }
        let c = self.clone();
        self.run_busy(async move {
            if let Err(e) = c.search_item(epoch, index, force).await {
                c.status(ST_ERROR, 0, 0, e.to_string());
            }
        });
    }

    fn download_candidate(&self, candidate: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        let (Ok(index), Ok(candidate)) =
            (usize::try_from(ui.global::<AppState>().get_selected_file()), usize::try_from(candidate))
        else {
            return;
        };
        let epoch = self.generation();
        let c = self.clone();
        self.run_busy(async move {
            match c.fetch_item(epoch, index, Some(candidate)).await {
                Ok(Some(saved)) => {
                    let name = saved.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    c.status(ST_SAVED_ONE, saved.remaining.map_or(-1, |r| r as i32), 0, name);
                    if c.auto_sync() {
                        let _ = c.sync_item(epoch, index, Reference::Audio).await;
                    }
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
        let (mut saved, mut missing, mut with_subs, mut searched) = (0u32, 0u32, 0u32, 0u32);
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

            match self.process_item(epoch, i, download, &languages, skip_existing).await {
                Ok(done) => {
                    searched += u32::from(done.searched);
                    with_subs += u32::from(done.found);
                    saved += u32::from(done.saved);
                    missing += u32::from(download && !done.saved && !done.skipped);
                }
                Err(Error::Cancelled) => {
                    self.status(ST_STOPPED, 0, 0, "");
                    return;
                }
                Err(e) => return self.stop_with(e),
            }
        }
        self.progress(1.0);
        if download {
            self.status(ST_FINISHED, saved as i32, missing as i32, "");
        } else {
            self.status(ST_SEARCH_FINISHED, with_subs as i32, searched as i32, "");
        }
    }

    /// One video of a batch run or the folder watch: skip it if it has the first language,
    /// search if needed, and (if `download`) save the best subtitle and sync it. Errors are the
    /// ones that stop a run: fatal provider errors and Stop.
    async fn process_item(
        &self,
        epoch: u64,
        i: usize,
        download: bool,
        languages: &[String],
        skip_existing: bool,
    ) -> Result<ItemDone, Error> {
        let mut done = ItemDone::default();
        self.probe_item(epoch, i).await;
        let (has_first, was_searched) = {
            let items = self.shared.items.lock().unwrap();
            let Some(it) = items.get(i) else { return Ok(done) };
            (it.media.has_language(&languages[0]), it.searched)
        };
        if skip_existing && has_first {
            self.set_state(epoch, i, HAS_SUBTITLE, "");
            done.skipped = true;
            return Ok(done);
        }
        if !was_searched {
            done.searched = true;
            done.found = self.search_item(epoch, i, false).await? > 0;
            tokio::time::sleep(BATCH_PAUSE).await;
        } else {
            done.found = self.shared.items.lock().unwrap().get(i).is_some_and(|it| !it.candidates.is_empty());
        }
        if download {
            match self.fetch_item(epoch, i, None).await {
                Ok(Some(_)) => {
                    done.saved = true;
                    if self.auto_sync()
                        && let Err(Error::Cancelled) = self.sync_item(epoch, i, Reference::Audio).await
                    {
                        return Err(Error::Cancelled);
                    }
                }
                Ok(None) => {
                    let generate = self.shared.settings.lock().unwrap().generate_when_missing;
                    if generate && self.whisper_model().is_some() {
                        match self.generate_item(epoch, i).await {
                            Ok(_) => done.saved = true,
                            Err(Error::Cancelled) => return Err(Error::Cancelled),
                            Err(_) => {}
                        }
                    }
                }
                Err(e) if is_fatal(&e) => return Err(e),
                Err(_) => {}
            }
            tokio::time::sleep(BATCH_PAUSE).await;
        }
        Ok(done)
    }

    /// The chosen Whisper model's file, if it is downloaded.
    fn whisper_model(&self) -> Option<PathBuf> {
        let id = self.shared.settings.lock().unwrap().whisper_model.clone();
        speech::model(&id).and_then(|m| m.installed())
    }

    /// Writes a subtitle for video `index` from its audio with Whisper and saves it next to it.
    async fn generate_item(&self, epoch: u64, index: usize) -> Result<PathBuf, Error> {
        let Some(model) = self.whisper_model() else {
            self.status(ST_NO_MODEL, 0, 0, "");
            return Err(Error::NotConfigured { provider: "Whisper", message: "no speech model".into() });
        };
        let Some(ffmpeg) = self.ffmpeg() else {
            self.status(ST_NO_FFMPEG, 0, 0, "");
            return Err(Error::NoFfmpeg);
        };
        let Some(video) = self.shared.items.lock().unwrap().get(index).map(|it| it.media.path.clone()) else {
            return Err(Error::Parse("no such video".into()));
        };
        let target = self.languages().swap_remove(0);
        self.set_state(epoch, index, GENERATING, "");
        self.status(ST_LISTENING, 0, 0, "");
        let progress: Arc<dyn Fn(f32) + Send + Sync> = {
            let c = self.clone();
            let last = Arc::new(std::sync::atomic::AtomicI32::new(-1));
            Arc::new(move |p: f32| {
                let pct = (p * 100.0) as i32;
                if last.swap(pct, Ordering::Relaxed) != pct {
                    c.status(ST_LISTENING, pct, 0, "");
                    c.progress(p);
                }
            })
        };
        let cancel = self.shared.cancel_flag();
        let path = video.clone();
        let result = tokio::task::spawn_blocking(move || {
            let transcript = speech::transcribe(&model, &ffmpeg, &path, Some(&target), cancel, progress)?;
            output::write_subtitle(&path, &transcript.language, "srt", &transcript.to_srt())
        })
        .await
        .map_err(|e| Error::Parse(e.to_string()))?;
        match result {
            Ok(saved) => {
                {
                    let mut items = self.shared.items.lock().unwrap();
                    if self.generation() == epoch
                        && let Some(it) = items.get_mut(index)
                    {
                        it.media.existing = media::existing_subtitles(&video);
                        it.subtitle = Some(saved.clone());
                    }
                }
                self.set_state(epoch, index, GENERATED, file_name(&saved));
                self.status(ST_GENERATED, 0, 0, file_name(&saved));
                Ok(saved)
            }
            Err(e) => {
                let stopped = matches!(e, Error::Cancelled);
                self.set_state(epoch, index, FAILED, e.to_string());
                self.status(if stopped { ST_STOPPED } else { ST_ERROR }, 0, 0, e.to_string());
                Err(e)
            }
        }
    }

    /// Downloads the Whisper model chosen in the settings.
    fn download_model(&self) {
        let id = self.shared.settings.lock().unwrap().whisper_model.clone();
        let (Some(model), Some(dir)) = (speech::model(&id), speech::models_dir()) else { return };
        let c = self.clone();
        self.run_busy(async move {
            let cancel = c.shared.cancel_flag();
            let mut last = -1;
            let reporter = c.clone();
            let mut progress = move |done: u64, total: Option<u64>| {
                let pct = total.filter(|t| *t > 0).map_or(0, |t| (done * 100 / t) as i32);
                if pct != last {
                    last = pct;
                    reporter.status(ST_MODEL_DOWNLOAD, pct, 0, "");
                    reporter.progress(pct as f32 / 100.0);
                }
            };
            match model.download(&dir, &cancel, &mut progress).await {
                Ok(_) => c.status(ST_MODEL_READY, 0, 0, model.id),
                Err(Error::Cancelled) => c.status(ST_STOPPED, 0, 0, ""),
                Err(e) => c.status(ST_ERROR, 0, 0, e.to_string()),
            }
            let installed = model.installed().is_some();
            let _ = c.ui.upgrade_in_event_loop(move |ui| ui.global::<AppState>().set_whisper_installed(installed));
        });
    }

    /// Starts or stops watching the open folder, as the setting says.
    fn update_watch(&self) {
        let (on, recursive) = {
            let s = self.shared.settings.lock().unwrap();
            (s.watch, s.recursive)
        };
        let root = self.shared.root.lock().unwrap().clone();
        let single_files = self.shared.opened_files.lock().unwrap().is_some();
        let mut slot = self.shared.watcher.lock().unwrap();
        *slot = None;
        let root = root.filter(|_| !single_files);
        let (true, Some(root)) = (on, root) else { return };
        let epoch = self.generation();
        let tx = self.shared.watch_tx.clone();
        match watch::watch(&root, recursive, WATCH_SETTLE, move |path| {
            let _ = tx.send((epoch, path));
        }) {
            Ok(handle) => *slot = Some(handle),
            Err(e) => self.status(ST_ERROR, 0, 0, e.to_string()),
        }
    }

    /// Handles videos the folder watch reports, one at a time.
    async fn watch_worker(self, mut rx: tokio::sync::mpsc::UnboundedReceiver<(u64, PathBuf)>) {
        while let Some((epoch, path)) = rx.recv().await {
            if self.generation() != epoch {
                continue;
            }
            let Some(index) = self.add_video(epoch, &path) else { continue };
            self.status(ST_WATCH_NEW, 0, 0, file_name(&path));
            let (languages, skip_existing) = {
                let s = self.shared.settings.lock().unwrap();
                (s.language_codes(), s.skip_existing)
            };
            if let Err(e) = self.process_item(epoch, index, true, &languages, skip_existing).await {
                self.stop_with(e);
            }
        }
    }

    /// Adds `path` to the list (or finds it there) and returns its index.
    fn add_video(&self, epoch: u64, path: &Path) -> Option<usize> {
        let root = self.shared.root.lock().unwrap().clone().unwrap_or_default();
        let first = self.languages().swap_remove(0);
        let (index, row) = {
            let mut items = self.shared.items.lock().unwrap();
            if self.generation() != epoch {
                return None;
            }
            if let Some(i) = items.iter().position(|it| it.media.path == path) {
                return Some(i);
            }
            let media = media::media_file(path)?;
            items.push(Item::new(media));
            let index = items.len() - 1;
            (index, file_row(&root, &items[index], &first))
        };
        let shared = self.shared.clone();
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            if shared.generation.load(Ordering::SeqCst) != epoch {
                return;
            }
            let files = ui.global::<AppState>().get_files();
            match files.as_any().downcast_ref::<VecModel<FileRow>>() {
                Some(model) => model.push(row),
                None => {
                    let mut rows: Vec<FileRow> = files.iter().collect();
                    rows.push(row);
                    ui.global::<AppState>().set_files(ModelRc::new(VecModel::from(rows)));
                }
            }
        });
        Some(index)
    }

    fn stop_with(&self, e: Error) {
        match e {
            Error::Quota { message, .. } => self.status(ST_QUOTA, 0, 0, message),
            e => self.status(ST_ERROR, 0, 0, e.to_string()),
        }
    }

    /// Searches every provider for file `index`. Returns how many candidates were found, or
    /// the error when nothing was found and the reason is one a batch cannot continue past.
    /// `fresh` skips cached results ("Search again").
    async fn search_item(&self, epoch: u64, index: usize, fresh: bool) -> Result<usize, Error> {
        let languages = self.languages();
        let Some(media) = self.shared.items.lock().unwrap().get(index).map(|it| it.media.clone()) else {
            return Ok(0);
        };
        self.set_state(epoch, index, SEARCHING, "");
        let mut query = tokio::task::spawn_blocking(move || Engine::query_for(&media, &languages))
            .await
            .map_err(|e| Error::Parse(e.to_string()))?;
        if let Some(text) = self.shared.items.lock().unwrap().get(index).and_then(|it| it.search_as.clone()) {
            query.name = name::parse(&text);
        }
        let outcome = self.engine().search(&query, fresh).await;
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
                        items[index].subtitle = Some(saved.path.clone());
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

    fn auto_sync(&self) -> bool {
        self.shared.settings.lock().unwrap().auto_sync
    }

    fn ffmpeg(&self) -> Option<PathBuf> {
        let configured = PathBuf::from(&self.shared.settings.lock().unwrap().ffmpeg_path);
        audio::find_ffmpeg(Some(&configured))
    }

    /// "Sync to audio" / "Sync to a subtitle…" for file `index`.
    fn start_sync(&self, index: i32, to_subtitle: bool) {
        let Ok(index) = usize::try_from(index) else { return };
        let reference = if to_subtitle {
            let dir = self
                .shared
                .items
                .lock()
                .unwrap()
                .get(index)
                .and_then(|it| it.media.path.parent().map(Path::to_path_buf));
            match self.dialog(dir).add_filter("Subtitles", media::SUBTITLE_EXTS).pick_file() {
                Some(file) => Reference::Subtitle(file),
                None => return,
            }
        } else {
            Reference::Audio
        };
        let epoch = self.generation();
        let c = self.clone();
        self.run_busy(async move {
            let _ = c.sync_item(epoch, index, reference).await;
        });
    }

    /// Moves the subtitle of file `index` by `ms`.
    fn shift(&self, index: i32, ms: i32) {
        let Ok(index) = usize::try_from(index) else { return };
        let Some(target) = self.target_subtitle(index) else {
            self.status(ST_NO_SUBTITLE, 0, 0, "");
            return;
        };
        let c = self.clone();
        self.run_busy(async move {
            let path = target.clone();
            match tokio::task::spawn_blocking(move || sync::shift_file(&path, ms as i64)).await {
                Ok(Ok(())) => c.status(ST_SHIFTED, ms, 0, file_name(&target)),
                Ok(Err(e)) => c.status(ST_ERROR, 0, 0, e.to_string()),
                Err(e) => c.status(ST_ERROR, 0, 0, e.to_string()),
            }
        });
    }

    /// The subtitle to sync or shift: the one saved this session, else an existing one in the
    /// most wanted language, else any existing one.
    fn target_subtitle(&self, index: usize) -> Option<PathBuf> {
        let languages = self.languages();
        let items = self.shared.items.lock().unwrap();
        let it = items.get(index)?;
        if let Some(p) = it.subtitle.as_ref().filter(|p| p.is_file()) {
            return Some(p.clone());
        }
        let by_lang = languages.iter().find_map(|l| it.media.existing.iter().find(|s| s.language == Some(l.as_str())));
        by_lang.or(it.media.existing.first()).map(|s| s.path.clone())
    }

    async fn sync_item(&self, epoch: u64, index: usize, reference: Reference) -> Result<Report, Error> {
        let Some(target) = self.target_subtitle(index) else {
            self.status(ST_NO_SUBTITLE, 0, 0, "");
            return Err(Error::Parse("no subtitle".into()));
        };
        let Some(video) = self.shared.items.lock().unwrap().get(index).map(|it| it.media.path.clone()) else {
            return Err(Error::Parse("no such video".into()));
        };
        let what = if matches!(reference, Reference::Audio) { "audio" } else { "subtitle" };
        log::info!("sync {} to {what}", target.display());
        self.set_state(epoch, index, SYNCING, "");
        let result = async {
            let spans = match reference {
                Reference::Audio => self.speech(&video).await?,
                Reference::Subtitle(path) => Arc::new(
                    tokio::task::spawn_blocking(move || sync::reference_from_file(&path))
                        .await
                        .map_err(|e| Error::Parse(e.to_string()))??,
                ),
            };
            let path = target.clone();
            tokio::task::spawn_blocking(move || sync::sync_file(&path, &spans))
                .await
                .map_err(|e| Error::Parse(e.to_string()))?
        }
        .await;
        let pct = |v: f32| (v * 100.0).round() as i32;
        log::info!("sync result: {result:?}");
        match &result {
            Ok(r) if r.applied => {
                self.set_state(epoch, index, SYNCED, r.summary());
                self.status(ST_SYNCED, pct(r.overlap_before), pct(r.overlap_after), r.summary());
            }
            Ok(r) => {
                self.set_state(epoch, index, TIMING_OK, "");
                self.status(ST_TIMING_OK, pct(r.overlap_before), 0, "");
            }
            Err(Error::NoFfmpeg) => {
                // Not the subtitle's fault: it stays saved, the status bar says what is missing.
                self.set_state(epoch, index, SAVED, file_name(&target));
                self.status(ST_NO_FFMPEG, 0, 0, "");
            }
            Err(Error::Cancelled) => {
                self.set_state(epoch, index, SAVED, file_name(&target));
                self.status(ST_STOPPED, 0, 0, "");
            }
            Err(e) => {
                self.set_state(epoch, index, SYNC_FAILED, e.to_string());
                self.status(ST_ERROR, 0, 0, e.to_string());
            }
        }
        result
    }

    /// Speech spans of `video`, from the cache or decoded with ffmpeg.
    async fn speech(&self, video: &Path) -> Result<Arc<Vec<Span>>, Error> {
        if let Some(spans) = self.shared.speech.lock().unwrap().get(video) {
            return Ok(spans.clone());
        }
        let ffmpeg = self.ffmpeg().ok_or(Error::NoFfmpeg)?;
        let (c, path) = (self.clone(), video.to_path_buf());
        self.status(ST_AUDIO, 0, 0, "");
        let spans = tokio::task::spawn_blocking(move || {
            let mut last = -1;
            audio::extract_speech(&ffmpeg, &path, &c.shared.cancel, &mut |p| {
                let pct = (p * 100.0) as i32;
                if pct != last {
                    last = pct;
                    c.status(ST_AUDIO, pct, 0, "");
                }
            })
        })
        .await
        .map_err(|e| Error::Parse(e.to_string()))??;
        let spans = Arc::new(spans);
        self.shared.speech.lock().unwrap().insert(video.to_path_buf(), spans.clone());
        Ok(spans)
    }

    fn set_state(&self, epoch: u64, index: usize, state: i32, detail: impl Into<String>) {
        {
            let mut items = self.shared.items.lock().unwrap();
            if self.generation() != epoch {
                return;
            }
            let Some(it) = items.get_mut(index) else { return };
            it.state = state;
            it.detail = detail.into();
        }
        self.push_row(epoch, index);
    }

    /// Sends the current state of file `index` to its row in the window.
    fn push_row(&self, epoch: u64, index: usize) {
        let root = self.shared.root.lock().unwrap().clone().unwrap_or_default();
        let first = self.languages().swap_remove(0);
        let row = {
            let items = self.shared.items.lock().unwrap();
            if self.generation() != epoch {
                return;
            }
            let Some(it) = items.get(index) else { return };
            file_row(&root, it, &first)
        };
        let shared = self.shared.clone();
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            let files = ui.global::<AppState>().get_files();
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
            if shared.generation.load(Ordering::SeqCst) == epoch
                && ui.global::<AppState>().get_selected_file() == index as i32
            {
                ui.global::<AppState>().set_candidates(ModelRc::new(VecModel::from(rows)));
                ui.global::<AppState>().set_selected_candidate(-1);
                ui.global::<AppState>().set_candidates_loading(false);
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
            if let Some(book) = submagician_core::applog::global() {
                book.set_daily(s.detailed_logs);
            }
            (build_engine(&s), before != codes, codes.join(", "))
        };
        *self.shared.engine.lock().unwrap() = engine;
        ui.global::<AppState>().set_ffmpeg_found(ffmpeg_text(&self.shared.settings.lock().unwrap()).into());
        if languages_changed {
            log::info!("languages: {normalized}");
            // Old results were for other languages.
            let mut items = self.shared.items.lock().unwrap();
            for it in items.iter_mut() {
                it.searched = false;
                it.candidates.clear();
            }
            ui.global::<AppState>().set_candidates(ModelRc::default());
        }
    }
}

fn rt_spawn_watch_worker(c: &Controller, rx: tokio::sync::mpsc::UnboundedReceiver<(u64, PathBuf)>) {
    let worker = c.clone();
    c.rt.spawn(worker.watch_worker(rx));
}

fn apply_settings(ui: &AppWindow, s: &Settings) {
    ui.global::<AppState>().set_watch(s.watch);
    let models: Vec<slint::SharedString> =
        speech::MODELS.iter().map(|m| format!("{} ({} MB)", m.id, m.size_mb).into()).collect();
    ui.global::<AppState>().set_whisper_models(ModelRc::new(VecModel::from(models)));
    let current = speech::MODELS.iter().position(|m| m.id == s.whisper_model).unwrap_or(1);
    ui.global::<AppState>().set_whisper_model_index(current as i32);
    ui.global::<AppState>().set_whisper_installed(speech::MODELS[current].installed().is_some());
    ui.global::<AppState>().set_generate_missing(s.generate_when_missing);
    ui.global::<AppState>().set_languages(s.language_codes().join(", ").into());
    ui.global::<AppState>().set_recursive(s.recursive);
    ui.global::<AppState>().set_skip_existing(s.skip_existing);
    ui.global::<AppState>().set_os_username(s.opensubtitles_username.clone().into());
    ui.global::<AppState>().set_os_password(s.opensubtitles_password.clone().into());
    ui.global::<AppState>().set_os_api_key(s.opensubtitles_api_key.clone().into());
    ui.global::<AppState>().set_auto_sync(s.auto_sync);
    ui.global::<AppState>().set_ffmpeg_path(s.ffmpeg_path.clone().into());
    ui.global::<AppState>().set_use_opensubtitles(s.use_opensubtitles);
    ui.global::<AppState>().set_use_subdl(s.use_subdl);
    ui.global::<AppState>().set_use_addic7ed(s.use_addic7ed);
    ui.global::<AppState>().set_subdl_api_key(s.subdl_api_key.clone().into());
    ui.global::<AppState>().set_check_updates(s.check_updates);
    ui.global::<AppState>().set_detailed_logs(s.detailed_logs);
}

fn read_settings(ui: &AppWindow, s: &mut Settings) {
    s.languages = ui.global::<AppState>().get_languages().into();
    s.recursive = ui.global::<AppState>().get_recursive();
    s.skip_existing = ui.global::<AppState>().get_skip_existing();
    s.opensubtitles_username = ui.global::<AppState>().get_os_username().trim().into();
    s.opensubtitles_password = ui.global::<AppState>().get_os_password().into();
    s.opensubtitles_api_key = ui.global::<AppState>().get_os_api_key().trim().into();
    s.auto_sync = ui.global::<AppState>().get_auto_sync();
    s.ffmpeg_path = ui.global::<AppState>().get_ffmpeg_path().trim().into();
    s.use_opensubtitles = ui.global::<AppState>().get_use_opensubtitles();
    s.use_subdl = ui.global::<AppState>().get_use_subdl();
    s.use_addic7ed = ui.global::<AppState>().get_use_addic7ed();
    s.subdl_api_key = ui.global::<AppState>().get_subdl_api_key().trim().into();
    s.generate_when_missing = ui.global::<AppState>().get_generate_missing();
    s.check_updates = ui.global::<AppState>().get_check_updates();
    s.detailed_logs = ui.global::<AppState>().get_detailed_logs();
}

/// Where ffmpeg was found, or "" when it was not.
fn ffmpeg_text(s: &Settings) -> String {
    let configured = PathBuf::from(&s.ffmpeg_path);
    audio::find_ffmpeg(Some(&configured)).map(|p| p.display().to_string()).unwrap_or_default()
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn file_row(root: &Path, it: &Item, first_language: &str) -> FileRow {
    let folder =
        it.media.path.parent().map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string()).unwrap_or_default();
    let mut langs: Vec<&str> = it.media.existing.iter().map(|s| s.language.unwrap_or("?")).collect();
    langs.dedup();
    FileRow {
        name: it.media.file_name().into(),
        folder: folder.into(),
        existing: langs.join(", ").into(),
        embedded: it.media.embedded.join(", ").into(),
        has_wanted: it.media.has_language(first_language),
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
        provider: c.provider.clone().into(),
        downloads: c.downloads.min(i32::MAX as u64) as i32,
        hash_match: c.hash_match,
        trusted: c.trusted,
        machine: c.machine_translated,
        hearing_impaired: c.hearing_impaired,
        fps: fps.into(),
    }
}
