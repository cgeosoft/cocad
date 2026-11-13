# How CoCAD finds the parts

*Advanced topic. This page is for technical readers. For the part functions, see [Parts](Parts).*

OpenSCAD joins all shapes of a model into one mesh. CoCAD divides the model into parts again. It uses four methods, in this order.

## The part list

Methods 1 and 2 use the part list of the model. It comes from the first of these that the model has:

1. `parts` in the [model definition](Model-definition#parts). The parameter is `part`, or the `part_parameter` of the definition. It must be a string parameter of the model. The names are the `name` of each part. With `multicolor: true`, it is a multicolor list.
2. A `cocad_part` dropdown list with string values (models of CoCAD 0.7). It is a multicolor list, and the values are the names.
3. A `part` dropdown list with string values, the value `all` and two or more other values. The labels are the names.

## Method 1: a multicolor list

For a model with a multicolor list that shows all parts, CoCAD builds each part in the background. See [Write models for CoCAD](Write-models-for-CoCAD#parts-for-a-multicolor-print).

- When the parts are done, they replace the solids, so parts that touch stay separate.
- Shapes that are in no part become "Other" parts.
- Until the parts are done, the view shows the solids of the model in orange.
- A model with a multicolor list that shows one part keeps the solids of the mesh.

## Method 2: the part list

Many OpenSCAD models select a part with a `part` parameter. CoCAD uses this list to find and name the parts when it is not a multicolor list. See [Write models for CoCAD](Write-models-for-CoCAD#names-for-the-parts).

When `part` is `all`:

1. CoCAD builds each other value in a separate OpenSCAD run, with `part` set to that value. The runs are in parallel. The **Parts** card shows "Building the parts…" while they run.
2. Each solid of each value is one part, so parts of different values that touch stay separate.
3. Solids that touch in one value become one solid, for example four segments of one board. So CoCAD also applies the [module split](#method-3-the-module-split) to each value. When its pieces give more solids than the build of the value, each solid of each piece is one part.
4. A part gets the name of its value. A value that gives two or more parts gives "Name 1", "Name 2" and so on. The solids of one build are in volume order, the largest first. The pieces of the module split are in the order of the model.
5. Shapes of the `all` build that are in no value become "Other" parts.
6. A value that gives no 3D geometry gives no part.
7. In the [Optimize](Optimize) tab, each part is a separate print part.

When `part` is one value, the solids of the build are the parts. As with `all`, CoCAD also applies the module split, and uses its pieces when they give more solids. The parts get the name of that value.

When a run fails, the module split finds the parts. The error goes to the OpenSCAD output.

## Method 3: the module split

For a model without a part list:

1. After each build, CoCAD runs OpenSCAD one more time to write the CSG tree of the model. This run makes no geometry.
2. In the tree, each module call is a group. CoCAD finds the highest level with two or more module calls, and one more level below it. An example is `assembly() { board(); pieces(); }`.
3. CoCAD builds each module call on its own, in parallel.
4. Each solid of each module is one part, so parts that touch stay separate.

The **Parts** card shows "Finding the parts…" while this runs.

Limits of the module split:

- A part made of two or more module calls divides into these modules.
- An explicit `union()` stays one part.
- A level with more than 64 module calls gives no split.
- Fonts from `use <font.ttf>` in the model file work, but not fonts that only a library loads.
- When the module split fails, the error goes to the OpenSCAD output and the solids stay.

## Reference parts: the `%` subtrees

OpenSCAD keeps the `%` (background) subtrees in the CSG tree, and leaves them out of the mesh. After each build, CoCAD builds each outermost `%` subtree on its own, as a [reference part](Parts#reference-parts). This works with all methods.

- A `%` subtree inside a `*` (disabled) subtree stays hidden, as in OpenSCAD.
- More than 64 `%` subtrees build as one reference part.
- A reference part that fails to build is left out.
- The reference parts come after the other parts. The [Optimize](Optimize) tab, the export, the slice and the photo do not use them.

## Method 4: the solids of the mesh

For all other models, or when the module split finds no parts, CoCAD uses the solids of the mesh.

- A solid is a set of triangles that connect at their edges.
- A cavity stays with the solid around it.
- A sliver with almost no volume goes to the nearest part.
- The solids of one mesh are in volume order, the largest first.
- The parts get the names "Part 1", "Part 2" and so on.

## Colors

- A part of a multicolor list keeps its color.
- The other parts get colors from a palette of 12 colors when **Part colors** is on.

## Selection and focus

- The selection and the focus use the part name. When the model builds again and has a part with that name, they stay.
- With **Explode** on, a click finds the part at its moved position, and the focus moves with the part.
- A click goes through the transparent parts to the part in focus. A click on a transparent part selects it when the part in focus is not under the mouse.
- When the part in focus is not fully in the view, the camera moves back until the part fits.
- **Leave focus** moves the center of rotation back to the center of the model. The camera direction and distance stay.

## Explode

- The slider goes to approximately 2 times the largest size of the model. The default distance is approximately half of the largest size.
- The grid moves down below the lowest part.

## Section

- In the **Model** and **Render** tabs, the cut is in the coordinates of the model. With the explode on, each part moves with its cut.
- In the **Optimize** tab, the cut is in the coordinates of the bed. The print parts there are turned and placed, and same parts share one mesh, so a cut in model coordinates would not show the parts of the plate correctly. The plate positions are apart from the model positions. The cut applies to the layers and to the clicks on the plate too.
- A transparent part shows no cut face. A part that is not closed can show no cut face or a wrong cut face.
- The cut faces need a stencil buffer in the OpenGL driver. Without it, the inside of each part shows in a dark part color.
- The grid, the fit function and the exports do not use the cut.
