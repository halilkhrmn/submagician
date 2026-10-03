# Changelog

Release notes, newest first. The app shows the sections since the version that ran before
("What's new"), and the release workflow uses the section of the released version.

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
