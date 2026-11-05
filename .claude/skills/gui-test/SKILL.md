---
name: gui-test
description: Run CoCAD GUI screenshots, click tests or MCP tests on a private display. Use when a task needs the app window, xdotool or `cocad mcp`.
trimmedAt: 2026-11-05
---

# GUI tests

## Files

- `scripts/tour-shots.sh`: a full example. It runs the app on `:9` and drives it with MCP and `xdotool`.
- `src/mcp.rs`: `cocad mcp`. It finds a window through `mcp/<pid>.json` in the settings folder.
- `src/app/tour.rs`: the welcome tour and its `tour_seen` marker.

## Procedures

### Screenshot of the user's window

1. Start the app with `--slice` or `--photo` when you need that view.
2. Capture the window. Do not send input to it.

### Click or MCP test on a private display

1. Make private settings folders:

   ```bash
   export XDG_CONFIG_HOME=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d)
   mkdir -p "$XDG_CONFIG_HOME/cocad"
   touch "$XDG_CONFIG_HOME/cocad/tour_seen"
   ```

2. Start the display: `Xephyr :9 -screen 1600x1040 -ac &`.
3. Start the app: `DISPLAY=:9 LIBGL_ALWAYS_SOFTWARE=1 target/debug/cocad model.scad &`.
4. Send clicks and keys with `DISPLAY=:9 xdotool`.
5. For MCP, run `target/debug/cocad mcp` in the same shell. The `mcp()` function in `scripts/tour-shots.sh` shows the JSON-RPC calls.
6. Stop the app and Xephyr when you finish.

## Rules

1. Do not send mouse or key input to the user's desktop. Send input to `:9` only.
2. Use a separate `XDG_CONFIG_HOME` and `XDG_DATA_HOME`. The user's settings, recent list and window state then stay as they are.
3. Run `cocad mcp` with the same private `XDG_CONFIG_HOME` as the app. If not, the bridge finds the user's window.
4. Create `cocad/tour_seen` in the private `XDG_CONFIG_HOME`. Without it, the welcome tour opens and covers the window.
