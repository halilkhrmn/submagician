# AGENTS.md — SubMagician

Guide for anyone (human or AI agent) working on this repo. **Read this first, then `docs/PLAN.md`
and `docs/PROGRESS.md`.**

Desktop app (Rust + Slint) that finds subtitles for a folder of videos, picks the one made for
the file, fixes encoding (and, from Phase 2, timing) and saves it next to the video.

## Project docs — keep them current

| File | What it holds | When to update |
|---|---|---|
| `AGENTS.md` | Rules, architecture, commands, file map | When structure, commands or conventions change |
| `docs/PLAN.md` | Product plan, sources, phases | When scope or phases change |
| `docs/PROGRESS.md` | Phase checklist + dated work log | **At the end of every work session** |
| `docs/DECISIONS.md` | Numbered decisions with reasons | Whenever a non-obvious choice is made |

Work-log entries: newest on top, `### YYYY-MM-DD — short title`, then bullets for *done*, *verified how*, *open/next*.

## Rules

- Commits are authored as the owner: `Halil Kahraman <52932792+halilkhrmn@users.noreply.github.com>`
  (`git config user.name/user.email` in the clone). No AI co-author or session trailers in commit
  messages.
- Platforms: Windows and Linux first; macOS is Phase 7. Do not add platform-only features without
  a plan for the other platform.
- `core` must not depend on Slint or any GUI crate; everything testable goes there.
- Sources go behind `core::provider::Provider`. APIs where a site has one; scraping only where it
  has none, rate-limited and cached. Never put user credentials in code or logs.
- Provider application keys come from build-time env (`SUBMAGICIAN_OPENSUBTITLES_API_KEY`,
  `SUBMAGICIAN_SUBDL_API_KEY`) and CI secrets (`OPENSUBTITLES_API_KEY`, `SUBDL_API_KEY`); never
  commit them.
- The UI is English only for now (translations come later). Keep every user-visible string in
  `.slint` inside `@tr(...)` so translations can be added as gettext files without code changes;
  Rust passes state codes, not sentences.
- `README.md` is the main README and stays in English; `README.tr.md` is its translation. Change
  them together.

## Commands

Needs Rust 1.88+ (stable) and a C compiler (webrtc-vad). Linux build needs `libfontconfig1-dev`
and `libxkbcommon-dev` (runtime: `libxkbcommon-x11-0` on X11). Syncing to audio needs `ffmpeg`.

| What | Command |
|---|---|
| All tests | `cargo test --all` (audio tests need ffmpeg with flite, the 7z test bsdtar; `SUBMAGICIAN_REQUIRE_FFMPEG=1` / `SUBMAGICIAN_REQUIRE_BSDTAR=1` make a skip fail) |
| Live provider tests | `SUBMAGICIAN_LIVE=1 cargo test -p submagician-core live_ -- --nocapture` (needs the provider keys at build time; CI job `live` runs them with the secrets) |
| Lint | `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` |
| Run | `cargo run -p submagician` |
| Run with an OpenSubtitles key | `SUBMAGICIAN_OPENSUBTITLES_API_KEY=… cargo run -p submagician` |
| Release build | `cargo build --release -p submagician -p submagician-cli` |
| CLI | `cargo run -p submagician-cli -- --dry-run <folder>` |
| Windows compile check from Linux | `rustup target add x86_64-pc-windows-gnu`, MinGW, `cargo check --target x86_64-pc-windows-gnu` |
| Headless screenshot (Linux) | `xvfb-run -a env SLINT_BACKEND=winit-software target/debug/submagician` + `import -window root shot.png` |

## Architecture and file map

```
crates/core/   submagician-core, no GUI (shared by app and CLI)
  media        folder scan, video/subtitle extensions, existing <stem>.<lang>.srt detection
  hash         OpenSubtitles moviehash
  name         hunch-based name parsing (title, year, S/E, source, group), folder fallback, tokens
  lang         language table (OpenSubtitles codes, ISO 639-2, encoding-detection TLD hint)
  provider/    Provider trait, SearchQuery, Candidate; opensubtitles (REST), subdl (JSON),
               gestdown (Addic7ed TV through api.gestdown.info)
  cache        search results on disk (JSON per provider+query, 3 days / 12 h)
  score        candidate scoring and ranking, best pick per language order
  text         bytes → UTF-8 (BOM, UTF-16, chardetng, cp1254 for Turkish), format detection
  timing       SRT/VTT/ASS cue times: parse, change, render with the rest untouched
  audio        ffmpeg discovery, audio → 8 kHz PCM → WebRTC VAD → speech spans
  sync         frame-rate trial + alass alignment, speech overlap, apply-if-better; file helpers
  archive      zip extraction; RAR/7z through bsdtar / 7-Zip / unrar
  output       <stem>.<lang>.<ext> writer (UTF-8 BOM, CRLF, .bak once)
  engine       query → search all providers → rank → fetch → decode → save
  settings     settings file (JSON in the OS config folder) and the engine it describes
  probe        subtitle track languages inside a video (ffprobe)
  watch        folder watch: videos that appeared and stopped growing
  integration  "Find subtitles" in the file manager (HKCU registry / Linux launchers, scripts)
  tools        Windows: ffmpeg/ffprobe download on request (gyan.dev, SHA-256 checked)
crates/app/    submagician (binary)
  ui/app.slint window: Subtitles / Settings / About tabs, Texts global (state codes → @tr text)
  src/main.rs  startup, tokio runtime, command-line path, drag & drop
  src/controller.rs  UI callbacks → tokio tasks → upgrade_in_event_loop; batch runs, epochs
  src/wayland_drop.rs  drops under Wayland (own wl_data_device on winit's connection)
  assets/icon.svg
crates/cli/    submagician-cli: same pipeline for scripts (--lang, --sources, --dry-run, …)
docs/          PLAN, PROGRESS, DECISIONS
```

- Work runs on a 2-thread tokio runtime; the UI thread only touches Slint models. Results of old
  work are dropped by comparing the folder *epoch* (bumped on every rescan).
- One busy operation at a time (`busy` property); batch runs stop on fatal errors (no key, login
  failed, download limit).
