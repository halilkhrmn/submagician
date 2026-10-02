# Progress

## Phases

- [x] Phase 1 — Foundation: scan, hash, name parse, OpenSubtitles, scoring, UTF-8 fix, save, UI, CI
  - [ ] Live test against OpenSubtitles with a real application key
- [x] Phase 2 — Timing: ffmpeg, VAD + alass sync, frame rate, reference sync, manual offset
  - [ ] Check on real films (long soundtracks, music-heavy scenes) and tune VAD / thresholds
- [x] Phase 3 — More sources: SubDL, Podnapisi, Addic7ed, RAR/7z, cache, source switches
  - [ ] SubDL live test (needs the application key, `SUBDL_API_KEY` secret)
  - [ ] Podnapisi live test (site unreachable from the dev environment)
  - [ ] Turkish sites: decide on a hidden WebView (Türkçealtyazı and PlanetDP need JavaScript)
- [ ] Phase 4 — Convenience: embedded tracks, watch folder, drag & drop, context menu, CLI
- [ ] Phase 5 — Speech: Whisper generate / verify
- [ ] Phase 6 — Packaging for Windows and Linux
- [ ] Phase 7 — macOS

## Work log

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
