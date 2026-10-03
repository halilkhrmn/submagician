#!/usr/bin/env bash
# Builds the Flatpak bundle target/flatpak/SubMagician-<version>-x86_64.flatpak from this folder.
# Needs flatpak, flatpak-builder, python3 with aiohttp and tomlkit (for flatpak-cargo-generator)
# and network for the runtimes and crates. CI runs it in the flathub-infra container
# (.github/workflows/packaging.yml, release.yml). Provider keys come from
# SUBMAGICIAN_OPENSUBTITLES_API_KEY / SUBMAGICIAN_SUBDL_API_KEY, like the other packages.
set -euo pipefail
cd "$(dirname "$0")/.."

id=io.github.halilkhrmn.SubMagician
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
work=target/flatpak
mkdir -p "$work"

# Every crate in Cargo.lock as a Flatpak source.
generator=$work/flatpak-cargo-generator.py
if [ ! -f "$generator" ]; then
    curl -sSfL -o "$generator" \
        https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
fi
python3 "$generator" Cargo.lock -o packaging/flatpak/cargo-sources.json

# The manifest with the provider keys, kept out of the repository.
manifest=packaging/flatpak/$id.build.yml
python3 - "$manifest" <<'PY'
import os, sys
src = open("packaging/flatpak/io.github.halilkhrmn.SubMagician.yml").read()
keys = {k: os.environ[k] for k in ("SUBMAGICIAN_OPENSUBTITLES_API_KEY", "SUBMAGICIAN_SUBDL_API_KEY") if os.environ.get(k)}
anchor = '        WHISPER_DONT_GENERATE_BINDINGS: "1"\n'
assert anchor in src
extra = "".join(f'        {k}: "{v}"\n' for k, v in keys.items())
open(sys.argv[1], "w").write(src.replace(anchor, anchor + extra))
PY
trap 'rm -f "$manifest"' EXIT

flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak-builder --user --force-clean --disable-rofiles-fuse --install-deps-from=flathub \
    --repo="$work/repo" --state-dir="$work/state" "$work/build" "$manifest"
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    "$work/repo" "$work/SubMagician-$version-x86_64.flatpak" "$id"
echo "built $work/SubMagician-$version-x86_64.flatpak"
