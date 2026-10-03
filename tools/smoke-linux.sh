#!/usr/bin/env bash
# Installs a package on a clean system and starts it: `smoke-linux.sh deb|appimage <file>`.
# The window must stay up for a few seconds and the command-line tool must answer.
set -euo pipefail
kind=$1
file=$(ls $2 | head -1)
export SLINT_BACKEND=winit-software

case "$kind" in
deb)
    apt-get install -y "./$file"
    app=(submagician)
    cli=(submagician-cli)
    ;;
appimage)
    chmod 755 "$file"
    app=("./$file" --appimage-extract-and-run)
    cli=("./$file" --appimage-extract-and-run --cli)
    ;;
*)
    echo "unknown kind $kind" >&2
    exit 2
    ;;
esac

"${cli[@]}" --version
"${cli[@]}" --help | grep -q -- "--player"

# The window: alive after 8 s means it started (a missing library ends it at once).
set +e
timeout 8 xvfb-run -a "${app[@]}"
code=$?
set -e
if [ "$code" -ne 124 ]; then
    echo "the window exited with $code instead of staying up" >&2
    exit 1
fi
echo "smoke test passed: $kind"
