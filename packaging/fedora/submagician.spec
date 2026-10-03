# Fedora / COPR package, built from source with vendored crates (no network during the build).
# packaging/fedora/make-srpm.sh fills in Version and makes the source archives.

# Release builds are stripped; there is nothing for debuginfo.
%global debug_package %{nil}

Name:           submagician
Version:        0.0.0
Release:        1%{?dist}
Summary:        Finds, fixes and syncs subtitles for your videos
# SubMagician itself; the bundled Rust crates and whisper.cpp are MIT / Apache-2.0 / BSD / ISC /
# Zlib / MPL-2.0 / Unicode.
License:        AGPL-3.0-only
URL:            https://github.com/halilkhrmn/submagician
Source0:        %{name}-%{version}.tar.gz
Source1:        %{name}-%{version}-vendor.tar.xz
# Build settings: provider application keys when the SRPM was made with them (CI), else empty.
Source2:        %{name}-build.env

ExclusiveArch:  x86_64 aarch64

BuildRequires:  cargo >= 1.88
BuildRequires:  rust >= 1.88
BuildRequires:  gcc
BuildRequires:  gcc-c++
BuildRequires:  cmake
BuildRequires:  clang-devel
BuildRequires:  pkgconfig(fontconfig)
BuildRequires:  pkgconfig(xkbcommon)
BuildRequires:  pkgconfig(wayland-client)
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

Requires:       hicolor-icon-theme
Requires:       libxkbcommon-x11
# Syncing to the audio and reading subtitle tracks inside videos: ffmpeg-free from Fedora, or
# ffmpeg from RPM Fusion.
Recommends:     /usr/bin/ffmpeg
Recommends:     /usr/bin/ffprobe

%description
SubMagician finds subtitles for a folder of videos on OpenSubtitles, SubDL and Addic7ed, picks
the one made for each file, fixes its text encoding, syncs it to the audio and saves it next to
the video. When no source has one, it writes it from the audio with Whisper on this computer.
Comes with a command-line tool and plugins for mpv and VLC.

%prep
%autosetup -n %{name}-%{version}
tar -xJf %{SOURCE1}
mkdir -p .cargo
# Keeps the settings of the repository's own .cargo/config.toml (GGML_NATIVE=OFF).
cat >> .cargo/config.toml <<'CARGO'

[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
CARGO

%build
set -a
. %{SOURCE2}
set +a
# Updates come from dnf; the app says so instead of offering a download.
export SUBMAGICIAN_PACKAGER=fedora-copr
# whisper.cpp for any x86-64 CPU since 2013, not the build machine's (docs/DECISIONS.md #52).
export GGML_NATIVE=OFF
cargo build --release --locked --offline -p submagician -p submagician-cli

%install
install -Dm755 target/release/submagician %{buildroot}%{_bindir}/submagician
install -Dm755 target/release/submagician-cli %{buildroot}%{_bindir}/submagician-cli
install -Dm644 packaging/linux/submagician.desktop %{buildroot}%{_datadir}/applications/submagician.desktop
install -Dm644 crates/app/assets/icon.png %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/submagician.png
install -Dm644 crates/app/assets/icon.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/submagician.svg
# The same description as the Flatpak, pointing at this package's launcher.
install -d %{buildroot}%{_metainfodir}
sed 's|<launchable type="desktop-id">.*</launchable>|<launchable type="desktop-id">submagician.desktop</launchable>|' \
    packaging/linux/io.github.halilkhrmn.SubMagician.metainfo.xml \
    > %{buildroot}%{_metainfodir}/io.github.halilkhrmn.SubMagician.metainfo.xml

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/submagician.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/io.github.halilkhrmn.SubMagician.metainfo.xml
%{buildroot}%{_bindir}/submagician-cli --version

%files
%license LICENSE
%doc README.md
%{_bindir}/submagician
%{_bindir}/submagician-cli
%{_datadir}/applications/submagician.desktop
%{_datadir}/icons/hicolor/256x256/apps/submagician.png
%{_datadir}/icons/hicolor/scalable/apps/submagician.svg
%{_metainfodir}/io.github.halilkhrmn.SubMagician.metainfo.xml

%changelog
# Filled in by make-srpm.sh from changelog/en.md.
