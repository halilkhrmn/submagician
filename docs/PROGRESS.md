# Progress

## Phases

- [x] Phase 1 — Foundation: scan, hash, name parse, OpenSubtitles, scoring, UTF-8 fix, save, UI, CI
  - [ ] Live test against OpenSubtitles with a real application key
- [ ] Phase 2 — Timing: ffmpeg, VAD + alass sync, frame rate, reference sync, manual offset
- [ ] Phase 3 — More sources: SubDL, Podnapisi, Addic7ed, Turkish scrapers, RAR, cache
- [ ] Phase 4 — Convenience: embedded tracks, watch folder, drag & drop, context menu, CLI
- [ ] Phase 5 — Speech: Whisper generate / verify
- [ ] Phase 6 — Packaging for Windows and Linux
- [ ] Phase 7 — macOS

## Work log

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
