#!/usr/bin/env bash
# Builds release binaries and the Linux packages into target/linux/:
#   submagician_<version>-1_amd64.deb and SubMagician-<version>-x86_64.AppImage
# Needs cargo-deb (cargo install cargo-deb), dpkg-dev, file, and network access the first time
# (appimagetool is downloaded into target/). Provider keys come from
# SUBMAGICIAN_OPENSUBTITLES_API_KEY / SUBMAGICIAN_SUBDL_API_KEY.
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out=target/linux
rm -rf "$out"
mkdir -p "$out"

cargo build --release -p submagician -p submagician-cli
cargo deb -p submagician --no-build --output "$out/"

# AppImage: the app and the command-line tool in one file. `SubMagician.AppImage --cli …` runs
# the tool (the player plugins and the worker process use that, see crates/core/src/players.rs).
appdir=target/AppDir
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/256x256/apps"
cp target/release/submagician target/release/submagician-cli "$appdir/usr/bin/"
cp packaging/linux/submagician.desktop "$appdir/"
cp packaging/linux/submagician.desktop "$appdir/usr/share/applications/"
cp crates/app/assets/icon.png "$appdir/submagician.png"
cp crates/app/assets/icon.png "$appdir/usr/share/icons/hicolor/256x256/apps/submagician.png"
cat > "$appdir/AppRun" <<'APPRUN'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
if [ "$1" = "--cli" ]; then
    shift
    exec "$HERE/usr/bin/submagician-cli" "$@"
fi
exec "$HERE/usr/bin/submagician" "$@"
APPRUN
chmod 755 "$appdir/AppRun"

tool=${APPIMAGETOOL:-target/appimagetool-x86_64.AppImage}
if [ ! -x "$tool" ]; then
    curl -fsSL -o "$tool" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
    chmod 755 "$tool"
fi
# --appimage-extract-and-run: works without FUSE (CI runners, containers).
ARCH=x86_64 "$tool" --appimage-extract-and-run "$appdir" "$out/SubMagician-$version-x86_64.AppImage"

ls -la "$out"
