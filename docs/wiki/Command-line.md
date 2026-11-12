# Command line

*Advanced topic. This page is for technical readers.*

The `cocad` program also works in a terminal, without the window. Use it to export, slice or make photos of many models in a script.

## Open the window

```bash
cocad model.scad            # open in the model view
cocad --slice model.scad    # open in the slice view
cocad --photo model.scad    # open in the photo view
```

## Commands without the window

| Command | Action |
|---|---|
| `cocad params FILE.scad` | List the Customizer parameters |
| `cocad export FILE.scad -o OUT` | Render and write `OUT`. The extension sets the format: `.stl`, `.3mf` or `.step`. |
| `cocad slice FILE.scad` | Slice and show the estimates |
| `cocad photo FILE.scad -o OUT.png` | Make a path traced photo from the iso view |
| `cocad mcp` | Serve MCP on stdin and stdout. See [MCP reference](MCP-reference). |
| `cocad --help` | Show the usage text |
| `cocad --version` | Show the version |

## Examples

```bash
cocad params model.scad
cocad export model.scad -o model.3mf -D width=80 -D 'shape="square"'
cocad export model.scad -o big.stl -P big          # saved set 'big' from model.json
cocad export badge.scad -o badge.3mf --for orca     # multicolor 3MF for OrcaSlicer
cocad slice model.scad --layer-height 0.2 --infill 20 --walls 3
cocad photo model.scad -o model.png --size 1600x1200 --samples 256 --color eb6f40 --finish satin --layer-lines
```

## Parameter values

| Option | Action |
|---|---|
| `-P SET` | Apply a saved [parameter set](Parameter-sets) |
| `-p FILE` | Read the sets from `FILE`. Without `-p`, the command uses the sets file that the app remembers for the model, else `<model>.json`. |
| `-D name=value` | Set one parameter. You can use it more than once. These values apply after the set. |

## Options of `export`

| Option | Action |
|---|---|
| `-o OUT` | The output file |
| `--for prusa` or `--for orca` | Write a multicolor 3MF for that slicer. See [Multicolor prints](Multicolor-prints). |

## Options of `slice`

| Option | Action |
|---|---|
| `--layer-height MM` | Layer height in mm |
| `--infill PCT` | Infill in percent |
| `--walls N` | Number of walls |

## Options of `photo`

| Option | Default | Action |
|---|---|---|
| `-o OUT.png` | | The output file |
| `--size WxH` | `1600x1200` | Image size in pixels |
| `--samples N` | `128` | Samples per pixel |
| `--color RRGGBB` | `ff9300` | Plastic color |
| `--finish matte\|satin\|gloss` | `satin` | Finish |
| `--studio dark\|light` | `dark` | Backdrop |
| `--layer-lines` | off | Show print layer lines |

## Environment

| Variable | Action |
|---|---|
| `OPENSCAD` | The OpenSCAD binary. It overrides the saved setting. |
| `OPENSCAD_ARGS` | Extra OpenSCAD arguments. It overrides the saved setting. |

The commands use the settings of the app. See [OpenSCAD engine and files](OpenSCAD-engine-and-files).

## Notes

- The commands `params`, `export`, `slice` and `photo` send no analytics and no error reports. See [Privacy](Privacy).
- On Windows, a release build opens no console window. The commands write to the console that started them.
