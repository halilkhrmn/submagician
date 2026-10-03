# Progress

## Phases

- [x] Phase 1 — Foundation: scan, hash, name parse, OpenSubtitles, scoring, UTF-8 fix, save, UI, CI
  - [ ] Live test against OpenSubtitles with a real application key
- [x] Phase 2 — Timing: ffmpeg, VAD + alass sync, frame rate, reference sync, manual offset
  - [ ] Check on real films (long soundtracks, music-heavy scenes) and tune VAD / thresholds
- [x] Phase 3 — More sources: SubDL, Addic7ed, RAR/7z, cache, source switches
  - [x] Live tests in CI with the secrets: OpenSubtitles, SubDL, Addic7ed search + download
  - [ ] Turkish sites: decide on a hidden WebView (Türkçealtyazı and PlanetDP need JavaScript)
- [x] Phase 4 — Convenience: embedded tracks, watch folder, drag & drop, context menu, CLI
  - [ ] Wayland drop tried on a real desktop (starts fine on weston; the drop itself untested)
  - [ ] Windows: Explorer menu entries and drag & drop tried on a real machine
- [x] Phase 5 — Speech: Whisper write-from-audio (app, batch/watch fallback, CLI)
  - [ ] Try on real films with base/small models (speed and quality on a normal PC)
- [x] Phase 6 — Packaging, updates, logs, new interface, player plugins, fast sync
  - [ ] First release run on GitHub (installer and packages built and smoke-tested in CI)
  - [ ] In-app update tried for real (the repository is public now; needs a second release)
  - [ ] VLC extension tried in VLC (mpv script tried end to end; VLC only syntax-checked)
- [ ] Phase 7 — macOS

## Work log

### 2026-10-03 — First release run: AppImage start fix
- Found: the release smoke test started the AppImage on a clean Ubuntu 24.04 container without
  ca-certificates; building the HTTP client failed and the app panicked, so v0.1.0 was not
  published (Windows build, installer smoke test and .deb smoke test passed).
- Done: `core::net` builds every HTTP client, falling back to built-in Mozilla roots.
- Verified: with `SSL_CERT_FILE`/`SSL_CERT_DIR` pointing at nothing, the old CLI panicked and the
  new one logged a warning and searched Addic7ed fine; unit test; full test suite.

