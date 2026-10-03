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
| Addic7ed (TV) | via Gestdown's public API | nothing |
| Turkish sites (Türkçealtyazı, PlanetDP) | need a browser engine (see Phase 3 notes) | — |

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
- ffmpeg discovery (setting, next to the exe, PATH).
- Audio → voice activity (webrtc-vad) → align subtitle with `alass-core`; handles offset,
  frame-rate drift and cut/ad splits.
- Frame-rate mismatch (23.976 / 24 / 25) found by trying the ratios against the reference.
- Reference sync: align to another subtitle that is already in sync (no audio needed).
- "Sync after download" switch; per-file "Sync to audio" / "Sync to subtitle…"; result shown
  (shift, frame-rate fix, speech match before → after); a sync that does not improve the match
  is not applied.
- Manual fine-tune: −1 s / −0.1 s / +0.1 s / +1 s.

### Later
- Interface translations (Turkish first): add `.po` files back, language picker.
- Tray icon / start minimized with the folder watch; desktop notification when a watched video
  got its subtitle.
- Keyboard shortcuts (Enter = download selected, F5 = search again, Ctrl+O = choose folder).
- Sort and search in the file list; remember window size and column layout.
- Subtitle preview: the lines around a moment, before and after a sync.
- History: which subtitle came from where, with a way to report a bad one.
- When candidates are close, download and try-sync the top few, keep the best fit (costs
  downloads from the daily limit, so opt-in).
- Preview of the lines around a chosen time while fine-tuning.
- Pick the audio track by language when a video has several.

### Phase 3 — More sources
- SubDL and Addic7ed (Gestdown) providers. (Podnapisi was added and removed: podnapisi.net no
  longer resolves.)
- RAR and 7z archives (through bsdtar / 7-Zip / unrar).
- Search cache on disk so re-opening a folder does not re-query; provider on/off in settings.
- Turkish sites: **not doable with plain HTTP**. Türkçealtyazı answers every request with a
  Cloudflare JavaScript challenge ("Just a moment…", `cf-mitigated: challenge`); PlanetDP's
  search ignores the query without its JavaScript and form token. Scraping them needs a real
  browser engine (a hidden system WebView: WebView2 on Windows, WebKitGTK on Linux) that
  passes the challenge and hands the page to the parser. Decision pending (see PROGRESS).

### Phase 4 — Convenience
- Embedded subtitle tracks (ffprobe): skip videos that already carry the wanted language.
- Watch folder: new videos get subtitles automatically once they stop growing.
- Drag & drop folders/videos (Windows, X11 through winit; Wayland through our own
  `wl_data_device`), Play, Show in folder.
- Explorer context menu (Windows) and file-manager entries (Linux: Open with, Nautilus/Nemo/Caja
  scripts, Dolphin service menu): "Find subtitles with SubMagician".
- `submagician <folder|video>` opens the app there; `submagician-cli` for scripts.
- Search as (another name for badly named files), Only missing filter, Restore previous.
- UI is English only for now; a language system comes back later.

### Phase 5 — Speech (Whisper)
- whisper.cpp through `whisper-rs` (core feature `whisper`), models downloaded on request
  (tiny … large-v3-turbo) into the data folder.
- Write a subtitle from the audio for one video ("Write from audio"), or automatically when no
  source has one (setting; batch and watch), and from the CLI (`--generate`).
- Into English any language is translated; other targets get the spoken language (Whisper
  only translates into English).
- Later: use a short transcript to check which candidate matches the audio; Whisper segments as
  a sync reference for music-heavy films; GPU builds (CUDA/Vulkan/Metal).

### Phase 6 — Packaging (Windows, Linux) and polish
- Windows: Inno Setup installer per user (no admin), ffmpeg bundled, icon/version resources;
  portable zip.
- Linux: AppImage (`--cli` runs the tool) and .deb (Flatpak later).
- Release workflow: version without a tag on main → build, smoke-test, publish with the keys from
  the repository secrets; notes from `changelog/en.md`.
- In-app updates (installer, AppImage), "What's new" after an update.
- Logs (errors always, detailed on request) and "Report a problem" (GitHub issue / e-mail).
- New interface: sidebar, file cards, details panel, settings in cards (About, Logs, Updates).
- Player plugins: mpv/mpv.net script and VLC extension running `submagician-cli --player`,
  installed from the app.
- Fast sync: quick look at windows across the film, parallel full read, speech cache; subtitles
  inside videos taken out and synced; heavy jobs in a worker process with per-video progress.
- Later: password to the OS keyring; Flatpak; signed Windows installer.

### Phase 7 — macOS
- .app bundle, dmg, signing/notarization; portal-free folder dialog check.
