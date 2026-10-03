//! The app's log.
//!
//! - Every line is kept in memory (the newest [`RECENT_LINES`]) for "Report a problem".
//! - Warnings, errors and panics always go to `errors.log`: the critical events are kept even
//!   while saving logs is off, so a failure can be explained afterwards.
//! - With "Save detailed logs" on, every line also goes to `submagician-YYYY-MM-DD.log`; files
//!   older than [`KEEP_DAYS`] days are deleted.
//!
//! Lines from SubMagician itself are kept from `info` (from `debug` while detailed logs are on);
//! other crates (HTTP, whisper.cpp, …) only from `warn`.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Daily files older than this are deleted.
pub const KEEP_DAYS: u64 = 14;
/// Lines kept in memory.
pub const RECENT_LINES: usize = 500;
/// `errors.log` is moved to `errors.old.log` when it grows past this.
const MAX_ERROR_LOG: u64 = 1024 * 1024;
const DAILY_PREFIX: &str = "submagician-";
pub const ERROR_LOG: &str = "errors.log";

/// `<data>/logs`: `%LOCALAPPDATA%\SubMagician\data\logs`, `~/.local/share/SubMagician/logs`.
pub fn logs_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "SubMagician").map(|d| d.data_local_dir().join("logs"))
}

/// The log behind the `log` macros; also usable on its own (tests).
pub struct Logbook {
    dir: PathBuf,
    state: Mutex<State>,
    echo: bool,
}

struct State {
    daily: bool,
    recent: VecDeque<String>,
}

impl Logbook {
    /// A log writing into `dir`; `echo` also prints every line to stderr.
    pub fn new(dir: PathBuf, daily: bool, echo: bool) -> Logbook {
        let _ = fs::create_dir_all(&dir);
        rotate(&dir.join(ERROR_LOG));
        prune(&dir, chrono::Local::now().date_naive());
        Logbook { dir, state: Mutex::new(State { daily, recent: VecDeque::new() }), echo }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn set_daily(&self, on: bool) {
        self.state.lock().unwrap().daily = on;
    }

    pub fn daily(&self) -> bool {
        self.state.lock().unwrap().daily
    }

    /// The newest lines, oldest first.
    pub fn recent(&self) -> Vec<String> {
        self.state.lock().unwrap().recent.iter().cloned().collect()
    }

    fn wants(&self, level: Level, target: &str) -> bool {
        let ours = target.starts_with("submagician");
        match level {
            Level::Error | Level::Warn => true,
            Level::Info => ours,
            Level::Debug => ours && self.daily(),
            Level::Trace => false,
        }
    }

    /// Records one line.
    pub fn write(&self, level: Level, target: &str, text: &str) {
        let now = chrono::Local::now();
        let source = target.rsplit("::").next().unwrap_or(target);
        let line = format!("{} {level:<5} [{source}] {text}", now.format("%Y-%m-%d %H:%M:%S"));
        if self.echo {
            eprintln!("{line}");
        }
        let daily = {
            let mut state = self.state.lock().unwrap();
            if state.recent.len() == RECENT_LINES {
                state.recent.pop_front();
            }
            state.recent.push_back(line.clone());
            state.daily
        };
        if level <= Level::Warn {
            append(&self.dir.join(ERROR_LOG), &line);
        }
        if daily {
            append(&self.dir.join(format!("{DAILY_PREFIX}{}.log", now.format("%Y-%m-%d"))), &line);
        }
    }
}

impl Log for Logbook {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.wants(metadata.level(), metadata.target())
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            self.write(record.level(), record.target(), &record.args().to_string());
        }
    }

    fn flush(&self) {}
}

static GLOBAL: OnceLock<&'static Logbook> = OnceLock::new();

/// Installs the log for this process (once) and logs panics as errors. Returns `None` when there
/// is no folder for logs or a logger is already installed.
pub fn init(daily: bool) -> Option<&'static Logbook> {
    let dir = logs_dir()?;
    let book: &'static Logbook = Box::leak(Box::new(Logbook::new(dir, daily, cfg!(debug_assertions))));
    log::set_logger(book).ok()?;
    log::set_max_level(LevelFilter::Debug);
    let _ = GLOBAL.set(book);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        log::error!(target: "submagician::panic", "panic in {}: {info}", thread.name().unwrap_or("a thread"));
        previous(info);
    }));
    Some(book)
}

/// The log installed by [`init`].
pub fn global() -> Option<&'static Logbook> {
    GLOBAL.get().copied()
}

fn append(path: &Path, line: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

fn rotate(path: &Path) {
    if fs::metadata(path).is_ok_and(|m| m.len() > MAX_ERROR_LOG) {
        let _ = fs::rename(path, path.with_file_name("errors.old.log"));
    }
}

/// Deletes our own dated files past the retention window; nothing else in the folder.
fn prune(dir: &Path, today: chrono::NaiveDate) {
    let Some(cutoff) = today.checked_sub_days(chrono::Days::new(KEEP_DAYS)) else { return };
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let date = name
            .strip_prefix(DAILY_PREFIX)
            .and_then(|rest| rest.strip_suffix(".log"))
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
        if date.is_some_and(|d| d < cutoff) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("submagician-log-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn errors_always_daily_files_when_on() {
        let dir = temp("files");
        fs::write(dir.join("submagician-2000-01-01.log"), "old").unwrap();
        fs::write(dir.join("notes.txt"), "keep").unwrap();
        let book = Logbook::new(dir.clone(), false, false);
        assert!(!dir.join("submagician-2000-01-01.log").exists(), "old daily file pruned");
        assert!(dir.join("notes.txt").exists(), "other files left alone");

        book.write(Level::Info, "submagician::controller", "scan started");
        book.write(Level::Error, "submagician::controller", "sync failed");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let daily = dir.join(format!("submagician-{today}.log"));
        assert!(!daily.exists(), "no daily file while off");
        let errors = fs::read_to_string(dir.join(ERROR_LOG)).unwrap();
        assert!(errors.trim_end().ends_with("ERROR [controller] sync failed"), "{errors}");
        assert!(!errors.contains("scan started"));

        book.set_daily(true);
        book.write(Level::Info, "submagician_core::sync", "synced");
        assert!(fs::read_to_string(&daily).unwrap().contains("INFO  [sync] synced"));
        assert_eq!(book.recent().len(), 3);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn filters_and_keeps_recent_lines() {
        let dir = temp("filter");
        let book = Logbook::new(dir.clone(), false, false);
        assert!(book.wants(Level::Info, "submagician::controller"));
        assert!(!book.wants(Level::Info, "reqwest::connect"));
        assert!(book.wants(Level::Warn, "reqwest::connect"));
        assert!(!book.wants(Level::Debug, "submagician_core::engine"));
        book.set_daily(true);
        assert!(book.wants(Level::Debug, "submagician_core::engine"));

        for i in 0..RECENT_LINES + 5 {
            book.write(Level::Info, "submagician", &format!("line {i}"));
        }
        let recent = book.recent();
        assert_eq!(recent.len(), RECENT_LINES);
        assert!(
            recent[0].ends_with("line 5") && recent.last().unwrap().ends_with(&format!("line {}", RECENT_LINES + 4))
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
