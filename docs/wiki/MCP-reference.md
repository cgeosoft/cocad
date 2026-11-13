# MCP reference

*Advanced topic. This page is for technical readers. For a simple introduction, see [AI assistant](AI-assistant).*

CoCAD has an MCP server, so that an AI client can work with the open window. The client can read the model and its parameters, change values, see the result as an image, prepare the print plates, and run app commands.

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

The client starts `cocad mcp`. This process has no window. It sends each tool call to the open window, and you see the changes there. `open_model` and `new_model` start the app when it is not running.

## Tools

| Tool | Action |
|---|---|
| `get_state` | Loaded file, build status, last error, mesh data, changed parameters, parameter sets, the [model definition](Model-definition) (file, title, description, problems), tab, parts and reference parts, part colors, explode, section cut, the plates of the Optimize tab, end of the OpenSCAD log |
| `get_params` | All Customizer parameters with type, value, default, limits and options |
| `get_source` | Source of the `.scad` file |
| `view_model` | Render an image and return it to the client |
| `open_model` | Open a `.scad` file |
| `new_model` | Write a new `.scad` file and open it. Without `source`, the file gets a box with four parameters |
| `set_params`, `reset_params` | Change parameters or reset them, then build |
| `build` | Build the model: run OpenSCAD again. The name before 0.7, `render`, also works. |
| `load_parameter_set`, `save_parameter_set` | Load or save a parameter set. The sets are in `<model>.yml`. |
| `get_optimize` | The print preparation of the Optimize tab: the state and the result |
| `set_optimize` | Change the print preparation of the Optimize tab |
| `export_model` | Write a plate as STL or 3MF, or the model as STEP. With `slicer`, write a multicolor 3MF |
| `save_photo` | Write a path traced PNG |
| `slice` | Slice the current plate and return layers, filament, weight and a rough print time |
| `set_view` | Change the tab, camera direction, wireframe, grid, part colors, explode, section cut and the layer of the Optimize tab. Start or stop the render of the Render tab |
| `list_windows`, `select_window` | Choose a window when more than one is open |

## Behavior

### Parameters and builds

- `set_params` uses the limits of the Customizer panel. One bad value rejects the whole call.
- `set_params` builds the model by default. With `"build": false`, the app follows its **Auto build** switch. The name before 0.7, `render`, also works.
- Calls wait for the build, for at most 45 seconds. Then they return the state with status `building`, or an error.
- The status of `get_state` is one of `ready`, `building`, `needs build`, `failed`, `no model`, `idle` and `no openscad with manifold`.

### Optimize

`get_optimize` returns:

- `state`: the values of the Optimize tab, in the form that `set_optimize` takes.
- `parts`: one entry for each print part: `name`, `count`, `copies`, `fits`, `size_mm`, `plates`, `orientation`, `issues` (the problems of the Check card), `fixes` (the number of places that each fix changed, and the places that it skipped), and for a part that does not fit, `split_proposal`. A piece of a split part has `piece`.
- `plates`, `plate` (from 1), `summary` (the text that shows when you hold the mouse on **Export 3MF**), `errors` and `seconds`.

`set_optimize` merges the given keys into the state, as a JSON merge patch: objects merge key by key, and `null` removes a key. Then it waits for the plates and returns the result as `get_optimize` does.

| Key | Value |
|---|---|
| `area` | The print area `[width, depth, height]` in mm |
| `gap` | The gap between the parts on a plate, in mm |
| `only` | The name of one print part, or `null` for all parts |
| `plate` | The plate that the window and the export show, from 0 |
| `fixes` | `teardrops`, `bridges`, `hole_above` (`sacrificial` or `stepped`), `sacrificial_layers`, `chamfer`, `chamfer_mm`, `brim`, `brim_w`, `brim_gap`, `ears`, `ear_d`, `ear_min`, `hole_size`, `hole_add` |
| `parts` | The values of each print part, by name: `include`, `copies`, `orient` (`model`, `auto` or `face`), `down` (with `face`: the direction of the model that goes down), `turns` (quarter turns on the bed) and `split`: `joint` (`flat`, `pins` or `dovetail`), `cuts` (a list of `[axis 0 to 2, position in mm]`), `clearance`, and the sizes in mm `pin_d`, `pin_depth`, `tab_root`, `tab_tip`, `tab_depth`, `tab_pitch` |

