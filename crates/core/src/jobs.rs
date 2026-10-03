//! The heavy jobs (syncing to the audio, taking a subtitle out of a video, writing one from the
//! audio), and running them in a worker process: `submagician-cli --worker <job as JSON>`.
//!
//! In a worker a crash (whisper.cpp aborting, a decoder bug) ends only that process, never the
//! window, and Stop simply ends it. The worker prints one line per event:
//! - `progress<TAB>0.42`
//! - `done<TAB>{json Done}` or `failed<TAB>{json WireError}` at the end.
//!
//! Without the command-line tool next to the app the same code runs in the app's own process.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::autosync::{self, Method};
use crate::sync::Report;
use crate::{Error, Result, output, probe};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "job", rename_all = "kebab-case")]
pub enum Job {
    /// Sync `subtitle` to the audio of `video`.
    SyncAudio { subtitle: PathBuf, video: PathBuf, ffmpeg: PathBuf },
    /// Take the best text track in `languages` out of `video`, save it next to the video and
    /// sync it to the audio.
    Embedded { video: PathBuf, ffmpeg: PathBuf, ffprobe: PathBuf, languages: Vec<String> },
    /// Write a subtitle from the audio with the Whisper `model` (`language` wanted).
    Generate { video: PathBuf, ffmpeg: PathBuf, model: PathBuf, language: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum Done {
    Synced {
        report: Report,
        method: Method,
    },
    /// The track was saved; `report` is `None` when the sync failed (`sync_error` says why).
    Extracted {
        path: PathBuf,
        language: String,
        report: Option<Report>,
        method: Option<Method>,
        sync_error: Option<String>,
    },
    Written {
        path: PathBuf,
        language: String,
    },
}

/// An error as it travels from the worker.
#[derive(Debug, Serialize, Deserialize)]
pub struct WireError {
    kind: String,
    message: String,
}

impl From<&Error> for WireError {
    fn from(e: &Error) -> Self {
        let kind = match e {
            Error::NoFfmpeg => "no-ffmpeg",
            Error::NoSpeech => "no-speech",
            Error::Cancelled => "cancelled",
            _ => "other",
        };
        WireError { kind: kind.into(), message: e.to_string() }
    }
}

impl From<WireError> for Error {
    fn from(w: WireError) -> Self {
        match w.kind.as_str() {
            "no-ffmpeg" => Error::NoFfmpeg,
            "no-speech" => Error::NoSpeech,
            "cancelled" => Error::Cancelled,
            _ => Error::Other(w.message),
        }
    }
}

pub type Progress = Arc<dyn Fn(f32) + Send + Sync>;

/// Runs `job` in this process.
pub fn run(job: &Job, cancel: Arc<AtomicBool>, progress: Progress) -> Result<Done> {
    match job {
        Job::SyncAudio { subtitle, video, ffmpeg } => {
            let (report, method) = autosync::sync_to_audio(subtitle, video, ffmpeg, &cancel, &mut |p| progress(p))?;
            Ok(Done::Synced { report, method })
        }
        Job::Embedded { video, ffmpeg, ffprobe, languages } => {
            let tracks = probe::subtitle_tracks(ffprobe, video)?;
            let track = probe::pick(&tracks, languages).ok_or_else(|| {
                let pictures =
                    tracks.iter().any(|t| !t.is_text() && t.language.is_some_and(|l| languages.iter().any(|w| w == l)));
                Error::Other(if pictures {
                    "the track inside is pictures (Blu-ray/DVD), which cannot be synced as text".into()
                } else {
                    format!("no {} text track inside", languages.join(", "))
                })
            })?;
            let language = track.language.unwrap_or("und").to_owned();
            progress(0.02);
            let text = probe::extract(ffmpeg, video, track)?;
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let path = output::write_subtitle(video, &language, track.extension(), &text)?;
            progress(0.2);
            let synced = autosync::sync_to_audio(&path, video, ffmpeg, &cancel, &mut |p| progress(0.2 + p * 0.8));
            Ok(match synced {
                Ok((report, method)) => {
                    Done::Extracted { path, language, report: Some(report), method: Some(method), sync_error: None }
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => {
                    Done::Extracted { path, language, report: None, method: None, sync_error: Some(e.to_string()) }
                }
            })
        }
        #[cfg(feature = "whisper")]
        Job::Generate { video, ffmpeg, model, language } => {
            let t = crate::speech::transcribe(model, ffmpeg, video, Some(language), cancel, progress)?;
            let path = output::write_subtitle(video, &t.language, "srt", &t.to_srt())?;
            Ok(Done::Written { path, language: t.language })
        }
        #[cfg(not(feature = "whisper"))]
        Job::Generate { .. } => Err(Error::Other("this build has no speech recognition".into())),
    }
}

/// The worker side: runs the job given as JSON and prints the events.
pub fn serve(json: &str) -> bool {
    let job: Job = match serde_json::from_str(json) {
        Ok(job) => job,
        Err(e) => {
            print_failed(&Error::Parse(format!("bad job: {e}")));
            return false;
        }
    };
    let last = Arc::new(std::sync::atomic::AtomicI32::new(-1));
    let progress: Progress = Arc::new(move |p| {
        // Thousandths are plenty, and keep the pipe quiet.
        let step = (p.clamp(0.0, 1.0) * 1000.0) as i32;
        if last.swap(step, Ordering::Relaxed) != step {
            println!("progress\t{:.3}", step as f32 / 1000.0);
        }
    });
    match run(&job, Arc::new(AtomicBool::new(false)), progress) {
        Ok(done) => {
            println!("done\t{}", serde_json::to_string(&done).expect("serialize"));
            true
        }
        Err(e) => {
            print_failed(&e);
            false
        }
    }
}

fn print_failed(e: &Error) {
    println!("failed\t{}", serde_json::to_string(&WireError::from(e)).expect("serialize"));
}

enum Event {
    Progress(f32),
    Done(Result<Done>),
}

fn parse_line(line: &str) -> Option<Event> {
    let (kind, value) = line.split_once('\t')?;
    match kind {
        "progress" => value.trim().parse().ok().map(Event::Progress),
        "done" => Some(Event::Done(serde_json::from_str(value).map_err(|e| Error::Parse(format!("worker: {e}"))))),
        "failed" => Some(Event::Done(Err(serde_json::from_str::<WireError>(value)
            .map(Error::from)
            .unwrap_or_else(|e| Error::Parse(format!("worker: {e}")))))),
        _ => None,
    }
}

/// Runs `job` in a worker process started with `command` (the command-line tool, see
/// [`crate::players::cli_command`]). Stop (`cancel`) ends the worker.
pub fn run_in_worker(command: &[String], job: &Job, cancel: Arc<AtomicBool>, progress: Progress) -> Result<Done> {
    let (program, args) = command.split_first().ok_or_else(|| Error::Other("no worker command".into()))?;
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .args(args)
        .arg("--worker")
        .arg(serde_json::to_string(job).expect("serialize"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    // The worker's own log goes into ours; its last lines explain a crash.
    let stderr_thread = std::thread::spawn(move || {
        let mut tail = std::collections::VecDeque::new();
        for line in BufReader::new(stderr).lines().map_while(std::result::Result::ok) {
            log::info!(target: "submagician::worker", "{line}");
            if tail.len() == 5 {
                tail.pop_front();
            }
            tail.push_back(line);
        }
        tail
    });
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(std::result::Result::ok) {
            if let Some(event) = parse_line(&line)
                && tx.send(event).is_err()
            {
                return;
            }
        }
    });
    let mut result = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Progress(p)) => progress(p),
            Ok(Event::Done(r)) => result = Some(r),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait()?;
    let tail = stderr_thread.join().unwrap_or_default();
    match result {
        Some(r) => r,
        None => {
            // The details stay in the log; the message is for people.
            log::error!("the worker stopped unexpectedly ({status}); last lines: {tail:?}");
            Err(crash_error(&status))
        }
    }
}

/// The error for a worker that died without an answer.
fn crash_error(status: &std::process::ExitStatus) -> Error {
    #[cfg(windows)]
    let illegal = status.code().map(|c| c as u32) == Some(0xC000_001D);
    #[cfg(unix)]
    let illegal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal() == Some(4)
    };
    #[cfg(not(any(windows, unix)))]
    let illegal = false;
    Error::Crashed { cpu: illegal, detail: status.to_string() }
}

/// Runs `job` in a worker process when the command-line tool is there, else in this process.
pub fn run_isolated(job: &Job, cancel: Arc<AtomicBool>, progress: Progress) -> Result<Done> {
    match crate::players::cli_command() {
        Some(command) => run_in_worker(&command, job, cancel, progress),
        None => {
            log::info!("no command-line tool next to the app; running the job here");
            run(job, cancel, progress)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_worker_killed_by_an_illegal_instruction_is_a_cpu_crash() {
        let command: Vec<String> = ["sh", "-c", "echo noise >&2; kill -ILL $$", "worker"].map(String::from).into();
        let job = Job::SyncAudio { subtitle: "a.srt".into(), video: "a.mkv".into(), ffmpeg: "ffmpeg".into() };
        let result = run_in_worker(&command, &job, Arc::new(AtomicBool::new(false)), Arc::new(|_| {}));
        match result {
            Err(Error::Crashed { cpu: true, detail }) => assert!(!detail.contains("noise"), "{detail}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn jobs_and_results_travel_as_json() {
        let job = Job::SyncAudio { subtitle: "a b.srt".into(), video: "ç.mkv".into(), ffmpeg: "ffmpeg".into() };
        let json = serde_json::to_string(&job).unwrap();
        assert!(json.starts_with(r#"{"job":"sync-audio""#), "{json}");
        assert!(matches!(serde_json::from_str::<Job>(&json).unwrap(), Job::SyncAudio { .. }));

        assert!(matches!(parse_line("progress\t0.250"), Some(Event::Progress(p)) if p == 0.25));
        let report =
            Report { offset_ms: 1200, ratio: 1.0, splits: 0, overlap_before: 0.2, overlap_after: 0.9, applied: true };
        let done = Done::Synced { report: report.clone(), method: Method::Quick };
        let line = format!("done\t{}", serde_json::to_string(&done).unwrap());
        match parse_line(&line) {
            Some(Event::Done(Ok(Done::Synced { report: r, method: Method::Quick }))) => assert_eq!(r, report),
            _ => panic!("{line}"),
        }
        let line = format!("failed\t{}", serde_json::to_string(&WireError::from(&Error::NoFfmpeg)).unwrap());
        assert!(matches!(parse_line(&line), Some(Event::Done(Err(Error::NoFfmpeg)))));
        let line =
            format!("failed\t{}", serde_json::to_string(&WireError::from(&Error::Ffmpeg("bad".into()))).unwrap());
        assert!(matches!(parse_line(&line), Some(Event::Done(Err(Error::Other(m)))) if m == "ffmpeg failed: bad"));
        assert!(parse_line("message\thello").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_crashing_worker_is_an_error_and_stop_ends_it() {
        let sh = |script: &str| vec!["sh".to_owned(), "-c".to_owned(), script.to_owned(), "worker".to_owned()];
        let job = Job::SyncAudio { subtitle: "s".into(), video: "v".into(), ffmpeg: "f".into() };
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress: Progress = {
            let seen = seen.clone();
            Arc::new(move |p| seen.lock().unwrap().push(p))
        };
        let r = run_in_worker(
            &sh("printf 'progress\\t0.5\\n'; echo boom >&2; exit 3"),
            &job,
            Arc::default(),
            progress.clone(),
        );
        // The worker's last words go to the log, not into the message people see.
        assert!(matches!(&r, Err(Error::Crashed { cpu: false, detail }) if !detail.contains("boom")), "{r:?}");
        assert_eq!(*seen.lock().unwrap(), vec![0.5]);

        let cancel = Arc::new(AtomicBool::new(false));
        let stopper = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            stopper.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let r = run_in_worker(&sh("sleep 30"), &job, cancel, progress);
        assert!(matches!(r, Err(Error::Cancelled)) && started.elapsed() < Duration::from_secs(5), "{r:?}");
    }
}
