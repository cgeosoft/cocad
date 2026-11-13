#!/usr/bin/env bash
# Makes the built-in thumbnails of the examples in assets/examples/.
# The start page shows them until the user opens the example. Then the
# thumbnail that CoCAD makes in the settings folder replaces it.
# Runs the app on a private display with private settings and opens each
# example through MCP. The window of the user is not touched.
# Needs Xephyr, ImageMagick and OpenSCAD.
#
# Usage: scripts/example-thumbs.sh
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --quiet
bin=$PWD/target/debug/cocad
out=$PWD/assets/examples
disp=:9
size=256

work=$(mktemp -d)
models=$work/models
export XDG_CONFIG_HOME=$work/config XDG_DATA_HOME=$work/data
thumbs=$XDG_CONFIG_HOME/cocad/thumbs
mkdir -p "$models" "$XDG_CONFIG_HOME/cocad" "$thumbs" "$out"
cp -r examples/. "$models/"
touch "$XDG_CONFIG_HOME/cocad/tour_seen"

xephyr=
app=
stop() {
    [ -n "$app" ] && kill "$app" 2>/dev/null || true
    [ -n "$xephyr" ] && kill "$xephyr" 2>/dev/null || true
    rm -rf "$work"
}
trap stop EXIT

Xephyr "$disp" -screen 1120x700 -ac -br -noreset 2>/dev/null &
xephyr=$!
sleep 1
export DISPLAY=$disp LIBGL_ALWAYS_SOFTWARE=1

"$bin" &
app=$!
sleep 2

# mcp TOOL JSON_ARGS
mcp() {
    printf '%s\n' \
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" \
        | "$bin" mcp >/dev/null
}

# The example files of the File menu, from src/examples.rs.
files=$(sed -n '/^pub const MENU/,/^];/p' src/examples.rs | grep -o '"[a-z0-9-]*\.scad"' | tr -d '"')
for f in $files; do
    before=$(ls "$thumbs")
    mcp open_model "{\"path\":\"$models/$f\",\"wait\":true}"
    # The thumbnail comes after the part search. Wait for a new file.
    new=
    for _ in $(seq 60); do
        sleep 0.5
        new=$(comm -13 <(echo "$before") <(ls "$thumbs") | grep '\.png$' || true)
        [ -n "$new" ] && break
    done
    if [ -z "$new" ]; then
        echo "no thumbnail for $f" >&2
        exit 1
    fi
    sleep 0.5
    convert "$thumbs/$new" -resize "${size}x${size}" -strip "PNG32:$out/${f%.scad}.png"
    echo "${f%.scad}.png"
done
