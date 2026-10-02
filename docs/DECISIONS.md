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
9. **UI strings through Slint `@tr` with bundled gettext files** (`crates/app/lang/<lang>/LC_MESSAGES/submagician.po`),
   no translation context. Messages that come from Rust are passed as state codes and turned into
   text in `.slint`, so they are translated too. Core error details stay English for now.
10. **Settings as JSON in the OS config folder.** The OpenSubtitles password is stored in plain
    text until Phase 6 moves it to the keyring.
11. **License: AGPL-3.0** (the owner's choice when the repository was created). Slint is used
    under its GPLv3 option, which AGPL-3.0 §13 allows combining with; the `AboutSlint` notice on
    the About tab is kept anyway.
