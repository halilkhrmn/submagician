# SubMagician — plan

Desktop app that finds subtitles for a folder of videos, picks the one made for *your* file,
fixes its encoding and timing, and saves it next to the video. Rust + Slint.

Platforms: **Windows and Linux first**; macOS once everything else is done.

## The problem

- Hash search (OpenSubtitles moviehash) identifies the file, but uploaders sometimes attach the
  wrong subtitle to a hash, so a "hash match" can still be off.
- Name search returns dozens of subtitles and you have to guess which release fits.
- Even the right subtitle drifts when the release differs: other cut (intro, recap, extended),
  other frame rate (23.976 vs 25).
- Turkish subtitles in Windows-1254 show broken ı ş ğ İ.

So the app does two jobs: **pick** the most likely subtitle, then **fix** it (encoding, timing).

## Sources

APIs where a site has one, scraping only where it has none. Every source sits behind the
`Provider` trait, so a broken site only disables itself.

| Source | How | User needs |
|---|---|---|
| OpenSubtitles.com | REST API, application key built in (like VLSub) | nothing; optional login raises the daily limit |
| SubDL | API, application key built in | nothing |
| Podnapisi | site JSON search | nothing |
| Addic7ed (TV) | via Gestdown | nothing |
| Turkish sites (Türkçealtyazı, …) | scraping, rate-limited, cached | nothing |

## Phases

### Phase 1 — Foundation (search, pick, save)
- Workspace: `core` (no GUI, unit-tested) + `app` (Slint).
- Folder scan (optional subfolders), skip samples, detect existing `<stem>.<lang>.srt`.
- OpenSubtitles hash, file/folder name parsing (hunch).
- OpenSubtitles REST provider: hash + name search, optional login, download, quota messages.
- Scoring: hash match, release group, source, streaming service, resolution, name similarity,
  wrong-episode rejection, trusted / machine-translated / downloads / rating.
- Encoding fix to UTF-8 (cp1254 for Turkish), zip extraction, season-pack file pick.
- Save as `<stem>.<lang>.<ext>` (UTF-8 BOM), previous file kept once as `.bak`.
- UI: file list with state, ranked candidates, "Download best for all", per-file download,
  settings (languages, login, API key, UI language), Turkish + English UI.
- CI on Linux and Windows.

### Phase 2 — Timing (auto-sync)
- ffmpeg/ffprobe discovery (PATH, bundled next to the exe, setting).
- Audio → voice activity (webrtc-vad) → align subtitle with `alass-core`; handles offset,
  frame-rate drift and cut/ad splits.
- Frame-rate detection (ffprobe) and 23.976↔25 conversion.
- Reference sync: align the wanted-language subtitle to a hash-matched subtitle in another
  language (no audio needed).
- "Sync after download" switch; per-file "Sync now"; result shown (offset, confidence).
- When candidates are close, try-sync the top few and keep the best fit.
- Manual fine-tune: ± offset with preview of lines at a given time.

### Phase 3 — More sources
- SubDL, Podnapisi, Addic7ed (Gestdown) providers.
- Turkish site scrapers (polite: rate limit, cache, clear User-Agent).
- RAR support for archives from Turkish sites.
- Search cache (SQLite) so re-opening a folder does not re-query; provider on/off in settings.

### Phase 4 — Convenience
- Embedded subtitle tracks (ffprobe): skip videos that already carry the wanted language.
- Watch folder: new videos get subtitles automatically.
- Drag & drop folders/files, open containing folder, open video in the default player.
- Explorer context menu (Windows) and file-manager action (Linux): "Find subtitles".
- Command line mode (`submagician <folder>`) for scripts.

### Phase 5 — Speech (Whisper)
- whisper.cpp through `whisper-rs`, model downloaded on demand by the user.
- Generate a subtitle when no source has one; optional translation.
- Use a short transcript to check which candidate matches the audio.

### Phase 6 — Packaging (Windows, Linux)
- Windows: installer (MSI or Inno Setup), ffmpeg bundled, icon/version resources.
- Linux: AppImage and .deb (Flatpak later).
- Release workflow building with the application keys from repository secrets.
- Password to the OS keyring; update check (opt-in).

### Phase 7 — macOS
- .app bundle, dmg, signing/notarization; portal-free folder dialog check.
