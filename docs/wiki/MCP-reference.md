# MCP reference

*Advanced topic. This page is for technical readers. For a simple introduction, see [AI assistant](AI-assistant).*

CoCAD has an MCP server, so that an AI client can work with the open window. The client can read the model and its parameters, change values, see the result as an image, and run app commands.

## Setup

Add the server to the client. For Claude Desktop, edit `claude_desktop_config.json` and restart Claude Desktop:

```json
{
  "mcpServers": {
    "cocad": {
      "command": "cocad",
      "args": ["mcp"]
    }
  }
}
```

For Claude Code:

```bash
claude mcp add cocad -- "cocad" mcp
```

Use the full path of the binary when it is not on the `PATH` of the client:

- Windows: `cocad.exe` in the folder where you unpacked the zip, with double backslashes in the JSON.
- macOS: `CoCAD.app/Contents/MacOS/cocad` in Applications.

The step **Connect an agent** of the welcome tour shows both texts with the path of the running program.

## How it works

The client starts `cocad mcp`. This process has no window. It sends each tool call to the open window, and you see the changes there. `open_model` starts the app when it is not running.

## Tools

| Tool | Action |
|---|---|
| `get_state` | Loaded file, render status, last error, mesh data, changed parameters, parameter sets, parts, part colors, explode, section cut, end of the OpenSCAD log |
| `get_params` | All Customizer parameters with type, value, default, limits and options |
| `get_source` | Source of the `.scad` file |
| `view_model` | Render an image and return it to the client |
| `open_model` | Open a `.scad` file |
| `set_params`, `reset_params` | Change parameters or reset them, then render |
| `render` | Run OpenSCAD again |
| `load_parameter_set`, `save_parameter_set` | Load or save a parameter set |
| `export_model` | Write STL, 3MF or STEP. With `slicer`, write a multicolor 3MF |
| `save_photo` | Write a path traced PNG |
| `slice` | Slice and return layers, filament, weight and a rough print time |
| `set_view` | Change the tab, camera direction, wireframe, grid, part colors, explode, section cut and slice layer |
| `list_windows`, `select_window` | Choose a window when more than one is open |

## Behavior

### Parameters and renders

- `set_params` uses the limits of the Customizer panel. One bad value rejects the whole call.
- Calls wait for the render, for at most 45 seconds. Then they return the state with status `rendering`, or an error.

### Files

- `export_model` and `save_photo` do not replace a file without `overwrite`.
- `export_model`, `save_photo` and `view_model` fail when the last render failed, so that they do not use an old mesh.
- `open_model` opens only `.scad` files. `save_photo` writes only `.png` files.
- `export_model` and `save_photo` do not write through a symbolic link.
- The tools cannot delete a parameter set or a file.

### Images

- `view_model` and `save_photo` put the whole model in the frame. They use the camera direction of the window, or the given `view`.
- `view_model` and `save_photo` show the part colors, the explode and the section cut of the window, as the **Photo** tab does. When the part colors or the explode are on and CoCAD still finds the parts, they wait for the parts, for at most 45 seconds.
- `save_photo` stops after 40 seconds and saves the samples it has.

### `set_view`: explode

- `explode` sets the explode distance in mm. 0 turns the explode off. A distance above the end of the slider uses the end of the slider.
- `explode_axis` sets the direction (`all`, `x`, `y` or `z`) and turns the explode on.

### `set_view`: section

`section` sets the section cut, for example:

```json
{"x": {"at": 12.5, "keep": "+"}}
```

- The object replaces all cuts. An axis that is missing or `null` has no cut. `{}` turns all cuts off.
- `at` is the position in mm. The default is the center of the model. A position out of the slider range uses the end of the range.
- `keep` is `+` (keep the side above the position) or `-` (keep the side below). The default is `+`.
- `get_state` shows the cuts in the same form.

### Window

- A minimized window may not answer until you restore it.

## Security

- Each window listens on a free port of `127.0.0.1` and accepts only calls with a random token.
- The token is in `mcp/<pid>.json` in the [settings folder](OpenSCAD-engine-and-files#the-settings-folder), which only your user can read.
- Any program that runs as your user can read that file and control the window, and write files through the export tools.

The server is on by default. To turn it on or off, use **MCP server** in **File > Settings**. The change applies when you save. Other open windows change at their next start.

## Analytics

If usage analytics are on, `open_model` and a `set_view` call that changes the `mode` send events. The other tools send no event. The `cocad mcp` process itself sends nothing. See [Privacy](Privacy#when-an-ai-assistant-is-connected).