### 2026-10-03 — Website and new logo
- Done: `site/` landing page (minimal: logo, one line, a download button that picks the file for
  the visitor's system from the newest release, a screenshot, three features), Pages workflow;
  new logo (a video frame with two subtitle lines) as the app icon too; README links the site;
  "1 subtitle found" singular.
- Verified: rendered in Chromium (light, dark, phone width); download button tested with a mocked
  release for Windows, Linux, macOS and Android user agents.
- Open: switch Pages on once (Settings → Pages → Source: GitHub Actions).

### 2026-10-02 — Phase 6: packaging, updates, logs, new interface, plugins, fast sync
- Done: `core::applog` (errors.log always, daily files on request, panics), `update` (GitHub
  release check, SHA-256 checked download, silent installer / AppImage swap), `whatsnew`,
  `report`; new interface (sidebar with Library / Player plugins / Settings, file cards with
  status and progress, details panel, settings in cards incl. Updates, Logs and problems, About;
  What's new and Report dialogs; settings saved as they change); player plugins (mpv/mpv.net
  Lua script, VLC Lua extension, `submagician-cli --player`, install/remove in the app, refreshed
  at start); `autosync` (quick look at 10 windows, parallel full read, speech cache on disk);
  subtitles inside videos (`probe::subtitle_tracks/pick/extract`, "Use the subtitle inside",
  CLI `--from-video`); `jobs` + worker process (`--worker`) with progress on every video;
  packaging (Inno Setup per user with ffmpeg, portable zip, .deb, AppImage with `--cli`),
  `release.yml` with smoke tests, `docs/RELEASING.md`, icons.
- Verified: unit tests (applog, update, whatsnew, report, players, quick fit, jobs incl. a
  crashing and a stopped worker, probe/extract); end-to-end 20-minute synthetic film: wrong frame
  rate + offset fixed by the quick look in 0.4 s, a cut scene by the parallel full read in 1.4 s,
  then from the cache; the app under Xvfb (pages, What's new, Report dialog, progress on a 1-hour
  file, "Use the subtitle inside" fixing a 3.8 s late track); mpv 0.37 with the installed script:
  the video started, the CLI wrote a subtitle and mpv loaded it (2.3 s); Lua syntax of both
  plugins; .deb and AppImage built locally.
- Open: the first release run in CI; in-app updates need public releases; VLC tried for real.

### 2026-10-02 — Phase 5 Whisper
- Done: `core::speech` (models, download, pieces cut at silence, VAD-snapped segments, sound tags
  dropped, SRT); app: Settings → Speech (model, download, write when nothing is found), "Write
  from audio"; CLI `--generate`, `--model`, `--download-model`; CI downloads ggml-tiny (cached)
  and requires the Whisper test on Linux.
- Verified: unit tests; real transcription of the 8-sentence synthetic film with ggml-tiny
  (every line within 0.8 s of its sentence, key words right); in the app under Xvfb "Write from
  audio" saved `Harbor….en.srt` starting at 1.65 s; CLI downloaded tiny from Hugging Face and
  wrote a subtitle with `--generate`.
- Found: whisper-rs 0.16's safe abort callback is unsound; the raw callback is used instead.

### 2026-10-02 — First Windows test: fixes
- Feedback: opening one video listed the whole folder; "Saved, sync failed: ffmpeg" on every
  row (no ffmpeg on Windows); the window stopped responding after "Sync to audio".
- Done: opening a video lists only that video, plus an "Open video…" button; Settings → Timing
  can download ffmpeg on Windows (gyan.dev essentials, SHA-256 checked, only ffmpeg/ffprobe
  kept, found automatically); a missing ffmpeg leaves the row "Saved" with a clear status; file
  dialogs are owned by the window; `submagician.log` in the data folder with sync steps, errors
  and panics.
- Verified: unit test for the zip extraction, live download of the real ffmpeg build (checksum,
  two `MZ` programs); in the app under Xvfb: a video from the command line listed alone, "Sync
  to audio" without ffmpeg kept "Saved" and showed the hint, log file written; Windows cross
  check.
- Open: the freeze did not reproduce here; the log file will tell if it happens again.

### 2026-10-02 — Phase 4 convenience
- Done: English-only UI for now (Turkish bundle and picker removed, strings stay in `@tr`);
  `probe` (embedded subtitle languages via ffprobe, counted as having the language, read in the
  background); `submagician-cli`; settings and engine setup moved to core; open from the command
  line; drag & drop (winit on Windows/X11, own `wl_data_device` listener on Wayland); Play / Show
  in folder; file-manager entries (`integration`); folder watch (`watch`, 10 s settle);
  Search as, Only missing, Restore previous.
- Verified: unit tests (ffprobe on a real MKV with two tracks, watcher with a growing file,
  integration files, restore swap, uri-list parsing, CLI args); CLI against Addic7ed (dry run,
  download, skip on second run); in the app under Xvfb: opening a video from the command line,
  Settings → Add to the menu wrote the launcher/script/service menu, the folder watch picked up a
  new episode and saved its subtitle, Only missing filter; app started natively on Wayland
  (weston headless) with the drop listener; core, CLI and app cross-checked for Windows
  (`x86_64-pc-windows-gnu`).
- Not verified: typing in Search as (Xvfb delivers no key presses to any field here), a real
  Wayland/Windows drop, Windows Explorer entries.

### 2026-10-02 — Live provider check, Podnapisi removed
- Done: CI job `live` ran with the repository secrets: OpenSubtitles (42 results for Inception,
  download OK, 99 left today), SubDL (30 results, download OK), Addic7ed OK. Podnapisi failed:
  `www.podnapisi.net` does not resolve, so the provider is removed.
- Verified: CI log of the `live` job on main (ddf6343).

### 2026-10-02 — Phase 3 sources
- Done: providers `gestdown` (Addic7ed TV), `subdl`, `podnapisi`; search cache (JSON files,
  3 days / 12 h); RAR and 7z through bsdtar / 7-Zip / unrar; settings: sources on/off, SubDL key,
  clear cache; "Search again" skips the cache; CI installs bsdtar and passes the SubDL secret.
- Verified: Gestdown live (search + download, `SUBMAGICIAN_LIVE=1`), fixtures from its real
  answers; SubDL's real no-key error; 7z archive unpacked through bsdtar; engine cache test; in
  the app under Xvfb a real Addic7ed search for The.Office.US.S03E07.720p.WEB-DL listed 4
  subtitles, ranked the 720p WEB-DL release first, and double-click saved `….en.srt`.
- Found: Türkçealtyazı sits behind a Cloudflare JavaScript challenge; PlanetDP search needs
  its JavaScript. Neither works over plain HTTP.
- Open/next: SubDL and Podnapisi live tests; decision on a WebView for Turkish sites; Phase 4.

### 2026-10-02 — Phase 2 timing
- Done: `timing` (SRT/VTT/ASS cue times, rewrite with everything else untouched); `audio`
  (ffmpeg discovery, 8 kHz PCM, WebRTC VAD → speech spans, progress, cancel, no console window on
  Windows); `sync` (frame-rate ratio trial, alass with splits, speech-overlap before/after,
  apply only if better; file sync / shift / reference from another subtitle). App: sync after
  download (setting, on by default), "Sync to audio", "Sync to subtitle…", −1/−0.1/+0.1/+1 s,
  ffmpeg path setting with found/not found, speech cache per video, new states and status texts
  (Turkish too). CI installs ffmpeg on Linux and requires the audio tests there.
- Verified: unit tests for plain offset, 23.976↔25 + offset, a 30 s cut scene, an already synced
  subtitle; real-audio tests with ffmpeg's flite voice (speech found at the right times; a 4 s
  late 8-line subtitle synced to within 0.5 s per line); clippy 1.99; in the app under Xvfb,
  clicking "Sync to audio" moved a 4 s late subtitle by −3.80 s (speech match 20 % → 79 %) and
  "+0.1 s" shifted it, file stayed UTF-8 with Turkish letters intact.
- Open/next: real-film check; Windows run (CI builds it; ffmpeg must be installed there);
  Phase 3 (more sources).

### 2026-10-02 — Phase 1 foundation
- Done: workspace (`core`, `app`); folder scan with existing-subtitle detection; OpenSubtitles
  hash; hunch-based name parsing with folder fallback; OpenSubtitles REST provider (hash + name
  search, optional login, download, quota/auth errors); scoring and ranking; encoding fix
  (BOM, UTF-16, cp1254 for Turkish); zip extraction and season-pack pick; save as
  `<stem>.<lang>.<ext>` with `.bak`; Slint UI (file list, candidates, batch "download best",
  settings, about); Turkish translation; settings file; CI for Linux and Windows.
- Verified: `cargo test` (core: 25 tests, including an engine run with a fake provider that
  saves a cp1254 Turkish subtitle as UTF-8), `cargo clippy -D warnings`, app started under Xvfb
  with a sample folder and rendered in Turkish.
- Open/next: OpenSubtitles application key (register at opensubtitles.com → API consumers, put
  it in the `OPENSUBTITLES_API_KEY` secret); live search/download test with it; then Phase 2.
