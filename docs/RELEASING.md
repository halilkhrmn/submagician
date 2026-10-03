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
| `submagician-X.Y.Z-1.fcNN.x86_64.rpm` | Fedora (the current release), built from the same source RPM COPR gets. Fedora users are better served by COPR (updates through dnf). |
| `SubMagician-X.Y.Z-x86_64.flatpak` | Any Linux with Flatpak: `flatpak install --user ./SubMagician-….flatpak`; ffmpeg inside. Needs the Flathub remote for its runtime. |

The .deb and AppImage binaries are built on Ubuntu 22.04, so they run on that glibc and newer.
The RPM and the Flatpak come from `.github/workflows/packaging.yml` (also run on pull requests
that change `packaging/`), each installed and started on a clean system before publishing.

Every release also needs a `<release version="X.Y.Z" date="…">` entry in
`packaging/linux/io.github.halilkhrmn.SubMagician.metainfo.xml` (the software centers show it);
the workflow stops if it is missing.

## Fedora COPR (one-time setup)

Fedora users install from the COPR repository `halilkahraman/SubMagician`:
`sudo dnf copr enable halilkahraman/SubMagician && sudo dnf install submagician`.
The release workflow builds a source RPM (`packaging/fedora/make-srpm.sh`: the release's source,
all crates vendored, the provider keys from the secrets) and sends it to COPR with `copr-cli`;
COPR builds it for every Fedora version in the project, without network.

1. On <https://copr.fedorainfracloud.org> (log in with a Fedora account), **New project**:
   - Project name: `SubMagician` (case-sensitive, cannot be renamed later)
   - Chroots: the supported Fedora releases, `x86_64` and `aarch64` (e.g. `fedora-42`,
     `fedora-43`; Fedora needs Rust 1.88 or newer, which their updates have).
   - Internet access during the build: **not** needed.
2. **API token**: <https://copr.fedorainfracloud.org/api/> shows a `[copr-cli]` block. Copy the
   whole block and save it on GitHub: **Settings → Secrets and variables → Actions → New
   repository secret**, name `COPR_CONFIG`. The token expires after 180 days; the page renews it
   (paste the new block into the secret).
3. If the project has another name or owner, set the repository **variable** `COPR_PROJECT`
   (e.g. `someone/SubMagician`).
4. From then on every published release starts a COPR build (job `copr` in the Release
   workflow; without the secret it only warns). Builds are listed on the project's page.

Fallback without the secret: in the COPR project, **Packages → New package**, source type
**SCM**, clone URL `https://github.com/halilkhrmn/submagician.git`, spec file
`packaging/fedora/submagician.spec`, SRPM build method **make_srpm** (`.copr/Makefile`), then
**Rebuild**. It builds the newest `v*` tag, but without the provider keys (users then need their
own OpenSubtitles key).

Note: the source RPM carries the provider keys in `submagician-build.env`, as readable as they
are in the released binaries. COPR source RPMs are public.

Build locally: `sh packaging/fedora/make-srpm.sh target/srpm` (needs `cargo`, `git`, `rpm-build`),
then `rpmbuild --rebuild target/srpm/submagician-*.src.rpm` on Fedora, or `mock` for a clean chroot.

## Flatpak and Flathub

`packaging/flatpak/io.github.halilkhrmn.SubMagician.yml` builds SubMagician with ffmpeg on the
freedesktop 25.08 runtime; `tools/build-flatpak.sh` generates the crate list
(`cargo-sources.json`, from Cargo.lock) and makes the bundle the release publishes. Locally:
`flatpak-builder` and `flatpak` installed, then `./tools/build-flatpak.sh`.

The sandbox gets the home folder and removable drives (subtitles are written next to the videos),
the network and "Show in folder". Inside it the player plugins and right-click menu entries run
`flatpak run …`, and updates are left to Flatpak (the app says so).

Publishing on Flathub (one-time, by hand): fork <https://github.com/flathub/flathub>, branch from
`new-pr`, add the manifest with the app source as `type: git` + the release tag and commit, and the
generated `cargo-sources.json`, then open the pull request; Flathub reviewers check it. Flathub
builds without the provider keys, so that build needs the users' own OpenSubtitles key until a
key policy for Flathub is decided. After acceptance, new releases are pull requests to
`flathub/io.github.halilkhrmn.SubMagician` (its bot opens them for new tags).

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
