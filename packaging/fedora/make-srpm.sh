#!/bin/sh
# Builds submagician-<version>-1.src.rpm into the directory given as $1 (default target/srpm):
# the source tree of a release tag plus every crate vendored, so the RPM build itself needs no
# network. The release workflow runs it (with the provider keys) and sends the SRPM to Fedora
# COPR; COPR's own "make_srpm" method runs it through .copr/Makefile (without keys).
#
# Builds the newest v* tag, or SUBMAGICIAN_REF (any commit, branch or tag) when set. The provider
# keys come from SUBMAGICIAN_OPENSUBTITLES_API_KEY / SUBMAGICIAN_SUBDL_API_KEY when set.
set -eu
outdir=$(realpath -m "${1:-target/srpm}")
cd "$(dirname "$0")/../.."

ref=${SUBMAGICIAN_REF:-$(git tag --list 'v[0-9]*' --sort=-v:refname | head -n 1)}
ref=${ref:-HEAD}
version=$(git show "$ref:Cargo.toml" | sed -n 's/^version = "\(.*\)"/\1/p' | head -n 1)
echo "building submagician $version from $ref"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git archive --format=tar --prefix="submagician-$version/" "$ref" | tar -x -C "$work"
git archive --format=tar.gz --prefix="submagician-$version/" -o "$work/submagician-$version.tar.gz" "$ref"
(
    cd "$work/submagician-$version"
    cargo vendor --locked --quiet vendor > /dev/null
    # Windows- and macOS-only crates are not needed here; their bulk (prebuilt import
    # libraries) is left out. Cargo still wants the package, so its checksum file stays.
    for crate in vendor/windows_*_msvc* vendor/windows_*_gnu* vendor/windows_*_gnullvm*; do
        [ -d "$crate" ] || continue
        find "$crate" -name '*.lib' -o -name '*.a' | xargs -r rm -f
        sed -i 's/"files":{[^}]*}/"files":{}/' "$crate/.cargo-checksum.json"
    done
    tar -cJf "$work/submagician-$version-vendor.tar.xz" vendor
)

# Provider keys, when given: compiled into the binaries like in the other release packages.
{
    [ -n "${SUBMAGICIAN_OPENSUBTITLES_API_KEY:-}" ] && printf 'SUBMAGICIAN_OPENSUBTITLES_API_KEY=%s\n' "$SUBMAGICIAN_OPENSUBTITLES_API_KEY"
    [ -n "${SUBMAGICIAN_SUBDL_API_KEY:-}" ] && printf 'SUBMAGICIAN_SUBDL_API_KEY=%s\n' "$SUBMAGICIAN_SUBDL_API_KEY"
    true
} > "$work/submagician-build.env"

# Version, and a changelog entry from changelog/en.md for it.
date=$(LC_ALL=C git log -1 --format=%cd --date=format:'%a %b %d %Y' "$ref")
notes=$(git show "$ref:changelog/en.md" | awk -v v="## $version" '
    $0 == v { on = 1; next }
    on && /^## / { exit }
    on && /^- / { sub(/^- /, ""); if (line != "") print "- " line; line = $0; next }
    on && /^  / { sub(/^ +/, ""); line = line " " $0; next }
    END { if (line != "") print "- " line }')
[ -n "$notes" ] || notes="- Release notes: https://github.com/halilkhrmn/submagician/releases"
# A % in the notes would start an RPM macro.
notes=$(printf '%s\n' "$notes" | sed 's/%/%%/g')
sed -e "s/^Version:.*/Version:        $version/" -e '/^%changelog/,$d' packaging/fedora/submagician.spec > "$work/submagician.spec"
printf '%%changelog\n* %s Halil Kahraman <halilkahraman@yandex.com> - %s-1\n%s\n' "$date" "$version" "$notes" >> "$work/submagician.spec"

mkdir -p "$outdir"
rpmbuild -bs --define "_sourcedir $work" --define "_srcrpmdir $outdir" "$work/submagician.spec"