For example, this call turns on the bed chamfer, puts the part "Clip" on its best face and splits the part "Plate" with a dovetail joint at x = 130 mm:

```json
{
  "fixes": {"chamfer": true},
  "parts": {
    "Clip": {"orient": "auto"},
    "Plate": {"split": {"joint": "dovetail", "cuts": [[0, 130]], "clearance": 0.2}}
  }
}
```

- A `split_proposal` of `get_optimize` is a valid `split`.
- `"parts": {"Lid": null}` gives the part "Lid" its default values again.
- An unknown key rejects the call.
- All fixes are off until you turn them on.

### Files

- `export_model` writes `.stl` and `.3mf` as the window does: a plate of the Optimize tab, with the print parts in print position and the fixes that are on. `plate` selects the plate (from 1) or `all`. The default is the plate that the window shows. With `all`, each plate goes to its own file, with `-plateN` before the extension. A plate 3MF has one object for each print part and one build item for each copy. When print parts are too large for the print area, the reply lists them in `left_out`: they are not in the file.
- `export_model` writes `.step` as the model as built. When `only` of the Optimize state names a part, the STEP file has only that part.
- `export_model` prepares the plates when they are not ready.
- `export_model` and `save_photo` do not replace a file without `overwrite`.
- `export_model`, `save_photo` and `view_model` fail when the last build failed, so that they do not use an old mesh.
- `open_model` opens only `.scad` files. `new_model` writes only `.scad` files, and only when the file does not exist. `save_photo` writes only `.png` files.
- `export_model` and `save_photo` do not write through a symbolic link.
- The tools cannot delete a parameter set or a file.

### Images

- `view_model` and `save_photo` put the whole model in the frame. They use the camera direction of the window, or the given `view`.
- `view_model` and `save_photo` show the model as built, with the part colors, the explode and the section cut of the window, as the **Render** tab does. They do not show the reference parts. When the part colors or the explode are on and CoCAD still finds the parts, they wait for the parts, for at most 45 seconds.
- `save_photo` stops after 40 seconds and saves the samples it has.

### `slice`

`slice` slices the current plate of the Optimize tab, with the fixes. Given values (`layer_height`, `first_layer`, `line_width`, `walls`, `infill`, `solid_layers`, `speed`) also change the print settings of the window. The result has the plate, the number of plates, the layers, the path and filament length, the weight and a rough print time in minutes.

### `set_view`: tab and layer

- `mode` is `model`, `optimize` or `render`. The names before 0.7, `slice` and `photo`, also work.
- `layer` sets the layer of the Optimize tab, from 1.
- `render: true` starts the render of the Render tab, as the **Render** button does. It needs the Render tab, so give `mode: "render"` in the same call or before it. `false` stops the render. The tab does not start a render by itself. The `render` value of `get_state` has `running`, `samples` and `of`. To save an image, use `save_photo`.

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
- The cuts and the sides to keep stay in all tabs. In the Optimize tab, the cuts go across the plate: `at` is a position on the bed, and the default is the center of the plate. The plate has its own positions. With `mode` in the same call, `at` is in the space of that tab.
- `get_state` shows the cuts in the same form, with the positions of the current tab.
- `view_model` and `save_photo` show the cut of the model. No export uses the cut.

### Window

- A minimized window may not answer until you restore it.

## Security

- Each window listens on a free port of `127.0.0.1` and accepts only calls with a random token.
- The token is in `mcp/<pid>.json` in the [settings folder](OpenSCAD-engine-and-files#the-settings-folder), which only your user can read.
- Any program that runs as your user can read that file and control the window, and write files through the export tools.

The server is on by default. To turn it on or off, use **MCP server** in **File > Settings**. The change applies when you save. Other open windows change at their next start.

## Analytics

If usage analytics are on, `open_model`, `new_model` and a `set_view` call that changes the `mode` send events. The other tools send no event. The `cocad mcp` process itself sends nothing. See [Privacy](Privacy#when-an-ai-assistant-is-connected).
