# Model definition

*Advanced topic. This page is for persons who write OpenSCAD models.*

The model definition is a YAML file next to the model: `box.yml` for `box.scad`. The `.scad` file holds only OpenSCAD code. The definition holds the rest:

- The title, the description and other text about the model.
- The names and the colors of the parts.
- The [parameter sets](Parameter-sets).

All items are optional. A model without a definition works in CoCAD too.

## Example

```yaml
# yaml-language-server: $schema=https://cocad.cgeosoft.com/schema/model-1.json
cocad: 1
title: Part selector
description: A box with a lid and four feet.
author: Your name
license: MIT
url: https://example.com/part-selector

parts:
  - id: box
    name: Box
  - id: lid
    name: Lid
  - id: feet
    name: Foot

sets:
  Small box:
    description: The default box.
  Card box:
    description: A box for playing cards.
    values:
      width: 65
      depth: 80
      lid_t: 4
```

The examples in **File > Examples** all have a definition. CoCAD writes them to the `examples` folder of the [settings folder](OpenSCAD-engine-and-files).

## The keys

| Key | Value |
|---|---|
| `cocad` | The version of the format: `1`. CoCAD does not read a file with a newer version. |
| `title` | The name of the model. The **Parameters** panel and the start page show it. |
| `description` | What the model is. |
| `author`, `version`, `license`, `url` | More text about the model. |
| `parts` | The part names. See [Parts](#parts). |
| `part_parameter` | The parameter that selects a part. Default: `part`. |
| `multicolor` | `true`: the parts are the colors of one object. See [Multicolor prints](Multicolor-prints). Default: `false`. |
| `sets` | The parameter sets. See [Sets](#sets). |
| `x-...` | A key for other tools. CoCAD does not read it. |

## Parts

The model selects one part with a string parameter. The value `all` shows all parts:

```openscad
part = "all"; // [all, box, lid, feet]

if (part == "all" || part == "box") box();
if (part == "all" || part == "lid") lid();
if (part == "all" || part == "feet") feet();
```

The `parts` list of the definition gives each other value a name:

```yaml
parts:
  - id: box        # the value of the part parameter
    name: Box      # the name that CoCAD shows
  - id: lid
    name: Lid
  - feet           # only the id: the name is "feet"
```

- The order of the list is the order in the dropdown list of the parameter. For a multicolor model, it also sets the filament number.
- `color` sets the color of a part in a multicolor model: `#RRGGBB` or a color name (white, black, red, green, blue, yellow, orange, purple, pink, brown, cyan, magenta, gold, silver, gray).
- The parameter must be a string parameter in the `.scad` file, not in the `Hidden` group. A dropdown list in the `.scad` file is optional. With the list, the part selection also works in the OpenSCAD Customizer. The names of the definition replace the labels of the list.
- For the rules of the parts, see [Write models for CoCAD](Write-models-for-CoCAD#names-for-the-parts).

## Sets

```yaml
sets:
  Big:
    description: For the workshop.
    values:
      width: 80          # number
      shape: round       # text
      label: "5"         # text that looks like a number: use quotes
      lid: true          # true or false
      offset: [0, 5]     # vector
```

A parameter that is not in the set keeps the default of the model. A set without values gives the defaults. See [Parameter sets](Parameter-sets).

## Problems in the file

- A file that CoCAD cannot read gives a message, and the model opens without the definition.
- An unknown key, a part without an `id` or a wrong color gives a warning. CoCAD does not use that entry. The warnings show in a message and at the top of the OpenSCAD output.
- A `parts` list for a parameter that the model does not have gives a warning. CoCAD then uses the dropdown list of the `.scad` file.

## Schema

The JSON Schema of the format is at <https://cocad.cgeosoft.com/schema/model-1.json>. The first line of the example tells the YAML extension of VS Code and other editors to use it. The editor then completes the keys and marks the errors.

## Models from CoCAD 0.7

CoCAD 0.7 kept the sets in `box.json`, in the format of the OpenSCAD Customizer. The part names were the labels of the `part` list, and a multicolor model had a `cocad_part` list and `echo(cocad_color = ...)`.

- A model without a `.yml` file and with a `.json` file: CoCAD reads the sets and the text of the `.json` file. The first save of a set writes the `.yml` file. The `.json` file stays.
- Without `parts` in the definition, the labels of a `part` list are the part names, and a `cocad_part` list is a multicolor list. `echo(cocad_color = ...)` sets the color of a part without a `color` in the definition.
- For OpenSCAD, `cocad sets box.scad --json` writes the sets in the Customizer format: `openscad -p <(cocad sets box.scad --json) -P Big -o big.stl box.scad`.
