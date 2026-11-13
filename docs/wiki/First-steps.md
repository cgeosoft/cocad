# First steps

This page shows how to open a model and how to look at it.

## The welcome tour

At the first start, a welcome tour shows the main features one step at a time. Click **Next** and **Back**, or press the Right and Left arrow keys. Press Esc to close the tour.

To see the tour again, use **Help > Welcome tour**.

## Open a model

A model is a file with the extension `.scad`. You can open it in four ways:

- Press Ctrl+O, or use **File > Open…**, and select the file.
- Drop the file on the CoCAD window.
- Select a model in the list of recent models. The **File** menu shows the last 10 models. The window shows the same list when no model is open.
- Use **File > Examples** to open a sample model.

If you have no model, start with an example. **File > Examples > Gear** is a good first model. **File > Examples > Desk dock** shows the print preparation. See [Prepare a model for printing](Prepare-a-model-for-printing).

> The examples are copies in the settings folder of CoCAD. You can change them. Your changes stay.

## The window

![The CoCAD window](https://raw.githubusercontent.com/cgeosoft/cocad/main/assets/tour/1-welcome.jpg)

| Area | Position | Content |
|---|---|---|
| Menu bar | Top | All commands: **File**, **Model**, **View** and **Help** |
| Parameters panel | Left | The values that you can change. See [Change the parameters](Parameters). |
| 3D view | Middle | The model |
| Sidebar | Right | Three tabs: **Model**, **Optimize** and **Render**, and the **Export** card at the bottom |
| Status bar | Bottom | The state of the model, its size and the **Console** button |

The three tabs of the sidebar change what the 3D view shows:

- **Model**: the shape of the model and its [parts](Parts).
- **Optimize**: the parts on the bed of your printer, and the print layers. Press `O`. See [Optimize](Optimize).
- **Render**: a [photo](Render) of the part. Press `R`.

The tab names show the key letter underlined. Press the same key again to go back to the **Model** tab.

Each tab has cards. Click the header of a card to close it or to open it. **Collapse** at the top of the tab closes all cards, and **Expand** opens them again. The **Export** card stays at the bottom of all tabs. See [Export](Export).

## Move the view

| You do this | The view does this |
|---|---|
| Drag with the left mouse button | Turns around the model |
| Drag with the right or the middle button | Moves to the side |
| Hold Shift and drag with the left button | Moves to the side |
| Turn the mouse wheel | Moves nearer or farther |
| Double click, or press `F` | Shows the full model |

The buttons at the bottom of the 3D view set a fixed direction: **Iso**, **Top**, **Front** and **Right**. **Fit** shows the full model. **Wire** shows the triangles of the model. **Grid** shows or hides the floor grid.

## The status bar

The status bar shows, from left to right:

- The state of the build and the time it took. **Ready** means that the model is up to date. CoCAD calls the OpenSCAD step "build".
- The OpenSCAD version. It shows in red when the program has no Manifold backend. See [Install](Install).
- The size of the model in mm, the number of triangles and the volume.
- The number of open edges, if the model has some. See [Export](Export#check-the-model-before-you-export).
- In the **Optimize** tab: the plate (when there are two or more), the layer and the print estimate. In the **Render** tab: the samples and the image size.
- The **Console** button. It shows the messages of OpenSCAD. Look there when a model shows an error.

## When the model file changes

If you or another program change the `.scad` file on disk, CoCAD loads the model again. The values that you changed in CoCAD stay.

## Next step

Go to [Change the parameters](Parameters).
