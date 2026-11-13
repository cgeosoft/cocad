# Write models for CoCAD

*Advanced topic. This page is for persons who write OpenSCAD models.*

CoCAD shows a model as OpenSCAD makes it. A few comments in the `.scad` file tell CoCAD which values the user can change. The [model definition](Model-definition), a `.yml` file next to the model, holds the rest: the title, the part names and the parameter sets. The `.scad` file holds only OpenSCAD code.

## One view: the assembled model

A model for CoCAD needs only one view: the assembled model, with each part in its place of use. The model needs no print code. The [Optimize](Optimize) tab of CoCAD does the print preparation:

| Do not put this in the model | CoCAD does it |
|---|---|
| A print layout of all parts | The **Plate** card: all parts or one part, copies and more plates |
| `rotate()` and `translate()` to put a part on the bed | **Custom orientation**: Auto or Select face |
| Teardrop shapes for horizontal holes | **Teardrop holes** |
| Larger diameters for vertical holes, or polygon holes | **Hole size** |
| Square layers or sacrificial layers over holes, cones at the end of holes | **Bridges** |
| A chamfer of the bed edges in layers | **Bed chamfer** |
| A brim for thin feet | **Brim on small islands** |
| Disks on the corners of a large flat part | **Mouse ears** |
| The bed size and a fit check | The **Printer** card and the **Check** card |
| Cuts of a part that is too large, with joint tabs | **Split**, with flat, pins or dovetail joints |

Three rules:

