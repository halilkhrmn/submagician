# Changelog

Release notes, newest first. The app shows the sections since the version that ran before
("What's new"), and the release workflow uses the section of the released version.

## 0.1.1
- Fixed: writing a subtitle from the speech ("From audio") crashed on many processors (the speech engine was built for the build
  server's processor). It now works on any 64-bit PC from 2013 on, and says so clearly when a
  processor is too old.
- A simpler window: no sidebar. Settings open from the gear at the top and have a back button;
  player plugins, subfolders and the folder watch moved there.
- Subtitle languages are shown as flags, and the found subtitles say how well they fit:
  Exact, Good, Fair or Weak (the ? button explains them).
- "From audio" is now "Write from speech" and "To a subtitle…" is "Copy timing…", each with a
  short explanation next to it. Writing from speech asks to download the speech model right
  there instead of sending you to Settings.
- If a background task crashes, SubMagician says so and offers to report it by GitHub or e-mail.
- Right-click menu: choose its entries in Settings: open in SubMagician, get subtitles in your
  languages, or sync the subtitle to the audio.

## 0.1.0
- First release: finds subtitles on OpenSubtitles, SubDL and Addic7ed, picks the one made for
  your file, fixes the text encoding and saves it next to the video.
- Syncs subtitles to the audio (frame rate, offset and splits) and writes one from the audio
  with Whisper when no source has it.
- Folder watch, drag and drop, "Find subtitles" in the file manager and a command-line tool.
- Player plugins for mpv and VLC: press a key in the player to find and sync a subtitle.
- New look with a sidebar, file cards and a details panel.
- Updates itself (Windows installer and AppImage) and shows what's new after an update.
- Logs: problems are always logged; detailed logs can be switched on in Settings, and
  "Report a problem" prepares a report for GitHub or e-mail.
