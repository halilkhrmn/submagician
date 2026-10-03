# Releasing

Releases are built and published by `.github/workflows/release.yml`.

## Publish a version

1. Raise `version` in the root `Cargo.toml` (`[workspace.package]`), e.g. `0.1.0` → `0.2.0`.
2. Add a `## 0.2.0` section at the top of `changelog/en.md`: short user-facing bullets. The app
   shows it as "What's new" after the update, and it becomes the release notes. The workflow
   stops if the section is missing.
3. Merge the pull request into `main`. The workflow sees a version without a tag, builds
   everything, installs and starts each package on a clean system (smoke tests), then creates the
   tag `v0.2.0` and the GitHub release with the files and `SHA256SUMS.txt`.

Pushing a tag `vX.Y.Z` by hand works too (it must match `Cargo.toml`). **Actions → Release →
Run workflow** builds and tests everything without publishing (a dry run).

The provider keys come from the repository secrets `OPENSUBTITLES_API_KEY` and `SUBDL_API_KEY`;
a build without them works but needs the user's own keys.

## What is published

| File | For |
|---|---|
| `submagician-setup-X.Y.Z.exe` | Windows installer: per user, no admin rights, ffmpeg and ffprobe included. Updates itself from inside the app. |
| `SubMagician-X.Y.Z-windows-x64-portable.zip` | Windows without installing (no in-app updates; the app links to the release). |
| `SubMagician-X.Y.Z-x86_64.AppImage` | Any Linux. Updates itself from inside the app. `--cli` runs the command-line tool. |
| `submagician_X.Y.Z-1_amd64.deb` | Debian/Ubuntu: `sudo apt install ./submagician_….deb` (recommends ffmpeg). |

The Linux binaries are built on Ubuntu 22.04, so they run on that glibc and newer.

## In-app updates

The app asks `api.github.com/repos/halilkhrmn/submagician/releases/latest` at start (Settings →
Updates can switch that off) and offers the newer version. The installer and the AppImage are
downloaded into the data folder, checked against the SHA-256 digest GitHub publishes for each
release file, then:

- Windows: the installer runs with `/VERYSILENT /CLOSEAPPLICATIONS /RELAUNCH=1`; it closes the
  app, replaces the files and starts it again.
- AppImage: the new file replaces the running one, which starts it once it has exited.

The repository is public, so every installed copy sees new releases. (A 404 — no release yet —
means "no update".)

## Website

`site/` is the landing page at <https://halilkhrmn.github.io/submagician/>, published by
`.github/workflows/pages.yml` on every push to `main` that changes it (or by hand: Actions →
Pages → Run workflow). Its download button asks the GitHub API for the newest release and picks
the file for the visitor's system, so it needs no change when a version is released. One-time
setup: Settings → Pages → Build and deployment → Source: **GitHub Actions**.

## Build by hand

- Windows: `tools\build-installer.ps1` (Inno Setup 6: `winget install JRSoftware.InnoSetup`).
  Output in `target\dist`.
- Linux: `tools/build-linux-packages.sh` (`cargo install cargo-deb`). Output in `target/linux`.
- Smoke tests: `tools/smoke-linux.sh deb|appimage <file>`, `tools\smoke-windows.ps1 <setup.exe>`.
