# Command line

*Advanced topic. This page is for technical readers.*

The `cocad` program also works in a terminal, without the window. Use it to export, slice or make photos of many models in a script.

## Open the window

```bash
cocad model.scad              # open in the Model tab
cocad --optimize model.scad   # open in the Optimize tab
cocad --render model.scad     # open in the Render tab
```

The names before 0.7, `--slice` and `--photo`, also work.

## Commands without the window

| Command | Action |
|---|---|
| `cocad params FILE.scad` | List the Customizer parameters. A part list shows the part names of the [model definition](Model-definition). |
| `cocad sets FILE.scad` | List the [parameter sets](Parameter-sets) of `FILE.yml`. With `--json`, write them in the format of the OpenSCAD Customizer, for `openscad -p`. |
| `cocad export FILE.scad -o OUT` | Build and write `OUT`. The extension sets the format: `.stl`, `.3mf` or `.step`. STL and 3MF get the print plate, STEP gets the model as built. |
| `cocad slice FILE.scad` | Slice the plate and show the estimates |
| `cocad photo FILE.scad -o OUT.png` | Make a path traced photo from the iso view. It uses all CPU cores, not the GPU. |
| `cocad mcp` | Serve MCP on stdin and stdout. See [MCP reference](MCP-reference). |
| `cocad --help` | Show the usage text |
| `cocad --version` | Show the version |

`export`, `slice`, `photo` and `mcp` stop with an error when the OpenSCAD binary has no Manifold backend. The error shows the install steps. `params` does not run OpenSCAD.

## Examples

```bash
cocad params model.scad
cocad export model.scad -o model.3mf -D width=80 -D 'shape="square"'
cocad export model.scad -o big.stl -P big           # saved set 'big' from model.yml
cocad sets model.scad --json > sets.json            # the sets for openscad -p sets.json -P big
cocad export model.scad -o box.3mf --chamfer --teardrops --bridges=stepped --brim
cocad export model.scad -o box.stl --bed 250x210x220
cocad export model.scad -o lid.stl --part Lid --orient auto
cocad export model.scad -o model.stl --as-built     # the whole model, as before 0.7
cocad export badge.scad -o badge.3mf --for orca     # multicolor 3MF for OrcaSlicer
cocad slice model.scad --layer-height 0.2 --infill 20 --walls 3
cocad photo model.scad -o model.png --size 1600x1200 --samples 256 --color eb6f40 --finish satin --layer-lines
```

## Parameter values

| Option | Action |
|---|---|
| `-P SET` | Apply a saved [parameter set](Parameter-sets) |
| `-p FILE` | Read the sets from `FILE`: a [model definition](Model-definition) (`.yml`) or a `.json` file of the OpenSCAD Customizer. Without `-p`, the command uses `<model>.yml`, else `<model>.json`. |
| `-D name=value` | Set one parameter. You can use it more than once. These values apply after the set. |

## Print options of `export` and `slice`

`export` and `slice` do the same print preparation as the [Optimize](Optimize) tab. They start from the Optimize values that the app stored for the model: the print area, the fixes, the parts, the orientations and the splits. When the app stored no values, they start from the defaults: all parts, the rotation of the model, all fixes off, and the print area of **File > Settings > Printer**. The options below change these values for this run only.

| Option | Action |
|---|---|
| `--part NAME` or `--part all` | Only this print part, or all print parts. A part that you turned off in the app is printed when you name it. |
| `--bed WxDxH` | The print area in mm, for example `--bed 250x210x220` |
| `--orient model` or `--orient auto` | The orientation of all parts: the rotation of the model, or the best face |
| `--chamfer` or `--chamfer=MM` | Bed chamfer. 0.4 mm by default. `--chamfer=0` turns it off. |
| `--teardrops` | Teardrop holes |
| `--bridges` or `--bridges=sacrificial` or `--bridges=stepped` | Bridge fixes, with the fix for a hole above a bridge |
| `--brim` | Brim on small bed islands |
| `--ears` | Mouse ears on the sharp bed corners of large parts |
| `--holes` or `--holes=MM` | Hole size: vertical round holes get their diameter plus MM. 0.2 mm by default. |
| `--no-fixes` | All stored fixes off. The fix options above still turn a fix on. |
| `--as-built` | The model as built, without the print preparation, as before 0.7. With `--part NAME`, only that part, in its place in the model. |

All print parts go on one plate, in one file. The print area sets the fit of each part and the splits. When the parts need more room than the bed and each part fits, the report shows a note. Your slicer arranges the parts on its plates. CoCAD 0.7 had more plates and a `--plate` option. This option now gives an error.

The command line cannot split a part. It uses the splits that you made in the app. A part that is too large for the print area is on the plate, but your printer cannot print it. The report tells you to split it in the Optimize tab.

`export` and `slice` write a report on stderr: the parts, the print parts with their sizes, the copies on the plate, the fixes and the problems of the check.

### Options of `export`

| Option | Action |
|---|---|
| `-o OUT` | The output file |
| `--for prusa` or `--for orca` | Write a multicolor 3MF for that slicer. See [Multicolor prints](Multicolor-prints). The print options do nothing with `--for`. |

STEP always gets the model as built. With `--part NAME`, the STEP file has only that part. The print options do nothing for STEP.

A 3MF plate has one object for each print part and one build item for each copy.

### Options of `slice`

| Option | Action |
|---|---|
| `--layer-height MM` | Layer height in mm |
| `--first-layer MM` | First layer height in mm |
| `--line-width MM` | Line width in mm |
| `--infill PCT` | Infill in percent |
| `--walls N` | Number of walls |
| `--solid-layers N` | Number of solid layers at the top and the bottom |
| `--speed MM_S` | Print speed in mm/s |

`slice` slices all parts on the plate as one job. The layer heights also set the steps of the bed chamfer, the brim and the sacrificial layers, for `export` too.

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

`photo` shows the model as built, not the print plate.

## Environment

| Variable | Action |
|---|---|
| `OPENSCAD` | The OpenSCAD binary. It overrides the saved setting. |
| `OPENSCAD_ARGS` | Extra OpenSCAD arguments. It overrides the saved setting. |

The commands use the settings of the app. See [OpenSCAD engine and files](OpenSCAD-engine-and-files).

## Notes

- The commands `params`, `export`, `slice` and `photo` send no analytics and no error reports. See [Privacy](Privacy).
- On Windows, a release build opens no console window. The commands write to the console that started them.
