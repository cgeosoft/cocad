#!/usr/bin/env bash
# Makes the screenshots of the Optimize guide in assets/guide/: the desk dock
# example (examples/desk-dock.scad) before and after the print preparation.
# Runs the app on a private display with private settings, and drives it
# through MCP and xdotool. The window of the user is not touched.
# Needs Xephyr, xdotool, ImageMagick and openscad-nightly.
#
# Usage: scripts/guide-shots.sh
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --quiet
bin=$PWD/target/debug/cocad
out=$PWD/assets/guide
disp=:19
width=1400
height=900

work=$(mktemp -d)
export XDG_CONFIG_HOME=$work/config XDG_DATA_HOME=$work/data
mkdir -p "$XDG_CONFIG_HOME/cocad" "$work/models" "$out"
cp examples/desk-dock.scad "$work/models/"
touch "$XDG_CONFIG_HOME/cocad/tour_seen"
echo 'theme = dark' > "$XDG_CONFIG_HOME/cocad/config"

xephyr=
app=
stop() {
    [ -n "$app" ] && kill "$app" 2>/dev/null || true
    [ -n "$xephyr" ] && kill "$xephyr" 2>/dev/null || true
    rm -rf "$work"
}
trap stop EXIT

# Another program on the same display would get the app and the clicks.
if [ -e "/tmp/.X11-unix/X${disp#:}" ]; then
    echo "Display $disp is in use. Stop the program that uses it, then run again." >&2
    exit 1
fi
Xephyr "$disp" -screen "${width}x${height}" -ac -br -noreset 2>/dev/null &
xephyr=$!
sleep 1
export DISPLAY=$disp LIBGL_ALWAYS_SOFTWARE=1

# mcp TOOL JSON_ARGS: prints the reply
mcp() {
    printf '%s\n' \
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" \
        | "$bin" mcp | tail -1
}

# shot NAME [SETTLE_SECONDS]
shot() {
    sleep "${2:-2}"
    import -window root -quality 85 "$out/$1.jpg"
    echo "$1.jpg"
}

"$bin" --optimize "$work/models/desk-dock.scad" >/dev/null 2>&1 &
app=$!
sleep 3
xdotool search --name 'CoCAD' windowmove 0 0 windowsize "$width" "$height" windowfocus
# The build, the parts and the plates.
sleep 8

mcp set_view '{"mode":"model","view":"iso","fit":true}' >/dev/null
shot app-1-model 3
mcp set_view '{"mode":"optimize"}' >/dev/null
sleep 3
before=$(mcp get_optimize '{}')
shot app-2-before
mcp set_optimize '{"fixes":{"teardrops":true,"bridges":true,"chamfer":true,"brim":true}}' >/dev/null
shot app-3-fixes 4
# Accept the split that CoCAD proposes for the base plate.
split=$(printf '%s' "$before" | python3 -c '
import json, sys
r = json.load(sys.stdin)
d = json.loads(r["result"]["content"][0]["text"])
print(json.dumps(next(p["split_proposal"] for p in d["parts"] if p.get("split_proposal"))))')
mcp set_optimize "{\"parts\":{\"Part 1\":{\"split\":$split}}}" >/dev/null
shot app-5-split 5
mcp set_view '{"view":"top","fit":true}' >/dev/null
shot app-6-top 3
mcp set_view '{"view":"iso","fit":true}' >/dev/null
# A smaller printer: two plates.
mcp set_optimize '{"area":[180,180,180],"plate":1}' >/dev/null
sleep 4
# Open the menu of the Export card with the items for all plates: the arrow
# is the narrow button at the right of Export 3MF. Then move the pointer off
# the buttons, so that no tooltip shows. The next click closes the menu.
xdotool mousemove $((width - 54)) $((height - 80)) click 1
xdotool mousemove 700 17
shot app-7-two-plates 2
mcp set_optimize '{"area":[250,220,270],"plate":0}' >/dev/null
sleep 3
# The Layers switch of the view.
xdotool mousemove 471 74 click 1
sleep 2
mcp set_view '{"layer":120}' >/dev/null
shot app-8-layers 4
