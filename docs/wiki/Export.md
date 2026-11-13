# Export

Export writes a file that other programs can read: a slicer for your 3D printer, or a CAD program.

![The Export card with two plates](https://raw.githubusercontent.com/cgeosoft/cocad/main/assets/guide/app-7-two-plates.jpg)

## How to export

1. Find the **Export** card at the bottom of the right panel. It is in all three tabs: **Model**, **Optimize** and **Render**.
2. Read the line above the buttons. It tells what the file will contain, for example "Plate: 4 parts, 6 copies. Bed chamfer, 3 teardrops."
3. Click **STL**, **3MF** or **STEP**.
4. Select a place and a name for the file. The proposed name is the model name. When a parameter set is active, the set name is added, for example `box-big.stl`. When the parts are on two or more plates, the plate number is added, for example `box-big-plate2.stl`. The dialog opens in the folder of your last export. Before your first export, it opens in the folder of the model.

The **Model** menu has the same commands.

## What is in the file

| Button | What the file contains |
|---|---|
| **STL** | The current plate of the [Optimize](Optimize) tab: the print parts on the bed, in print position, with the print fixes that are on. |
| **3MF** | The same plate. Each print part is one object, and each copy is one item, so your slicer shows them as separate objects. The file also contains the values of all parameters. |
| **STEP** | The model as built, as OpenSCAD made it, for a CAD program. It has no plate and no print fixes. When the dropdown of the **Plate** card shows one part, the file contains only that part, in its place in the model. |
| **All STL**, **All 3MF** | These buttons show when the parts are on two or more plates. They write one file for each plate, for example `box-plate1.3mf` and `box-plate2.3mf`. |

You do not have to open the **Optimize** tab before an export. When the plates are not ready, CoCAD prepares them first. A model that you did not change in the **Optimize** tab gives all parts in the rotation of the model, with no print fixes, on the print area from **File > Settings > Printer**.

A part that is too large for the print area is on no plate, so it is not in the STL or the 3MF file. The message after the export names each part that it left out. [Split](Optimize#split-a-part-that-is-too-large) it in the **Optimize** tab, or set a larger print area. When no part is on a plate, the **STL** and **3MF** buttons are not available. **STEP** still works.

For a print in more than one color, see [Multicolor prints](Multicolor-prints).

## Which format

| Format | Use it for |
|---|---|
| **STL** | A slicer for a 3D printer. It contains only the triangles of the surface. |
| **3MF** | A slicer for a 3D printer, when you want separate objects and your values in the file |
| **STEP** | A CAD program. STEP has flat faces in place of triangles. A closed model becomes one solid that CAD programs can edit. |

## Check the model before you export

The **Mesh** card of the **Model** tab and the status bar show data about the model:

- **Size** in mm: width × depth × height.
- **Volume** in cm³.
- **Triangles**: the number of triangles of the surface.
- **Open edges**: the status bar shows them only when the model has a problem. A model with open edges is not a closed solid, and a slicer can give wrong results for it.

Two more checks in the 3D view:

- Faces that point the wrong way show in **purple**.
- Press `W` to see the triangles of the surface (wireframe).

The **Check** card of the **Optimize** tab lists the print problems: overhangs, bridges and parts that are too large. See [Optimize](Optimize#check).

## What the export does not contain

Part colors, explode and section are for the view only. The parts with a `%` mark in the model ([reference parts](Parts#reference-parts)) are never in an export.

---

**For technical readers.** The STL is binary. The 3MF holds the parameter values in its metadata. Each object of a plate 3MF has the name of its print part. The STEP file is AP214: coplanar triangles become planar faces, and a closed mesh becomes one `FACETED_BREP` solid. An open mesh becomes an open shell. To export without the window, see [Command line](Command-line).
