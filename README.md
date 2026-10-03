# SubMagician

Finds subtitles for a whole folder of videos, picks the one made for **your** file, fixes the
encoding and the timing, and saves it next to the video. Windows and Linux (macOS later).

[Türkçe](README.tr.md)

![SubMagician](docs/img/main.png)

## Download

From the [website](https://halilkhrmn.github.io/submagician/) or the
[releases page](https://github.com/halilkhrmn/submagician/releases):

- **Windows**: `submagician-setup-….exe` (no admin rights needed, ffmpeg included), or the
  portable zip.
- **Linux**: `SubMagician-…-x86_64.AppImage` (make it executable and run it), or the `.deb` for
  Debian/Ubuntu (`sudo apt install ./submagician_….deb`).

The installer and the AppImage update themselves: SubMagician tells you when a new version is
out and shows what is new after the update.

## Why

- A hash match is not always right, and a name search gives you dozens of releases to guess from.
  SubMagician scores every candidate: hash match, release group, source (BluRay/WEB), streaming
  service, resolution, name similarity, wrong episodes rejected.
- Broken Turkish characters (Windows-1254) are fixed; everything is saved as UTF-8.
- Timing is fixed from the video's audio in seconds: offset, frame rate (23.976 / 24 / 25) and
  cut or added scenes. If the subtitle already fits, it is left alone.
- Sources: OpenSubtitles, SubDL and Addic7ed (TV series, through Gestdown); each can be switched
  off. Results are cached for a few days. RAR and 7z archives are opened with bsdtar / 7-Zip
  (Windows 10+ has `tar.exe` built in).

## Use

1. **Choose folder…** or **Open videos…**, drop them on the window, or right-click a folder or
   a video in your file manager (Settings → Right-click menu adds SubMagician there and picks
   its entries: open in SubMagician, get subtitles, sync the subtitle to the audio).
2. **Get subtitles for all**, or click a video: the panel on the right shows its subtitles as
   flags and the ones the sources offer, best first, each marked **Exact** (made for this very
   file), **Good**, **Fair** or **Weak**. **Search as…** finds a wrongly named file under
   another name.
3. The subtitle is saved as `Movie.tr.srt` next to `Movie.mkv`; players load it on their own.
   It is synced to the audio right away. The file it replaced is kept: the undo button next to
   *Sync to audio* puts it back.

![Sync](docs/img/sync.png)

More:

- **Fast sync**: SubMagician first listens to a few short parts across the film at once, which
  is enough for a wrong offset or frame rate (a second or two). Only when scenes were cut or
  added does it read the whole audio, one piece per CPU core. What it heard is remembered, so
  syncing the same video again is instant. *Sync to audio* works on any subtitle; *To a
  subtitle…* uses another subtitle that is in sync; ±0.1 s / ±1 s nudge it by hand.
- **Subtitles inside the video** (MKV/MP4 tracks): **Use the subtitle inside** saves the one in
  your language as a file and syncs it to the audio. Picture tracks (Blu-ray, DVD) cannot be
  used as text.
- Heavy work (syncing, writing from the audio) runs in a separate process: the window never
  freezes, every video shows its progress, **Stop** ends it at once.
- **Watch the open folder** (Settings → Library): new videos in the folder get their subtitle
  on their own, once they have finished copying. **Only videos without a subtitle** hides the
  ones that are done.
- **From audio** (Whisper, on this computer): when no source has a subtitle, one is written from
  the speech. The first time it offers to download the speech model; Settings → Speech picks
  another one, and it can also run automatically.
  Into English it translates any language; other languages are written as spoken.

### Player plugins

**Settings → Player plugins** finds mpv (and mpv.net) and VLC on your computer and installs
SubMagician into them with one click. Then:

- **mpv**: a video without a subtitle in your language gets one by itself; **Alt+S** asks for one,
  **Alt+Shift+S** searches again.
- **VLC**: *View → SubMagician* finds a subtitle for the video that is playing and, while it is
  on, for every video that starts.

The plugins use SubMagician's settings: your languages, sources, sync and Whisper. Players
installed from Flatpak or Snap run in a sandbox and cannot use the plugin.

![Player plugins](docs/img/players.png)

### Settings

Languages in order (`tr, en`), sources, optional OpenSubtitles login for a higher daily download
limit, sync and ffmpeg, Whisper, updates, logs. **Logs and problems**: warnings and errors are
always written to `errors.log`; *Save detailed logs* records every step for a while; **Report a
problem** shows exactly what would be sent, then opens a GitHub issue or an e-mail.

## Command line

`submagician-cli` does the same for scripts, using the app's settings:

```sh
submagician-cli ~/Videos                      # best subtitle + sync for every video
submagician-cli -l tr,en --dry-run Film.mkv   # show what it would pick
submagician-cli --from-video ~/Videos         # use the subtitles inside the videos, synced
submagician-cli --sources addic7ed --no-sync ~/Shows/The.Office
submagician-cli --download-model base && submagician-cli --generate ~/Videos
```

`submagician <folder or video>` opens the app on that folder. From the AppImage:
`SubMagician-….AppImage --cli …`.

## Build

Rust 1.88+, a C/C++ compiler and CMake. On Linux: `libfontconfig1-dev libxkbcommon-dev`. For syncing and
embedded tracks: `ffmpeg` and `ffprobe` on PATH or next to the program.

```sh
SUBMAGICIAN_OPENSUBTITLES_API_KEY=… SUBMAGICIAN_SUBDL_API_KEY=… \
  cargo build --release -p submagician -p submagician-cli
```

Packages: see `docs/RELEASING.md`. Roadmap: `docs/PLAN.md`. License: AGPL-3.0.
