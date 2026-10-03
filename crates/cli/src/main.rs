//! `submagician-cli`: the app's search → pick → save → sync, for scripts and file-manager actions.
//! Uses the same settings file as the app (languages, logins, sources, ffmpeg).
//!
//! `--player` is the mode the mpv and VLC plugins use (see `crates/core/src/players.rs`): one
//! video, machine-readable output.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use clap::Parser;

mod player;
use submagician_core::engine::Engine;
use submagician_core::media::{self, MediaFile};
use submagician_core::settings::Settings;
use submagician_core::{Error, audio, autosync, jobs, lang, output, probe, score, speech, sync};

#[derive(Parser, Debug)]
#[command(name = "submagician-cli", version, about = "Find, pick, fix and sync subtitles for videos")]
struct Args {
    /// Video files or folders.
    #[arg(required_unless_present_any = ["download_model", "worker"])]
    paths: Vec<PathBuf>,
    /// Wanted languages, most wanted first, e.g. "tr,en" (default: the app's setting).
    #[arg(short, long)]
    lang: Option<String>,
    /// Sources, comma separated: opensubtitles, subdl, addic7ed (default: the app's setting).
    #[arg(long)]
    sources: Option<String>,
    /// Only the given folders, not their subfolders.
    #[arg(long)]
    no_recursive: bool,
    /// Do not sync the subtitle to the audio after downloading.
    #[arg(long)]
    no_sync: bool,
    /// Also videos that already have a subtitle in the first language.
    #[arg(long)]
    force: bool,
    /// Search and show the best subtitle for each video; download nothing.
    #[arg(long)]
    dry_run: bool,
    /// Ask the sources again instead of using cached search results.
    #[arg(long)]
    fresh: bool,
    /// When no source has a subtitle, write one from the audio with Whisper.
    #[arg(long)]
    generate: bool,
    /// Whisper model for --generate: tiny, base, small, medium, large-v3-turbo (default: the app's).
    #[arg(long)]
    model: Option<String>,
    /// Download a Whisper model and exit.
    #[arg(long, value_name = "MODEL")]
    download_model: Option<String>,
    /// When a video has a text subtitle track inside in a wanted language, take it out as a
    /// file next to the video and sync it, instead of downloading one.
    #[arg(long)]
    from_video: bool,
    /// Internal: run one job for the app (JSON) and print its progress.
    #[arg(long, hide = true, value_name = "JOB")]
    worker: Option<String>,
    /// For player plugins: one video (a path or a file:// URI); prints `subtitle<TAB>path` for
    /// the subtitle to load and `message<TAB>text` for the player to show.
    #[arg(long)]
    player: bool,
    /// With --player: the video just started; do nothing when it already has a subtitle in the
    /// first language (next to it or inside it).
    #[arg(long, requires = "player")]
    auto: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    if let Some(job) = &args.worker {
        // The app shows these lines in its own log.
        // whisper.cpp reports every step of loading a model; only its warnings are worth keeping.
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,whisper_rs=warn")).init();
        return if submagician_core::jobs::serve(job) { ExitCode::SUCCESS } else { ExitCode::from(1) };
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();
    if let Some(id) = &args.download_model {
        return download_model(id);
    }
    let mut settings = Settings::load();
    if let Some(id) = &args.model {
        if speech::model(id).is_none() {
            eprintln!("unknown model {id:?}; models: {}", model_ids());
            return ExitCode::from(2);
        }
        settings.whisper_model = id.clone();
    }
    if let Some(list) = &args.lang {
        settings.languages = list.clone();
    }
    if let Some(sources) = &args.sources
        && let Err(unknown) = apply_sources(&mut settings, sources)
    {
        eprintln!("unknown source: {unknown} (use opensubtitles, subdl, addic7ed)");
        return ExitCode::from(2);
    }
    if lang::parse_list(&settings.languages).is_empty() {
        eprintln!("no known language in {:?}", settings.languages);
        return ExitCode::from(2);
    }
    if args.player {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
        return runtime.block_on(player::run(&args, &settings));
    }
    let videos = collect(&args.paths, !args.no_recursive);
    if videos.is_empty() {
        eprintln!("no videos found");
        return ExitCode::from(1);
    }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
    runtime.block_on(run(&args, &settings, videos))
}

fn download_model(id: &str) -> ExitCode {
    let (Some(model), Some(dir)) = (speech::model(id), speech::models_dir()) else {
        eprintln!("unknown model {id:?}; models: {}", model_ids());
        return ExitCode::from(2);
    };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
    let mut last = u64::MAX;
    let mut progress = |done: u64, total: Option<u64>| {
        let pct = total.filter(|t| *t > 0).map_or(0, |t| done * 100 / t);
        if pct != last && pct.is_multiple_of(10) {
            last = pct;
            eprintln!("{} {pct}%", model.id);
        }
    };
    match runtime.block_on(model.download(&dir, &AtomicBool::new(false), &mut progress)) {
        Ok(path) => {
            println!("{}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("download failed: {e}");
            ExitCode::from(1)
        }
    }
}

fn model_ids() -> String {
    speech::MODELS.iter().map(|m| m.id).collect::<Vec<_>>().join(", ")
}

/// Switches on exactly the named sources.
fn apply_sources(s: &mut Settings, list: &str) -> Result<(), String> {
    s.use_opensubtitles = false;
    s.use_subdl = false;
    s.use_addic7ed = false;
    for name in list.split([',', ' ']).filter(|n| !n.is_empty()) {
        match name.to_ascii_lowercase().as_str() {
            "opensubtitles" | "os" => s.use_opensubtitles = true,
            "subdl" => s.use_subdl = true,
            "addic7ed" | "gestdown" => s.use_addic7ed = true,
            other => return Err(other.to_owned()),
        }
    }
    Ok(())
}

fn collect(paths: &[PathBuf], recursive: bool) -> Vec<MediaFile> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            out.extend(media::scan(p, recursive));
        } else if let Some(video) = media::media_file(p) {
            out.push(video);
        } else {
            eprintln!("skipped {}: not a folder or a video", p.display());
        }
    }
    out
}

