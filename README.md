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
- Coming next: automatic timing fix from the audio (alass), more sources, Whisper.

## Use

1. **Choose folder…** — videos in it (and its subfolders) are listed with the subtitles they
   already have.
2. **Download best for all**, or click a video to see ranked candidates and download one.
3. The subtitle is saved as `Movie.tr.srt` next to `Movie.mkv`; players load it on their own.

Settings: wanted languages in order (`tr, en`), optional OpenSubtitles login for a higher daily
download limit, interface language.

## Build

Rust 1.88+. On Linux: `libfontconfig1-dev libxkbcommon-dev`.

```sh
SUBMAGICIAN_OPENSUBTITLES_API_KEY=your-app-key cargo build --release -p submagician
```

See `docs/PLAN.md` for the roadmap.