1. Make each print part a module call in one assembly module. The [module split](How-CoCAD-finds-the-parts#method-3-the-module-split) then finds the parts. To give the parts names, also add a [`part` list](#names-for-the-parts). A part that you need more than one time is more than one call of the same module. CoCAD finds parts with the same shape and prints them as copies.
2. Mark each part that is not printed with `%`. See [Parts that are not printed](#parts-that-are-not-printed).
3. Keep the design rules that only the model can follow, for example round corners, wall sizes and overhangs that need no support. The **Check** card of the Optimize tab shows the problems that stay.

The example **Desk dock** (`desk-dock.scad`) uses this form, with a `part` list for the names:

```openscad
part = "all"; // [all, plate, stand, clips]

module show(name) if (part == "all" || part == name) children();

module assembly() {
    show("plate") plate();
    show("stand") stand();
    show("clips") {
        clip_at(0.3);    // three copies of one clip
        clip_at(0.55);
        clip_at(0.8);
    }
    if (part == "all") {
        %phone();        // shown, not printed
        %magnets();
        %screws();
    }
}
assembly();
```

`desk-dock.yml` gives the parts the names "Base plate", "Stand" and "Cable clip".

See [Prepare a model for printing](Prepare-a-model-for-printing) for the print preparation of this model.

## Customizer parameters

CoCAD reads the same comments as the OpenSCAD Customizer. A model that works in the OpenSCAD Customizer works in CoCAD.

```openscad
/* [Size] */
// Outer width
width = 40;            // [10:100]
wall = 1.2;            // [0.8:0.2:4]
shape = "round";       // [round, square]
count = 2;             // [1:One, 2:Two]
lid = true;
offset = [0, 0];       // [-20:20]
label = "Hi";          // [10]
/* [Hidden] */
$fn = 64;
```

| Line | Result in the Parameters panel |
|---|---|
| `/* [Size] */` | Starts a group with the name "Size" |
| `// Outer width` above a parameter | A description. The panel shows it as the label. |
| `// [10:100]` | A slider from 10 to 100 |
| `// [0.8:0.2:4]` | A slider from 0.8 to 4 in steps of 0.2 |
| `// [round, square]` | A dropdown list |
| `// [1:One, 2:Two]` | A dropdown list that shows labels for the values |
| `true` or `false` | A switch |
| A vector with `// [-20:20]` | One number box for each element, with the limits |
| A string with `// [10]` | A text box with a maximum length of 10 |
| `/* [Hidden] */` | The parameters after it do not show |

CoCAD stops reading parameters at the first top-level `module` or `function`. Put all parameters before it.

To see the result, run `cocad params model.scad`. See [Command line](Command-line).

## Parts for the 3D view

You do not need to do something for the part functions. CoCAD finds the parts from the module calls of the model. A model of this form gives good parts:

```openscad
module assembly() {
    box();
    lid();
    knob();
}
assembly();
```

For the rules and the limits, see [How CoCAD finds the parts](How-CoCAD-finds-the-parts). The example `lid-box.scad` uses this form, with a [`part` list](#names-for-the-parts) for the names.

## Names for the parts

Without more information, the parts have the names "Part 1", "Part 2" and so on. To give them names, add a `part` parameter that selects one part, and list the parts in the [model definition](Model-definition#parts). Many OpenSCAD models select a part this way, and it works in OpenSCAD too:

```openscad
part = "all"; // [all, box, lid, feet]

if (part == "all" || part == "box") box();
if (part == "all" || part == "lid") lid();
if (part == "all" || part == "feet") feet();
if (part == "all") %coin();
```

In `model.yml`:

```yaml
parts:
  - id: box
    name: Box
  - id: lid
    name: Lid
  - id: feet
    name: Foot
```

Rules:

- The first value is `all`. It shows the assembled model: all parts in their place of use, and the `%` parts.
- Each other value shows only that part, in its place of use. Do not turn the part to a print position. The [Optimize](Optimize) tab does that.
- Do not show the `%` parts for the other values.
- The `name` in the definition is the name of the part. Without `parts` in the definition, CoCAD uses the labels of the dropdown list (`// [all:All, box:Box]`), else the values.
- `color` in the definition gives the part a color in the 3D view, for example `color: "#4e8fd6"`. See [Model definition](Model-definition#parts).
- `part_parameter` in the definition sets another name for the parameter.
- One value can give more than one solid, for example four feet. The parts then have the names "Foot 1", "Foot 2" and so on.
- Parts of one value that touch, for example the segments of a board, stay separate when each one is a module call. A `for` loop of module calls also works:

```openscad
module show(name) if (part == "all" || part == name) children();

show("segments") for (i = [0 : 3]) segment(i);
```
- Without `parts` in the definition, the dropdown list must have two or more values other than `all`.

CoCAD builds each value in a separate OpenSCAD run. Each solid is a separate print part. The names stay when the model builds again, so the selection, the focus and the part colors stay too. For the details, see [How CoCAD finds the parts](How-CoCAD-finds-the-parts#method-2-the-part-list).

The example `part-selector.scad` shows this. Open it with **File > Examples > Part selector**.

## Parts that are not printed

Put the OpenSCAD background modifier `%` in front of a part that is not printed, for example hardware, a board or a phone:

```openscad
box();
%screw();   // shown, not printed
```

- OpenSCAD shows a `%` object in its preview in transparent gray, and leaves it out of the render and the export. The model gives the same result in OpenSCAD and in CoCAD.
- CoCAD builds each `%` object on its own and shows it in the **Model** tab in transparent gray, as a [reference part](Parts#reference-parts). It is never on a plate, in an export, in the slice or in the photo.
- `*` (disabled) objects stay hidden, as in OpenSCAD.

The example `reference-part.scad` shows this. Open it with **File > Examples > Reference part**.

## Parts for a multicolor print

To let the user export a [multicolor 3MF](Multicolor-prints), select the parts with a `part` parameter, as for [the part names](#names-for-the-parts). Show each part only when it is selected:

```openscad
part = "all"; // [all, plate, star]

module show(name, col) {
    if (part == "all" || part == name)
        color(col) children();
}
```

In the definition, set `multicolor: true` and give each part its filament color:

```yaml
multicolor: true
parts:
  - id: plate
    name: Plate
    color: white
  - id: star
    name: Star
    color: red
```

Rules:

- CoCAD builds each part in a separate OpenSCAD run, with `part` set to the id of the part.
- In the [Optimize](Optimize) tab, the parts of the list are the colors of one object. They stay together as one print part.
- The order of the list sets the filament number.
- `color` sets the filament color of the part: a `#RRGGBB` value or a common color name. `color()` in the model only changes the OpenSCAD preview.
- Parts must not overlap. Cut a pocket in one part for the other part.
- `import()` ignores SVG fill colors. Use one SVG file for each color.
- Shapes that are in no part become "Other" parts in the 3D view.

The example `multicolor-badge.scad` shows all of this. Open it with **File > Examples > Multicolor badge**.

## Fonts

Fonts from `use <font.ttf>` in the model file work in the module split of the parts. Fonts that only a library loads do not.

## Models that fail

A model must make 3D geometry. A model with only 2D shapes, or with no geometry, gives an error.
