#!/usr/bin/env bash
# Makes the app screenshots of the website: extras/website/images/screen-*.jpg,
# 1600 x 1040, dark theme. Runs the app on a private display with private
# settings, and drives it through MCP and xdotool. The window of the user is
# not touched. After it, run scripts/website-assets.py --images-only.
# Needs Xephyr, xdotool, ImageMagick and openscad-nightly. The render shot
# takes some minutes.
#
# Usage: scripts/website-shots.sh [NAME...]
#   NAME: only these shots, for example "fixes render". The names are the
#   file names without "screen-": model menu optimize fixes fixinfo layers
#   export sets agent render.
set -euo pipefail
cd "$(dirname "$0")/.."
only=" $* "

# want NAME...: true when one of the shots NAME is to be made.
want() {
    [ "$only" = "  " ] && return 0
    local n
    for n in "$@"; do
        [[ "$only" == *" $n "* ]] && return 0
    done
    return 1
}

cargo build --quiet
bin=$PWD/target/debug/cocad
out=$PWD/extras/website/images
disp=:39
width=1600
height=1040

work=$(mktemp -d)
models=$work/models
export XDG_CONFIG_HOME=$work/config XDG_DATA_HOME=$work/data
mkdir -p "$models" "$XDG_CONFIG_HOME/cocad" "$out"
cp examples/*.scad examples/*.yml "$models/"
touch "$XDG_CONFIG_HOME/cocad/tour_seen"
for m in desk-dock gear knob vase honeycomb-tray demo-box; do
    echo "$models/$m.scad"
done > "$XDG_CONFIG_HOME/cocad/recent"

# config CARDS: the settings file, with the cards CARDS (comma list) closed.
# The app reads it at the start.
config() {
    printf 'theme = dark\nclosed_cards = %s\n' "$1" > "$XDG_CONFIG_HOME/cocad/config"
}

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

# start ARGS: starts the app and waits for the build.
start() {
    "$bin" "$@" >/dev/null 2>&1 &
    app=$!
    sleep 3
    xdotool search --name 'CoCAD' windowmove 0 0 windowsize "$width" "$height" windowfocus
    sleep 5
}

quit() {
    kill "$app" 2>/dev/null || true
    wait "$app" 2>/dev/null || true
    app=
}

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

# The pointer goes to the empty part of the menu bar, so that no tooltip
# or hover shows.
park() {
    xdotool mousemove 900 17
}

# The desk dock in the Optimize tab: all print fixes, Auto orientation (the
# default), the split that CoCAD proposes for the base plate, and a 250 x 220
# bed. On a 220 x 220 bed the parts with the mouse ears need more room than
# the bed. The app stores the values with the model.
dock() {
    mcp set_optimize '{"area":[250,220,270],"fixes":{"teardrops":true,"hole_size":true,"bridges":true,"chamfer":true,"brim":true,"ears":true},"parts":{"Base plate":{"split":{"joint":"dovetail","cuts":[[0,130]],"clearance":0.2}}}}' >/dev/null
    mcp set_view '{"view":"iso","fit":true}' >/dev/null
}

# The desk dock in the Model tab, then the File menu with the recent models.
if want model menu; then
    config ""
    start "$models/desk-dock.scad"
    mcp set_view '{"mode":"model","view":"iso","fit":true}' >/dev/null
    park
    if want model; then
        shot screen-model 3
    fi
    if want menu; then
        xdotool key alt+f
        shot screen-menu 2
        xdotool key Escape
    fi
    quit
fi

if want optimize; then
    config ""
    start --optimize "$models/desk-dock.scad"
    dock
    park
    shot screen-optimize 5
    quit
fi

# The Print fixes card. First the info window of Mouse ears: the info
# button after the name of the last fix. Then the card, with the panel
# scrolled down so that the Check card shows too.
if want fixes fixinfo; then
    config "Plate,Orientation,Layers,Printer,Print settings"
    start --optimize "$models/desk-dock.scad"
    dock
    sleep 4
    if want fixinfo; then
        xdotool mousemove 1398 769 sleep 0.5 click 1
        park
        shot screen-fixinfo 2
        xdotool key Escape
    fi
    if want fixes; then
        xdotool mousemove 1440 600 click 5 click 5 click 5
        park
        shot screen-fixes 2
    fi
    quit
fi

# The Layers view, with the Layers card open.
if want layers; then
    config "Plate,Orientation,Print fixes,Printer,Print settings"
    start --optimize "$models/desk-dock.scad"
    dock
    mcp set_view '{"layers":true}' >/dev/null
    sleep 5
    mcp set_view '{"layer":120}' >/dev/null
    park
    shot screen-layers 4
    quit
fi

# Export: the lid box in the Optimize tab, with the menu of the arrow at
# the right of Export 3MF open.
if want export; then
    config ""
    start --optimize "$models/lid-box.scad"
    mcp set_view '{"view":"iso","fit":true}' >/dev/null
    sleep 3
    xdotool mousemove $((width - 39)) $((height - 60)) sleep 0.5 click 1
    park
    shot screen-export 2
    xdotool key Escape
    quit
fi

# Parameter sets: a set of the gear from gear.yml, and the list open.
if want sets; then
    config ""
    start "$models/gear.scad"
    mcp load_parameter_set '{"name":"Large wheel"}' >/dev/null
    mcp set_view '{"view":"iso","fit":true}' >/dev/null
    sleep 2
    # The list of the sets, at the top of the left panel.
    xdotool mousemove 95 167 sleep 0.5 click 1
    xdotool mousemove 900 600
    shot screen-sets 2
    xdotool key Escape
    quit
fi

# Agent control: the message in the view names the change.
if want agent; then
    config ""
    start "$models/vase.scad"
    mcp set_params '{"values":{"sides":5,"twist":240,"taper":0.8}}' >/dev/null
    mcp set_view '{"view":"iso","fit":true}' >/dev/null
    park
    shot screen-agent 1
    quit
fi

# Render: the gear, with all samples done.
if want render; then
    config ""
    start --render "$models/gear.scad"
    mcp set_view '{"mode":"render","view":"iso","fit":true,"render":true}' >/dev/null
    for _ in $(seq 1 120); do
        sleep 10
        left=$(mcp get_state '{}' | python3 -c '
import json, sys
r = json.load(sys.stdin)
d = json.loads(r["result"]["content"][0]["text"])["render"]
print(d["of"] - d["samples"])')
        [ "$left" -le 0 ] && break
    done
    park
    shot screen-render 3
    quit
fi
