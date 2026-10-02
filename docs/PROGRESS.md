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
- [ ] Phase 5 — Speech: Whisper generate / verify
- [ ] Phase 6 — Packaging for Windows and Linux
- [ ] Phase 7 — macOS

## Work log

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
