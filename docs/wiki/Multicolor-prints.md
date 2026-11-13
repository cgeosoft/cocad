# Multicolor prints

Some printers can print one object with more than one filament. For such a print, the slicer must know which part of the object uses which filament. CoCAD can write a 3MF file with this data, for PrusaSlicer or for OrcaSlicer.

## What you need

The model must name its parts and their colors. The author of the model does this in the [model definition](Model-definition), the `.yml` file next to the model, with `multicolor: true`. See [Write models for CoCAD](Write-models-for-CoCAD#parts-for-a-multicolor-print).

To try it, open **File > Examples > Multicolor badge**.

## What you see

- The 3D view shows each part in its color.
- After the first build, CoCAD builds each part in the background. Until this is done, the view shows the model in orange.
- Shapes that are in no part stay orange. With **Part colors** on, they get other colors.

## How to export

When the model has named parts, the export menu in the **Optimize** tab has two more items: **Multicolor 3MF: PrusaSlicer 2.9** and **Multicolor 3MF: OrcaSlicer**. To open the menu, click the arrow at the right of **Export 3MF**. The **Model** menu has the same exports:

- **Export multicolor 3MF for PrusaSlicer 2.9**
- **Export multicolor 3MF for OrcaSlicer**

Select the slicer that you use. The two slicers read different data, so one file does not work in both.

The multicolor 3MF contains all parts of the model as built. It does not use the plate of the [Optimize](Optimize) tab.

## In the Optimize tab

The parts of the multicolor list are the colors of one object. In the [Optimize](Optimize) tab, they stay together as one print part with the name of the model. The orientation, the print fixes and the plate apply to this object as a whole. **Export 3MF** and **STL: this plate** write this object without colors.

## In the slicer

| Slicer | What you get |
|---|---|
| **PrusaSlicer 2.9** | Each part has its own extruder. The file contains no colors. Use a printer profile with more than one filament. With a one-filament profile, all parts show in one color. |
| **OrcaSlicer** | Each part has its own filament, and the file contains the colors. The printer and process settings stay as they are. |

The order of the parts in the `parts` list of the definition sets the filament number.

## Limits

- **PrusaSlicer 3.0 alpha** is not supported. It loads the file as one object.
- The tests used PrusaSlicer 2.9.7 and OrcaSlicer 2.4.2.
- CoCAD builds each part in a separate OpenSCAD run. A model with 5 parts takes 5 builds.

---

**For technical readers.** On the command line, use `cocad export badge.scad -o badge.3mf --for prusa` or `--for orca`. See [Command line](Command-line).
