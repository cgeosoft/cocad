#!/usr/bin/env bash
# Makes the before and after pictures of the Optimize fixes on
# examples/desk-dock.scad in assets/guide/, and the pictures of the fix info
# windows in assets/fixes/. Needs openscad-nightly and
# ImageMagick.
#
# Usage: scripts/showcase-shots.sh
set -euo pipefail
cd "$(dirname "$0")/.."
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
COCAD_SHOWCASE_OUT="$work/stl" cargo test --quiet showcase -- --ignored --test-threads=1
python3 -I scripts/showcase-shots.py "$work/stl" "$work/out"
mkdir -p assets/guide
for f in "$work"/out/*.png; do
    convert "$f" -strip "assets/guide/$(basename "$f")"
done
# The pictures of the fix info windows, which the binary embeds.
mkdir -p assets/fixes
for f in "$work"/out/app/*.png; do
    convert "$f" -resize '1400x>' -strip "assets/fixes/$(basename "$f")"
done
ls -l assets/guide assets/fixes
