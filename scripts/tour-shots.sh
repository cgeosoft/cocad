#!/usr/bin/env bash
# Makes the screenshots of the welcome tour in images/tour/.
# Runs the app on a private display with private settings, and drives it through
# MCP and xdotool. The window of the user is not touched.
# Needs Xephyr, xdotool, ImageMagick and OpenSCAD.
#
# Usage: scripts/tour-shots.sh
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --quiet
bin=$PWD/target/debug/scad-viewer
out=$PWD/images/tour
disp=:9
width=1120
height=700

work=$(mktemp -d)
models=$work/models
export XDG_CONFIG_HOME=$work/config XDG_DATA_HOME=$work/data
mkdir -p "$models" "$XDG_CONFIG_HOME/scad-viewer" "$out"
cp examples/*.scad "$models/"
# The tour must not cover the app in its own screenshots.
touch "$XDG_CONFIG_HOME/scad-viewer/tour_seen"
echo 'theme = dark' > "$XDG_CONFIG_HOME/scad-viewer/config"
for m in vase gear knob honeycomb-tray; do
    echo "$models/$m.scad"
done > "$XDG_CONFIG_HOME/scad-viewer/recent"

xephyr=
app=
stop() {
    [ -n "$app" ] && kill "$app" 2>/dev/null || true
    [ -n "$xephyr" ] && kill "$xephyr" 2>/dev/null || true
    rm -rf "$work"
}
trap stop EXIT

Xephyr "$disp" -screen "${width}x${height}" -ac -br -noreset 2>/dev/null &
xephyr=$!
sleep 1
export DISPLAY=$disp LIBGL_ALWAYS_SOFTWARE=1

start() {
    "$bin" "$@" &
    app=$!
    sleep 2
    xdotool search --name 'SCAD Viewer' windowmove 0 0 windowsize "$width" "$height" windowfocus
    sleep 1
}

quit() {
    kill "$app" 2>/dev/null || true
    wait "$app" 2>/dev/null || true
    app=
}

# mcp TOOL JSON_ARGS
mcp() {
    printf '%s\n' \
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" \
        | "$bin" mcp >/dev/null
}

# shot NAME [SETTLE_SECONDS]
shot() {
    sleep "${2:-1}"
    import -window root -quality 82 "$out/$1.jpg"
    echo "$1.jpg"
}

# Start view with the recent models.
start
shot 1-welcome
quit

# Customizer: the agent changes values as a user would with the sliders.
start "$models/gear.scad"
mcp set_params '{"values":{"teeth":18,"gear_module":2.5,"hub_height":10}}'
mcp set_view '{"view":"iso","fit":true}'
shot 2-customizer

# Parameter sets.
mcp save_parameter_set '{"name":"small drive gear"}'
mcp set_params '{"values":{"teeth":40,"gear_module":1.5}}'
mcp save_parameter_set '{"name":"large idler"}'
mcp set_view '{"fit":true}'
shot 3-sets
quit

# Mesh data and export.
start "$models/knob.scad"
mcp set_view '{"wireframe":true,"view":"iso","fit":true}'
shot 4-export
quit

# Slice preview.
start --slice "$models/honeycomb-tray.scad"
mcp set_view '{"view":"iso","fit":true,"layer":60}'
shot 5-slice 2

# Photo render.
quit
start --photo "$models/gear.scad"
shot 6-photo 60
quit

# Agent control: the message in the view names the change.
start "$models/vase.scad"
mcp set_params '{"values":{"sides":5,"twist":240,"taper":0.8}}'
mcp set_view '{"view":"iso","fit":true}'
shot 7-agent

# Settings with the MCP server card.
xdotool key ctrl+comma
shot 8-connect
xdotool key Escape
sleep 0.5

# Help menu with the tour item.
xdotool key alt+h
shot 9-menu
