//! Watching a folder for new videos. A video counts as ready once its size has not changed for
//! a while, so a file that is still being copied or downloaded is not touched.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher};

use crate::media::is_video_ext;
use crate::{Error, Result};

/// Stops watching when dropped.
pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
    stop: Arc<AtomicBool>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Calls `on_ready` (from a background thread) for each video that appears or changes under
/// `dir` and then keeps the same size for `settle`.
pub fn watch(
    dir: &Path,
    recursive: bool,
    settle: Duration,
    on_ready: impl Fn(PathBuf) + Send + 'static,
) -> Result<WatchHandle> {
    let pending: Arc<Mutex<HashMap<PathBuf, (u64, Instant)>>> = Arc::default();
    let mut watcher = {
        let pending = pending.clone();
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                return;
            }
            let mut pending = pending.lock().unwrap();
            for path in event.paths {
                let is_video = path.extension().is_some_and(|e| is_video_ext(&e.to_string_lossy()));
                if is_video {
                    let size = path.metadata().map(|m| m.len()).unwrap_or(0);
                    pending.insert(path, (size, Instant::now()));
                }
            }
        })
        .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?
    };
    let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
    watcher.watch(dir, mode).map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;

    let stop = Arc::new(AtomicBool::new(false));
    let tick = (settle / 4).clamp(Duration::from_millis(50), Duration::from_secs(2));
    {
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(tick);
                let ready = take_settled(&pending, settle);
                for path in ready {
                    if !stop.load(Ordering::SeqCst) {
                        on_ready(path);
                    }
                }
            }
        });
    }
    Ok(WatchHandle { _watcher: watcher, stop })
}

/// Paths whose size has stayed the same for `settle`; a changed size restarts their clock.
fn take_settled(pending: &Mutex<HashMap<PathBuf, (u64, Instant)>>, settle: Duration) -> Vec<PathBuf> {
    let mut pending = pending.lock().unwrap();
    let mut ready = Vec::new();
    pending.retain(|path, (size, since)| {
        let Ok(meta) = path.metadata() else { return false }; // gone (moved or deleted)
        if meta.len() != *size {
            *size = meta.len();
            *since = Instant::now();
            return true;
        }
        if since.elapsed() >= settle && meta.len() > 0 {
            ready.push(path.clone());
            return false;
        }
        true
    });
    ready.sort();
    ready
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::mpsc;

    #[test]
    fn reports_videos_once_they_stop_growing() {
        let dir = std::env::temp_dir().join(format!("submagician-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let (tx, rx) = mpsc::channel();
        let settle = Duration::from_millis(600);
        let handle = watch(&dir, true, settle, move |p| tx.send(p).unwrap()).unwrap();

        // A "download" that keeps growing for longer than the settle time.
        let video = dir.join("sub/New.Film.2024.mkv");
        let mut f = std::fs::File::create(&video).unwrap();
        let mut last_write = Instant::now();
        for _ in 0..8 {
            f.write_all(&[0u8; 4096]).unwrap();
            f.flush().unwrap();
            last_write = Instant::now();
            std::thread::sleep(Duration::from_millis(150));
        }
        drop(f);
        std::fs::write(dir.join("notes.txt"), "not a video").unwrap();

        let got = rx.recv_timeout(Duration::from_secs(10)).expect("video reported");
        assert_eq!(got, video);
        assert!(last_write.elapsed() >= settle, "reported before it stopped growing");
        assert!(rx.recv_timeout(Duration::from_millis(1500)).is_err(), "reported once, text files ignored");

        drop(handle);
        std::fs::write(dir.join("Later.mkv"), [1u8; 10]).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(1500)).is_err(), "stopped after drop");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
