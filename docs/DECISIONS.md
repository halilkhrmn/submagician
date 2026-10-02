# Decisions

Numbered, newest last. Each: what, and why.

1. **Rust + Slint, workspace `core` + `app`.** `core` has no GUI dependency, so scanning, scoring,
   encoding and providers are unit-tested on any machine; `app` only wires the window to it.
2. **Fluent style on every platform.** One look to design and screenshot; native styles differ
   in sizes and would need per-platform layout checks.
3. **APIs first, scraping only where a site has no API.** Scrapers break when HTML changes and
   hit captchas/Cloudflare; hash search is API-only anyway. Every source is a `Provider`, so a
   broken one does not stop the others.
4. **Provider API keys belong to the application** (build-time env
   `SUBMAGICIAN_OPENSUBTITLES_API_KEY`, CI secret `OPENSUBTITLES_API_KEY`), the same way VLSub
   ships its key. Users never need a key; a settings field can override it. Login is optional and
   only raises the download limit.
5. **Output name `<video stem>.<lang>.<ext>`.** VLC, mpv, MPC-HC, Kodi, Plex and Jellyfin load it
   automatically and it shows the language. An existing file is kept once as `.bak`.
6. **UTF-8 with BOM and CRLF.** Older players and TVs only recognize UTF-8 with a BOM; current
   players ignore it. CRLF is what most SRT files use.
7. **Turkish: a Windows-1252/ISO-8859-2 guess becomes Windows-1254.** Detectors often pick a
   Latin encoding for short Turkish files; the bytes are the same except ı ş ğ İ, which is exactly
   what breaks.
8. **Scoring** (`core/src/score.rs`): hash match +100; same release group +40; same source +25
   (different −10); same streaming service +10; same resolution +5; name similarity up to +30;
   trusted +5; machine translated −40; hearing impaired −3; downloads (log) and rating as small
   tie-breakers; a different season/episode rejects the candidate. Languages are ranked first,
   in the user's order.
9. **UI strings through Slint `@tr` with bundled gettext files** (bundle removed for now, see 25) (`crates/app/lang/<lang>/LC_MESSAGES/submagician.po`),
   no translation context. Messages that come from Rust are passed as state codes and turned into
   text in `.slint`, so they are translated too. Core error details stay English for now.
10. **Settings as JSON in the OS config folder.** The OpenSubtitles password is stored in plain
    text until Phase 6 moves it to the keyring.
11. **License: AGPL-3.0** (the owner's choice when the repository was created). Slint is used
    under its GPLv3 option, which AGPL-3.0 §13 allows combining with; the `AboutSlint` notice on
    the About tab is kept anyway.
12. **Own timing parser, not `subparse`.** Syncing only needs the cue times; a small line-based
    parser for SRT, WebVTT and ASS/SSA rewrites just the time fields, so styles, tags, positions
    and comments survive untouched. MicroDVD (`.sub`, frame based) is not synced.
13. **Audio: ffmpeg → 8 kHz mono PCM → WebRTC VAD (aggressive mode) in 10 ms frames.** Aggressive
    mode ignores more music and noise, which movie soundtracks have a lot of. Pauses under 200 ms
    are joined and voiced bits under 100 ms dropped. ffmpeg runs as a separate process (found via
    setting, next to the exe, or PATH; bundled in Phase 6), with no console window on Windows.
14. **Frame rate by trial, not ffprobe.** Each common ratio (23.976 / 24 / 25 pairs) is tried with
    alass' single-offset alignment and overlap scoring; the best (by more than 1 %) wins. This also
    catches subtitles timed for a sped-up release when the video's own frame rate says nothing.
15. **alass with split penalty 7 and speed optimization 1** (its documented defaults), so cut or
    added scenes get their own offsets.
16. **A sync is applied only if it raises the speech overlap by 2 points or more.** The overlap
    is the share of subtitle time that falls on detected speech, shown to the user before → after.
    It protects a subtitle that was already right from being moved by a noisy soundtrack.
17. **Synced and shifted subtitles are rewritten in place** (UTF-8 BOM, CRLF, atomic). The
    download step's `.bak` keeps the file that was there before, if any; manual ± steps undo a
    shift. Speech spans are cached per video for the session.
18. **`Candidate.provider` is a `String`** so candidates can be stored in the search cache.
19. **Search cache as JSON files, not SQLite.** One file per provider and query (FNV-1a of the
    query as name) in the OS cache folder; found results kept 3 days, empty ones 12 hours (new
    releases get subtitles within hours). No C library and nothing to migrate. "Search again"
    skips it; Settings can clear it.
20. **RAR/7z through an external tool**, not a library: the unRAR source license is not
    free-software compatible (AGPL), and no mature pure-Rust RAR reader exists. Order: `bsdtar`,
    `tar` if it is bsdtar (Windows 10+ ships one as `tar.exe`), `7z`/`7za`, `unrar`.
21. **Addic7ed through Gestdown's API**, not by scraping addic7ed.com: Gestdown mirrors it with
    JSON, no key, and handles Addic7ed's rate limits. The show is picked by title similarity plus
    a "(US)"/"(2005)" tag found in the file name, then by having the season.
22. **SubDL is only added when it has a key** (built in or from Settings), so a missing key
    never stops a batch run.
23. **No Podnapisi.** It was added through its XML interface, then removed: `www.podnapisi.net`
    no longer resolves in DNS (CI live test, 2026-10-02).
24. **No plain-HTTP scrapers for Türkçealtyazı and PlanetDP** (see Phase 3 in PLAN): both need
    JavaScript, so a parser alone would never get past the first request.
25. **English-only UI for now** (owner's call). The Turkish bundle and the language picker are
    removed; strings stay in `@tr(...)`, so a language system can come back by adding `.po` files
    and `with_bundled_translations` in `build.rs`.