#[derive(Default)]
struct Tally {
    saved: u32,
    skipped: u32,
    missing: u32,
    failed: u32,
}

async fn run(args: &Args, settings: &Settings, videos: Vec<MediaFile>) -> ExitCode {
    let languages = settings.language_codes();
    let engine = settings.engine();
    if engine.provider_names().is_empty() {
        eprintln!("no sources switched on");
        return ExitCode::from(2);
    }
    let ffmpeg_setting = PathBuf::from(&settings.ffmpeg_path);
    let ffmpeg = audio::find_ffmpeg(Some(&ffmpeg_setting));
    let ffprobe = audio::find_ffprobe(Some(&ffmpeg_setting));
    let sync_wanted = settings.auto_sync && !args.no_sync && !args.dry_run;
    if sync_wanted && ffmpeg.is_none() {
        eprintln!("note: ffmpeg not found, subtitles will not be synced to the audio");
    }
    let whisper = if args.generate {
        match speech::model(&settings.whisper_model).and_then(|m| m.installed()) {
            Some(path) => Some(path),
            None => {
                eprintln!(
                    "note: Whisper model {:?} is not downloaded (submagician-cli --download-model {}); --generate is off",
                    settings.whisper_model, settings.whisper_model
                );
                None
            }
        }
    } else {
        None
    };
    let mut tally = Tally::default();
    let mut reported = HashSet::new();
    for mut video in videos {
        let name = video.file_name();
        if let Some(ffprobe) = &ffprobe {
            video.embedded = probe::embedded_languages(ffprobe, &video.path).unwrap_or_default();
        }
        if args.from_video
            && !args.dry_run
            && let (Some(ffmpeg), Some(ffprobe)) = (&ffmpeg, &ffprobe)
            && video.embedded.iter().any(|l| languages.iter().any(|w| w == l))
            && !video.existing.iter().any(|s| s.language == Some(languages[0].as_str()))
        {
            let job = jobs::Job::Embedded {
                video: video.path.clone(),
                ffmpeg: ffmpeg.clone(),
                ffprobe: ffprobe.clone(),
                languages: languages.clone(),
            };
            match jobs::run(&job, Default::default(), std::sync::Arc::new(|_| {})) {
                Ok(jobs::Done::Extracted { path, report, sync_error, .. }) => {
                    let note = match (report, sync_error) {
                        (Some(r), _) => report_note(&r),
                        (None, Some(e)) => format!(", not synced: {e}"),
                        _ => String::new(),
                    };
                    println!("[video] {name} -> {} (the track inside{note})", file_name(&path));
                    tally.saved += 1;
                    continue;
                }
                Ok(_) => {}
                Err(e) => eprintln!("{name}: track inside not used: {e}"),
            }
        }
        if !args.force && video.has_language(&languages[0]) {
            println!("[skip]  {name}: already has {}", languages[0]);
            tally.skipped += 1;
            continue;
        }
        let query = Engine::query_for(&video, &languages);
        let outcome = engine.search(&query, args.fresh).await;
        for e in &outcome.errors {
            if reported.insert(e.to_string()) {
                eprintln!("warning: {e}");
            }
        }
        let Some(best) = score::best(&outcome.candidates, &languages) else {
            match (&whisper, &ffmpeg) {
                (Some(model), Some(ffmpeg)) if !args.dry_run => {
                    match generate(model, ffmpeg, &video.path, &languages[0]) {
                        Ok(path) => {
                            println!("[audio] {name} -> {} (written from the audio)", file_name(&path));
                            tally.saved += 1;
                        }
                        Err(e) => {
                            println!("[fail]  {name}: nothing found, and from the audio: {e}");
                            tally.failed += 1;
                        }
                    }
                }
                _ => {
                    println!("[none]  {name}: nothing found");
                    tally.missing += 1;
                }
            }
            continue;
        };
        let c = &outcome.candidates[best];
        if args.dry_run {
            println!("[best]  {name}: {} {} ({}, score {})", c.language, c.release, c.provider, c.score);
            continue;
        }
        match engine.fetch(&video, &query, c).await {
            Ok(saved) => {
                let note = match (&ffmpeg, sync_wanted) {
                    (Some(ffmpeg), true) => sync_note(ffmpeg, &video.path, &saved.path),
                    _ => String::new(),
                };
                println!("[saved] {name} -> {} ({}{note})", file_name(&saved.path), c.provider);
                tally.saved += 1;
            }
            Err(Error::Quota { message, .. }) => {
                eprintln!("download limit reached: {message}");
                tally.failed += 1;
                break;
            }
            Err(e) => {
                println!("[fail]  {name}: {e}");
                tally.failed += 1;
            }
        }
    }
    println!("{} saved, {} skipped, {} not found, {} failed", tally.saved, tally.skipped, tally.missing, tally.failed);
    if tally.failed == 0 { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

/// Writes a subtitle from the audio and saves it next to the video.
pub(crate) fn generate(model: &Path, ffmpeg: &Path, video: &Path, target: &str) -> Result<PathBuf, Error> {
    let progress: std::sync::Arc<dyn Fn(f32) + Send + Sync> = std::sync::Arc::new(|_| {});
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let t = speech::transcribe(model, ffmpeg, video, Some(target), cancel, progress)?;
    output::write_subtitle(video, &t.language, "srt", &t.to_srt())
}

pub(crate) fn sync_note(ffmpeg: &Path, video: &Path, subtitle: &Path) -> String {
    match autosync::sync_to_audio(subtitle, video, ffmpeg, &AtomicBool::new(false), &mut |_| {}) {
        Ok((r, _)) => report_note(&r),
        Err(e) => format!(", not synced: {e}"),
    }
}

fn report_note(r: &sync::Report) -> String {
    if r.applied {
        format!(", synced {}, speech {:.0}% -> {:.0}%", r.summary(), r.overlap_before * 100.0, r.overlap_after * 100.0)
    } else {
        format!(", timing fits ({:.0}% speech)", r.overlap_before * 100.0)
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_arguments() {
        let a = Args::try_parse_from(["submagician-cli", "-l", "tr,en", "--no-sync", "--dry-run", "/films"]).unwrap();
        assert_eq!(a.lang.as_deref(), Some("tr,en"));
        assert!(a.no_sync && a.dry_run && !a.force);
        assert_eq!(a.paths, vec![PathBuf::from("/films")]);
        assert!(Args::try_parse_from(["submagician-cli"]).is_err(), "a path is required");
        let d = Args::try_parse_from(["submagician-cli", "--download-model", "base"]).unwrap();
        assert_eq!(d.download_model.as_deref(), Some("base"));
        let g = Args::try_parse_from(["submagician-cli", "--generate", "--model", "tiny", "x"]).unwrap();
        assert!(g.generate && g.model.as_deref() == Some("tiny"));
        let p = Args::try_parse_from(["submagician-cli", "--player", "--auto", "--", "-odd name.mkv"]).unwrap();
        assert!(p.player && p.auto && p.paths == vec![PathBuf::from("-odd name.mkv")]);
        assert!(Args::try_parse_from(["submagician-cli", "--auto", "x"]).is_err(), "--auto needs --player");
    }

    #[test]
    fn selects_sources() {
        let mut s = Settings::default();
        apply_sources(&mut s, "subdl, Addic7ed").unwrap();
        assert!(!s.use_opensubtitles && s.use_subdl && s.use_addic7ed);
        assert_eq!(apply_sources(&mut s, "podnapisi"), Err("podnapisi".into()));
    }
}
