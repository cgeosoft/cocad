# Problems and answers

## Start and install

**At the start, CoCAD shows "CoCAD needs OpenSCAD with Manifold".**
CoCAD did not find an OpenSCAD development snapshot. The release 2021.01 and the `openscad` package of Debian 13 do not work with CoCAD. Install a development snapshot with the steps in the notice, then click **Check again**. If the snapshot is in an unusual place, click **OpenSCAD settings** and select the program. See [Install](Install#if-cocad-does-not-find-openscad-with-manifold).

**The status bar shows "Not built: OpenSCAD has no Manifold".**
The OpenSCAD program in **File > Settings > OpenSCAD** has no Manifold backend. Select a development snapshot, or clear the **Binary** field so that CoCAD finds the program itself.

**macOS does not open the app.**
The app is not signed. The first time, right-click the app and choose **Open**.

**How do I see the welcome tour again?**
Use **Help > Welcome tour**.

**How do I get a newer version?**
CoCAD does not install updates. Download the new version from the [releases page](https://github.com/cgeosoft/cocad/releases) and install it as before.

## The model

**The model takes a long time to show.**
Make sure that **Extra arguments** in **File > Settings > OpenSCAD** has no `--backend=cgal`. See [OpenSCAD engine and files](OpenSCAD-engine-and-files).

You can also turn off **Auto build** in the **Model** tab. Then the model builds only when you click **Build** or press `F5`.

**The model shows an error.**
Click **Console** in the status bar, or use **View > OpenSCAD console**. The console shows the messages of OpenSCAD.

**A model shows an error, but it is a 2D drawing.**
CoCAD needs a 3D shape. A model that contains only 2D shapes, or no shape, gives an error.

**Some faces are purple.**
Purple faces point the wrong way: you see the back of the surface. This is a problem in the model. While a [section](Parts#section) cut is on, the inside faces show in a dark part color and not in purple.

**The status bar shows open edges.**
The surface of the model has holes, so the model is not a closed solid. A slicer can give wrong results for such a model. Repair the model in its `.scad` file.

**I changed the `.scad` file in an editor. Must I open it again?**
No. CoCAD loads the file again when it changes on disk. Your edited values stay.

## Parts

**The Explode card is not there.**
CoCAD found only one part in the model. Explode has nothing to move, so the card does not show.

**CoCAD shows two parts as one, or one part as many.**
CoCAD finds the parts from the structure of the model. See the limits in [How CoCAD finds the parts](How-CoCAD-finds-the-parts).

**My part colors are not in the exported file.**
That is correct. The part colors are for the view and the photo only. For a file with colors, see [Multicolor prints](Multicolor-prints).

## Parameter sets

**The disk button is orange.**
The selected set has changes that are not saved. Click the button to save them.

**The model has no `.yml` file.**
CoCAD writes the [model definition](Model-definition) at the first save of a set.

**A message names the `.yml` file.**
The [model definition](Model-definition) has a problem, for example an unknown key or a part without an `id`. The message and the top of the OpenSCAD output tell what is wrong. CoCAD does not use that entry.

**Auto save does not save.**
With **Model defaults** selected, auto save saves nothing. Select a set, or make a new one.

## Optimize and print

**The Plate card says "Too large for the print area".**
The part does not fit the print area of the **Printer** card. Do one of these:
- Check the print area. Select your printer in the **Printer** card.
- Put the part on a different side: click **Select face** in the **Orientation** card. When the part fits on a different side, the **Plate** card tells you so.
- Click **Split** in the **Split** section below the row. See [Optimize](Optimize#split-a-part-that-is-too-large).

A part that is too large is on no plate, so it is not in the STL or 3MF export.

**The Split section says that no parallel cuts make the part fit.**
CoCAD cuts only with parallel planes across one axis. Turn the part with a custom orientation, or use a larger print area.

**The Plate card says that a cut "has no room for pins" or "has no room for tabs".**
The cut face is too small for the pins or the dovetail tabs, so that cut is a flat cut. Glue the pieces, or move the cut with its number field to a place with more material.

**My STL does not contain the whole model.**
The STL and 3MF export write the current plate of the Optimize tab. Parts with the `%` mark, parts that you turned off, and parts that are too large are not on the plate. When the parts are on two or more plates, click the arrow at the right of **Export 3MF**, then **3MF: all 2 plates** or **STL: all 2 plates**. For the model as OpenSCAD built it, use **STEP: the model as built** in the same menu, or `cocad export --as-built` on the [command line](Command-line).

**I use the Bed chamfer. Must I change a setting in my slicer?**
Yes. With the **Bed chamfer** on, set the elephant foot compensation of your slicer to 0. In PrusaSlicer and OrcaSlicer, the setting is "Elephant foot compensation". In Cura, it is "Initial Layer Horizontal Expansion". Without this, the bed edges get two corrections.

**The Print fixes card shows "1 place skipped".**
A fix found a place that it cannot change, for example a hole with a wall above it that is too thin for the teardrop point. Hold the mouse on the text to see the place and the reason.

**Can I print the slice preview?**
No. The preview is for checks only and writes no G-code. Export the plate and use a slicer program.

## Render

**The photo is grainy.**
Wait. The image gets cleaner with each sample. You can also set more **Samples** in the **Quality** section.

**The Render tab shows no photo.**
The tab does not start a render by itself. Click **Render** at the bottom of the tab.

**The photo stops.**
The render stops when you move the view or change a setting, because the image does not show the new view. Click **Render** again.

## Multicolor

**All parts have one color in PrusaSlicer.**
The file for PrusaSlicer contains no colors, only the extruder of each part. Use a printer profile with more than one filament.

**The file does not load correctly in my slicer.**
Use the export for your slicer. The file for PrusaSlicer does not work in OrcaSlicer, and the reverse. PrusaSlicer 3.0 alpha is not supported.

## AI assistant

**The assistant does not get an answer from CoCAD.**
- Make sure that the CoCAD window is not minimized.
- Make sure that **Allow MCP clients** is on in **File > Settings > MCP server**.
- If the client cannot find `cocad`, use the full path of the program. The welcome tour shows it in the step **Connect an agent**.

**The assistant says that a file exists.**
The assistant does not replace a file by default. Tell it to overwrite the file, or give a different name.

## More help

If your problem is not here, open an issue at <https://github.com/cgeosoft/cocad/issues>.
