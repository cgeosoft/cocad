# Problems and answers

## Start and install

**CoCAD says that it cannot find OpenSCAD.**
Install OpenSCAD first. If it is installed in an unusual place, set the program in **File > Settings > OpenSCAD**. See [Install](Install#if-cocad-does-not-find-openscad).

**macOS does not open the app.**
The app is not signed. The first time, right-click the app and choose **Open**.

**How do I see the welcome tour again?**
Use **Help > Welcome tour**.

**How do I get a newer version?**
CoCAD does not install updates. Download the new version from the [releases page](https://github.com/cgeosoft/cocad/releases) and install it as before.

## The model

**The model takes a long time to show.**
The 2021 release of OpenSCAD is slow on large models. Install a development snapshot of OpenSCAD. The status bar shows the OpenSCAD version and the word "Manifold" when the fast backend is in use. See [OpenSCAD engine and files](OpenSCAD-engine-and-files).

You can also turn off **Auto render** in the **Model** tab. Then the model renders only when you click **Render** or press `F5`.

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

**The Explode card says that it needs two or more parts.**
CoCAD found only one part in the model. Explode has nothing to move.

**CoCAD shows two parts as one, or one part as many.**
CoCAD finds the parts from the structure of the model. See the limits in [How CoCAD finds the parts](How-CoCAD-finds-the-parts).

**My part colors are not in the exported file.**
That is correct. The part colors are for the view and the photo only. For a file with colors, see [Multicolor prints](Multicolor-prints).

## Parameter sets

**The disk button is orange.**
The selected set has changes that are not saved. Click the button to save them.

**My new sets file is not on the disk.**
CoCAD writes a new sets file at the first save.

**Auto save does not save.**
With **Model defaults** selected, auto save saves nothing. Select a set, or make a new one.

## Slice and photo

**Can I print the slice preview?**
No. The preview is for checks only and writes no G-code. Export the model and use a slicer program.

**The photo is grainy.**
Wait. The image gets cleaner with each sample. You can also set more **Samples** in the **Quality** section.

**The photo starts again and again.**
The render starts again each time you move the view or change a setting. Let the view rest.

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
