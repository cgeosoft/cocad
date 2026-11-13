#!/usr/bin/env bash
# Makes the screenshots of the welcome tour in assets/tour/.
# Runs the app on a private display with private settings, and drives it through
# MCP and xdotool. The window of the user is not touched.
# Needs Xephyr, xdotool, ImageMagick and OpenSCAD.
#
# Usage: scripts/tour-shots.sh
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --quiet
bin=$PWD/target/debug/cocad
out=$PWD/assets/tour
disp=:9
width=1120
height=700

work=$(mktemp -d)
models=$work/models
export XDG_CONFIG_HOME=$work/config XDG_DATA_HOME=$work/data
mkdir -p "$models" "$XDG_CONFIG_HOME/cocad" "$out"
cp examples/*.scad examples/*.yml "$models/"
# The tour must not cover the app in its own screenshots.
touch "$XDG_CONFIG_HOME/cocad/tour_seen"
echo 'theme = dark' > "$XDG_CONFIG_HOME/cocad/config"
for m in vase gear knob honeycomb-tray; do
    echo "$models/$m.scad"
done > "$XDG_CONFIG_HOME/cocad/recent"

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

start() {
    "$bin" "$@" &
    app=$!
    sleep 2
    xdotool search --name 'CoCAD' windowmove 0 0 windowsize "$width" "$height" windowfocus
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

# Thumbnails of the recent models: open each model once. The last model gets
# its thumbnail after its first build.
start "$models/honeycomb-tray.scad"
for m in knob gear vase; do
    mcp open_model "{\"path\":\"$models/$m.scad\",\"wait\":true}"
done
sleep 3
quit
# The opens changed the order of the recent list. Write it again.
for m in vase gear knob honeycomb-tray; do
    echo "$models/$m.scad"
done > "$XDG_CONFIG_HOME/cocad/recent"

# Start page: new model, open, agent card, and the Recent tab.
start
# The pointer must not show a tooltip in the shot.
xdotool mousemove $((width - 10)) $((height - 60))
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

# Export: Export 3MF at the bottom of the Optimize tab, with the menu of
# the arrow open. The arrow is the narrow button at the right of Export 3MF.
start --optimize "$models/lid-box.scad"
mcp set_view '{"view":"iso","fit":true}'
sleep 3
xdotool mousemove 565 360 click 4 click 4 click 4
sleep 1
xdotool mousemove $((width - 39)) $((height - 60)) click 1
# Move the pointer off the buttons, so that no tooltip shows. The menu stays
# open. Then put it back in the middle for the next shots.
xdotool mousemove 600 17
shot 4-export
xdotool mousemove $((width / 2)) $((height / 2))
quit

# Parts: colors, explode and a section cut. The parts job runs after the render.
start "$models/lid-box.scad"
sleep 2
mcp set_view '{"part_colors":true,"explode":24,"explode_axis":"z","section":{"y":{"keep":"+"}},"view":"iso","fit":true}'
shot 5-parts 3
quit

# Optimize tab: the desk dock on the plate, with the print fixes on, the
# clips flat on the bed (Auto) and the base plate split in two pieces.
start --optimize "$models/desk-dock.scad"
mcp set_optimize '{"fixes":{"chamfer":true,"teardrops":true,"bridges":true,"brim":true},"parts":{"Cable clip 1":{"orient":"auto"},"Base plate":{"split":{"joint":"dovetail","cuts":[[0,130]],"clearance":0.2}}}}'
mcp set_view '{"view":"iso","fit":true}'
shot 6-slice 3

# Photo render.
quit
start --render "$models/gear.scad"
# The Render tab starts no render by itself: start it as the Render button
# does. The mode is also set, because the model can have a stored tab.
mcp set_view '{"mode":"render","render":true}'
shot 7-photo 60
quit

# Agent control: the message in the view names the change.
start "$models/vase.scad"
mcp set_params '{"values":{"sides":5,"twist":240,"taper":0.8}}'
mcp set_view '{"view":"iso","fit":true}'
shot 8-agent

# Settings on the MCP server page (the fifth page in the list).
xdotool key ctrl+comma
sleep 1
xdotool mousemove 300 334 click 1
shot 9-connect
xdotool key Escape
sleep 0.5

# Help menu with the tour item.
xdotool key alt+h
shot 10-menu
