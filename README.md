# SubMagician

Finds subtitles for a whole folder of videos, picks the one made for **your** file, fixes the
encoding and saves it next to the video. Windows and Linux (macOS later).

[Türkçe](README.tr.md)

![SubMagician](docs/img/main-tr.png)

## Why

- A hash match is not always right, and a name search gives you dozens of releases to guess from.
  SubMagician scores every candidate: hash match, release group, source (BluRay/WEB), streaming
  service, resolution, name similarity, wrong episodes rejected.
- Broken Turkish characters (Windows-1254) are fixed; everything is saved as UTF-8.
- Timing is fixed from the video's audio: offset, frame rate (23.976 / 24 / 25) and cut or added
  scenes. If the subtitle already fits, it is left alone. You can also sync to another subtitle
  that is in sync, or nudge it by ±0.1 s / ±1 s.
- Sources: OpenSubtitles, SubDL and Addic7ed (TV series, through Gestdown); each can
  be switched off. Results are cached for a few days. RAR and 7z archives are opened with
  bsdtar / 7-Zip (Windows 10+ has `tar.exe` built in).
- Coming next: watch folder, embedded subtitle tracks, Whisper.

## Use

1. **Choose folder…** — videos in it (and its subfolders) are listed with the subtitles they
   already have.
2. **Download best for all**, or click a video to see ranked candidates and download one.
3. The subtitle is saved as `Movie.tr.srt` next to `Movie.mkv`; players load it on their own.
4. With **ffmpeg** installed it is synced to the audio right after the download (Settings →
   Timing), or click **Sync to audio** for a subtitle you already have.

![Sync](docs/img/sync-tr.png)

Settings: wanted languages in order (`tr, en`), optional OpenSubtitles login for a higher daily
download limit, interface language.

## Build

Rust 1.88+ and a C compiler. On Linux: `libfontconfig1-dev libxkbcommon-dev`. For syncing:
`ffmpeg` on PATH or next to the program.

```sh
SUBMAGICIAN_OPENSUBTITLES_API_KEY=your-app-key cargo build --release -p submagician
```

See `docs/PLAN.md` for the roadmap. License: AGPL-3.0.
