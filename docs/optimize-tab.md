# Optimize tab (design draft)

Status: draft, 2026-11-13. The owner answered the first open questions (see Decisions). Nothing on this page is implemented.

## Goal

A model file holds only the model: all parts in their place in use. It has no print math. CoCAD does the print preparation in a new **Optimize** tab. The Optimize tab replaces the **Slice** tab and keeps the slice preview. The **Photo** tab becomes the **Render** tab (see Tabs and keys).

Today the models in `scad-models` do the print preparation in OpenSCAD. This table shows where each item goes:

| Item in scad-models | What it does | Goes to |
|---|---|---|
| `part = "print"`, `print_layout()` | All print parts flat on the bed, with a gap | Feature 6 and the plate layout |
| One `part` value for each print part | Export one part | Feature 6 |
| `print_part()` with `translate()` and `rotate()` | Puts the bed face of a part at z = 0 | Feature 2 |
| `teardrop2d()`, `teardrop_2d()` | Horizontal holes with a 45 degree point | Feature 3 |
| `bridge_top()`, `sac`, cone ends | Bridges start on straight edges, sacrificial layers | Feature 4 |
| `bed_brim()`, `print_box()` | Brim on small bed islands, inside the bed | Feature 4.1 |
| `bed_x`, `bed_y`, `fits()` | Bed size and the bed fit check | Feature 8 |
| `split`, `piece_a()`, `piece_b()`, `tabs_2d()` (book-cover), `z_split`, `lap_l` (open-case-frame) | Cut a part that is too large for the bed, with dovetail tabs or a lap joint | Feature 9 |
| `bed_cut()`, `bed_chamfer`, `layer_h` | 45 degree chamfer of the bed edges, in layers | Feature 5 |
| `PRINT` console line | Quantity of each part | Feature 6 |
| `source/tools/overhang.py` | Check of overhangs, arcs, small islands and bed fit | Print check (proposal, not in the list) |

## The features

| # | Feature | Short description |
|---|---|---|
| 1 | Export card at the bottom | One Export card, fixed at the bottom of the right panel, in all tabs |
| 2 | Orientation | Off by default. With custom orientation on, CoCAD puts a part on its best face (Auto), or the user selects the face |
| 3 | Teardrop holes | Round holes with a horizontal axis get a 45 degree point at the top |
| 4 | Bridges | Bridged ceilings start on straight edges. A hole above a bridge starts on a sacrificial layer or a stepped bridge |
| 4.1 | Brim on small islands | A one-layer brim on small first-layer islands, with a gap to the rest of the part |
| 5 | Bed chamfer | A 45 degree chamfer of the bed edges, in whole layers |
| 6 | Print one part or all | Select the parts to print. The default is all parts |
| 7 | Collapsible cards | The cards of the right panel open and close like the parameter groups |
| 8 | Print area | A Printer section in the settings with presets and a default print area. The Optimize tab sets the area of each model |
| 9 | Auto split | CoCAD cuts a part that is too large for the bed into pieces, with joints: dovetail, pins, plug, lap |

## The print pipeline

CoCAD does these steps in this order. Each step uses the result of the step before it.

1. **Build.** OpenSCAD makes the model mesh, as today (this step was called Render).
2. **Parts.** CoCAD finds the parts, as today: the `cocad_part` list, the module split, or the solids of the mesh.
3. **Print parts.** CoCAD joins parts that overlap in volume, finds parts with the same shape, and applies the part selection (feature 6).
4. **Split** (feature 9). When the user accepts it, a part that does not fit the print area (feature 8) is cut into pieces, with joints. Each piece is a print part from here on.
5. **Orientation** (feature 2). Each print part keeps its model rotation, or, with custom orientation on, gets the Auto rotation or the face that the user selected. Its lowest point goes to z = 0.
6. **Analysis.** In print position, CoCAD finds the horizontal holes, the bridged ceilings and the bed islands of each part.
7. **Fixes** (features 3, 4, 5 and 4.1, in this order). CoCAD changes the geometry of each part, only for the fixes that the user turned on. All fixes are off by default. The brim comes last, because the bed chamfer must not cut it. The fixes also apply to the joints of step 4: a horizontal pin hole gets a teardrop.
8. **Plate layout.** CoCAD puts the parts on the bed, with a gap.
9. **Preview and export.** The Optimize tab shows the plate. The slice preview and the export use the plate.

Steps 4 and 7 are the slow steps. CoCAD runs it in the background, and runs it again only for a part whose input changed: its mesh, its orientation or a fix setting.

## How CoCAD changes the geometry

Features 3, 4, 4.1, 5 and 9 change the mesh with booleans (union and difference). The export is a mesh for an external slicer, so the change must be in the mesh, not only in the slice preview.

| Option | How | For | Against |
|---|---|---|---|
| A. OpenSCAD pass | For each part, CoCAD writes a small `.scad` file: `import()` of the part mesh, `multmatrix()` for the orientation, and the fixes as `difference()` and `union()` with simple shapes. The `openscad` CLI makes the result. | One geometry engine, as today. The CSG recipes already work in scad-models (`bed_cut`, `bed_brim`, `bridge_top`, `teardrop2d`). No new native library. | One more OpenSCAD run for each changed part. Slow without the Manifold backend. |
| B. Booleans in Rust | A Rust crate does the booleans on the mesh. | Fast. No temporary files. | See "Mesh booleans in Rust" under Prior art. |
| C. Change the slice layers | The fixes change the 2D layers of the CoCAD slicer. | Simple 2D work. | The fixes do not go into the exported file. Not useful. |

Decision (owner, 2026-11-13): option A, and CoCAD requires an OpenSCAD nightly build. Rust does the analysis (steps 4 and 6) and writes the shapes. OpenSCAD with the Manifold backend does the booleans. See [Requirement: OpenSCAD nightly](#requirement-openscad-nightly).

The reason for the requirement: the CGAL backend is too slow for the extra runs. The lantern `print` export did not end in 4.9 hours with OpenSCAD 2021.01 (CGAL), and took 4 s with a nightly build with Manifold (scad-models notes).

## Requirement: OpenSCAD nightly

CoCAD requires an OpenSCAD build with the Manifold backend. Today this is a development snapshot (nightly). The last stable release, 2021.01, has no Manifold. Manifold is in the snapshots since 2024-09-28 and is their default since 2025-08-17 (see Prior art). When a stable release has Manifold, the requirement becomes "OpenSCAD with Manifold", and that release also meets it.

What changes in CoCAD:

- **Check at start.** CoCAD already probes the binary (`openscad::probe()`: the version, and `--backend` with `manifold` in `--help`). When CoCAD finds no binary with Manifold, the window shows a notice with:
  - the version that it found, or "not found",
  - the steps to get a nightly for the OS of the user (see below),
  - the link <https://openscad.org/downloads.html#snapshots>,
  - a button that opens the OpenSCAD settings, to select a binary,
  - a button that probes again, after the user installs the nightly.

  CoCAD does not build with CGAL.
- **Steps in the notice**, one set for each OS:
  - Debian and Ubuntu: add the OpenSCAD repository on `download.opensuse.org` and install the package `openscad-nightly`. On Debian 13 the source line is `deb https://download.opensuse.org/repositories/home:/t-paul/Debian_13/ ./`. The signing key step comes from the OpenSCAD download page. Other releases use another folder name.
  - Windows and macOS: download a development snapshot from the download page, install it, then probe again. CoCAD already looks in `C:\Program Files\OpenSCAD*` and `/Applications/OpenSCAD*.app`.
  - Check each step on a clean system before the release.
- **Search order.** CoCAD already looks for `openscad-nightly` first, then `openscad`. When it finds both, it uses the first one that has Manifold.
- **Settings.** The **Manifold backend** switch goes away. CoCAD always adds `--backend=manifold`. A user `--backend=cgal` argument gives a warning.
- **CLI and MCP.** `cocad export`, `slice`, `photo` and the MCP server stop with a clear error when the binary has no Manifold.
- **Code.** The CGAL paths can go: the `manifold` field of `openscad::Config`, and the engine label in the status bar can show only the version.

Packages and docs:

- `scripts/package-deb.sh` has `Depends: ... openscad-nightly | openscad`. Decision (owner): remove OpenSCAD from the `.deb`. The check at start tells the user how to get the nightly. Reason: the Debian 13 `openscad` package is 2021.01, so it does not meet the requirement, and `openscad-nightly` is not in Debian. It comes from the OpenSCAD repository on `download.opensuse.org` (`home:/t-paul`), so a `Depends` on it blocks the install until the user adds that repository. (Rule 7 of `CLAUDE.md`.)
- `README.md`, `docs/wiki/Install.md`: "a development snapshot is much faster" becomes "CoCAD needs a development snapshot". The same steps for each OS as the notice.
- `docs/wiki/Settings.md`, `docs/wiki/OpenSCAD-engine-and-files.md`: remove the switch and the CGAL text.
- `docs/wiki/Problems-and-answers.md`: an entry for the notice.
- The website: the install text.
- The Windows and macOS packages: the install text and the first start notice. CoCAD does not bundle OpenSCAD.

## Feature 1: Export card at the bottom

Now: the Export card is the second card of the Model tab. It scrolls with the other cards. The Slice and Photo tabs have no Export card.

Proposal:

- One Export card in the right panel, below the tab content, in all three tabs. It stays at the bottom. The other cards scroll above it.
- The same buttons as today: STL, 3MF, STEP, and the multicolor 3MF row when the model has a `cocad_part` list.
- One line above the buttons tells what the file will contain, for example "Plate: 7 parts, 9 objects. Bed chamfer, 4 teardrops." or "Model as built."
- egui: call `egui::TopBottomPanel::bottom("export").show_inside(ui, ...)` in the right `SidePanel`, before the tab content. The tab content then gets the remaining height.

What the export contains (decision, owner):

- **STL and 3MF**: the print result of the current plate. This is the selected parts, in print orientation, with the fixes that are on, in the plate layout. Parts with the `%` mark are not in it (feature 6).
- **Export all plates**: when the parts need more than one plate, one file for each plate, with the plate number in the name (for example `box-big-plate2.3mf`).
- **STEP**: the model as OpenSCAD made it, for a CAD program. With one part selected, only that part, in its model position. The print fixes are for the printer, not for CAD.
- **3MF of a plate**: one object for each different part, and one build item for each copy. The slicer then shows separate objects. The scad-models rule "split it to objects in the slicer" goes away.
- **Multicolor 3MF**: as today.

## Feature 2: Orientation

The model gives each part in its position in use. By default, CoCAD keeps this rotation: the part only goes down to z = 0. When the user turns on **Custom orientation** for a part, CoCAD can find the print position (Auto), or the user selects the face that goes on the bed. See User control below.

### Auto: the method

#### Candidates

- A rigid part can only lie on a face of its convex hull. Each face of the convex hull (coplanar triangles joined) is one candidate. This includes the six axis directions when the part has flat sides.
- The orientation as modeled is also a candidate.
- Remove a candidate when the center of mass is not above its contact area (the part falls over), or when the contact area is very small.

#### Cost

For each candidate, CoCAD turns the part so that the candidate face is at z = 0, and measures:

| Term | Meaning | Better when |
|---|---|---|
| Overhang area | Area of down-facing faces above the bed that are steeper than the overhang limit (45 degrees from the vertical by default). Faces in the bed chamfer zone do not count. | Smaller |
| Fixable overhang | Overhang that a fix makes printable: horizontal round holes (feature 3) and bridges shorter than the bridge limit with a fixable outline (feature 4). It counts with a low weight. | Smaller |
| Bed contact area | Area of the faces on the bed. Rule 1 of scad-models: one large flat bed face. | Larger |
| Height | Height of the part in print position. Fewer layers, less time, more stable. | Smaller |

The weights need tuning on the test models (`examples/` and the scad-models models). When the cost of the "as modeled" orientation is near the best cost, CoCAD keeps it. A model that is already in print position then does not turn.

#### Turn about z

After the bed face, CoCAD turns the part about z:

1. Take the convex hull of the footprint (the part seen from above).
2. Find the bounding rectangle with the smallest area (rotating calipers on the hull).
3. Put the long side along the long axis of the bed.
4. If the part does not fit the bed, try the diagonal. If it still does not fit, mark the part "Too large for the bed".

### User control

Decision (owner): custom orientation is an option, off by default.

For the selected part, the Orientation card has:

- **Custom orientation**: a switch, off by default. Off: the part keeps its model rotation.
- With the switch on:
  - **Auto**: CoCAD puts the part on its best face (the method above). This is the first value when the user turns the switch on.
  - **Select face**: a click on a face in the 3D view turns the part so that this face is on the bed.
  - Buttons to turn the part by 90 degrees about z.

CoCAD stores the switch and the choice per model and per part name in `models.json`. It stores the model direction that points down and the angle about z, so the choice stays when the model builds again with other values.

The geometry does not tell which face must look good or in which direction the part must be strong. So the model gives no orientation hint: the user selects the face where it matters, one time per model.

## Feature 3: Teardrop holes

"Holes that are vertical to the bed" in this page means: the round outline of the hole stands vertical, so the axis of the hole is parallel to the bed. The top of such a hole is a ceiling in the air. It sags or needs support.

What CoCAD does: the top of each such hole gets a 45 degree point up (a teardrop). Optional: a flat top at a set height, so that the hole stays closer to round. For a hole of radius r, the full point adds r(√2 - 1), about 0.41 r, above the circle.

### Find the holes

1. OpenSCAD makes a round hole as a prism of `$fn` flat faces. Each face has edges parallel to the axis of the hole.
2. CoCAD collects the faces that are parallel to one direction d. This direction is the axis.
3. CoCAD projects the vertices of these faces on the plane perpendicular to d, and fits a circle: center and radius. The fit error must be small.
4. The normals point to the center: this is a hole. The normals point away from the center: this is a pin or a boss. A pin gets no teardrop. The Print check shows its overhang.
5. CoCAD keeps a hole when its axis is near the bed plane (within about 10 degrees), the arc covers the top of the circle, and the diameter is in the limits (settings with defaults, to tune).
6. Coaxial holes with different radii (a counterbore on a horizontal axis) each get their own point, as `cb_x()` in `open-case-frame.scad` does.

### Change the geometry

- CoCAD cuts a prism along the hole. Its section is the triangle between the two points of the circle at 45 degrees and the tip at r√2 above the center. With a flat top, the tip is cut at the set height.
- The prism has the length of the hole along its axis, with a small overlap where the hole opens to a face.
- Before the cut, CoCAD casts rays up from the tip line. If the wall above is thinner than the point plus a minimum wall, CoCAD does not change this hole and lists it in the Print check.

## Feature 4: Bridges

A ceiling is a flat face that points down, above the first layer. It prints as a bridge. The slicer prints the outline of a bridge first, in the air. The print head cannot follow an arc in the air, so a round hole or a round corner in the outline of a bridge sags (rule 2 of scad-models).

### Cases and fixes

| Case | Example | Fix |
|---|---|---|
| a. Ceiling over a round cavity that opens to the bed | Round counterbore or slot on the bed side | The last layer under the ceiling gets an outline of straight lines that holds the round outline: a square for a circle, a rectangle for a slot (`bridge_top()`). The corners of the square sit on solid plastic. |
| b. A hole goes on above the bridged ceiling | Screw hole above a counterbore | Option 1, sacrificial layer: the hole starts `sac` (two layers, 0.4 mm) above the ceiling. Drill it after the print, or melt a heat-set insert through it. Option 2, stepped bridge: one layer with a slot as wide as the hole, one layer with a square, then the round hole. No drilling. |
| c. Blind hole that goes up, with a flat round ceiling | Vertical blind hole | A 45 degree cone at the end of the hole. If the cone breaks into a cavity above it, a square `bridge_top()` layer. |
| d. Bridge longer than the bridge limit (12 mm by default) | Wide pocket | No automatic fix. The Print check lists it. Another orientation can help. |

The fix setting chooses the option for case b. Option 2 needs no work after the print. Option 1 gives a closed, stronger bridge.

### Find the ceilings

1. Collect the faces with a normal near -z, above the first layer and above the bed chamfer. Join adjacent coplanar faces into regions.
2. Find the outline loops of each region.
3. Find the arcs: an outline vertex where the edge turns by 0.5 to 40 degrees is on an arc (the same test as `overhang.py`).
4. Measure the span: the width of the region across the best bridge direction.
5. Sort each region into case a, b, c or d. An inner loop of a region that is round is a hole that goes on above the bridge (case b).
6. Before a cut, check that the new square or rectangle stays inside the part, with a minimum wall to the outside.

### Feature 4.1: Brim on small bed islands

This is the `bed_brim()` module of `open-case-frame.scad`, moved into CoCAD. It applies to every small island, not only to the sacrificial walls of bridges.

- A bed island is an area of the first layer that is not joined to the rest of the part there.
- An island is small when it is thinner than 3 mm (two times the largest distance from a point in the island to its edge) or smaller than 25 mm². These are the `overhang.py` limits.
- A small island gets a brim of one layer, `brim_w` wide (5 mm), joined to the island. The brim keeps `brim_gap` (1 mm) from the rest of the part, so it comes off with the island.
- A thin strip of a larger island gets no brim. The larger island holds it.
- CoCAD makes the plate layout, so it knows the real bed position of each part. The brim stays inside the bed and away from the other parts. This is better than `bed_area2d()`, which assumes that the slicer puts the part at the bed center.

## Feature 5: Bed chamfer

The first layers spread out a little (elephant foot). A 45 degree chamfer of the bed edges hides it.

This is `bed_cut()` from scad-models, moved into CoCAD:

1. Take the section of the part just above the chamfer (at the chamfer height plus half a layer). This is the reference outline.
2. In each layer of the chamfer, keep only the reference outline, made smaller by the remaining chamfer height. The first layer gets the full chamfer, the last layer gets one layer height.
3. The steps follow the real layer heights from the Print settings (first layer, then layer height). In scad-models all steps use `layer_h`.

Results:

- An edge that already has a 45 degree chamfer or foot does not get a second one, because the reference outline is above it.
- Holes and pockets on the bed get the same chamfer, because a smaller outline makes its holes larger.
- Default 0.4 mm. 0 turns it off.
- A part of one or two layers (a sheet or a flap) gets no chamfer. Sacrificial walls and brims come after the chamfer.

The slicer can also do elephant foot compensation. With both on, the bed edge gets two corrections. The Optimize tab tells the user to set the slicer compensation to 0 or a low value when the CoCAD chamfer is on.

## Feature 6: Print one part or all parts

The Optimize tab has a **Plate** card:

- At the top: **All parts** (default) or one part.
- A list of the print parts: name, count, size, a check box (print or not), copies, and a status (fits the bed, open check items).
- With one part selected, the plate holds only this part and its copies. The 3D view and the export show it.
- A click on a part in the 3D view selects its row. The part menu (right click) gets **Print only this part**.

This replaces the `part` dropdown of the scad-models models.

### Which parts are print parts

- The parts that CoCAD finds today are the start.
- Two parts that **overlap in volume** cannot be two separate printed pieces. CoCAD joins them into one print part. Two parts that only touch (a lid on a box) stay separate. Test: the volume of the whole model is less than the sum of the part volumes. Then CoCAD finds the pairs that overlap.
- The parts of a `cocad_part` list are the colors of one print object. They stay together as one object.
- The user can uncheck a part for this print, for example a part that is already printed. CoCAD stores this per model.

### Parts that are not printed: the `%` mark

Decision (owner): a part that is never printed (hardware, a board, the pages of a book) has a mark in the `.scad` file, and it is never on the bed.

The mark is the OpenSCAD background modifier `%` (owner: ok).

```openscad
module assembly() {
    frame();
    %psu();      // shown, not printed
    %hardware();
}
```

Tested with OpenSCAD 2026.11.11 nightly and the Manifold backend:

- The CSG output keeps the `%` subtree, with `%` in front of its `group()`.
- The STL output does not contain the `%` subtree.
- `src/csg_split.rs` already reads the `%`, `#`, `!` and `*` modifiers, and treats `%` and `*` nodes as no geometry.

Why `%`:

- It is standard OpenSCAD. In OpenSCAD, the preview shows a `%` object in transparent gray, and the render and the export leave it out. A model with `%` gives the same result in OpenSCAD and in CoCAD.
- It needs no new CoCAD syntax and no new parameter. `show_hardware` and `show_pages` become `%hardware();` and `%pages();`.

What CoCAD does:

- Today CoCAD does not show `%` objects, because they are not in the STL. New: CoCAD finds each `%` subtree in the CSG tree, builds it on its own (as the module split does), and shows it in the 3D view as a reference part, in transparent gray.
- A reference part is never in the Plate card, the plate, the slice preview or the export. It shows in the Parts card with a "not printed" mark.
- `*` (disabled) objects stay hidden, as in OpenSCAD.

### More than one plate

Decision (owner): more plates are in the first version.

- When the parts do not fit on one bed, CoCAD fills plate 1, then plate 2, and so on.
- The Plate card shows "Plate 1 of 3", with buttons for the next and the previous plate. The 3D view shows the current plate.
- The Export card exports the current plate, and has **Export all plates** (feature 1).
- The slice estimate shows the current plate and the total of all plates.

### Same parts

CoCAD finds parts with the same shape: the same volume, surface area and principal moments of inertia, in a tolerance. It shows them as one row with a count, for example "leg ×4". The plate holds one copy for each count. The `PRINT` console line of scad-models goes away.

### Plate layout

- CoCAD puts the footprint rectangles of the parts in rows, the largest first, with a gap (10 mm by default, as `print_layout(g = 10)`). It keeps room for the brims. It centers the result on the bed.
- The layout can turn a part by 90 degrees about z to fit. This does not change how the part prints.
- A part that does not fit the bed alone goes to the split (feature 9). It is on no plate until the user splits it.
- The bed is the print area of the model (feature 8).

## Feature 7: Collapsible cards

- Each card in the right panel gets the header of the parameter groups (`ui::group_header`): a chevron and the title. A click opens or closes the card.
- A closed card shows a short value in its header where it helps, for example the Estimate card "1 h 20 min, 42 g", or the Bed chamfer row "0.4 mm".
- An **Expand all** / **Collapse all** button, as in the left panel.
- The open state is a layout choice, so CoCAD stores it per card for all models, in the app state. Today the state of the parameter groups is not stored. It resets at each start. Proposal: store both.
- The Export card does not close. It is small, and it holds the main action.

## Tabs and keys

Decision (owner): the right panel has the tabs **Model**, **Optimize** and **Render**.

| Tab | Was | Key | Label |
|---|---|---|---|
| Model | Model | | Model |
| Optimize | Slice | `O` | <u>O</u>ptimize |
| Render | Photo | `R` | <u>R</u>ender |

- `O` and `R` work as `S` and `P` work today: the key opens the tab, and the same key again goes back to the Model tab. The keys do nothing while a text field has the focus.
- `S` and `P` go away. Today `O` and `R` have no other use. `Ctrl+O` (open a file) and `Alt+M` (Model menu) stay.
- The tab label underlines the key letter, so the user sees which key to press. `ui::tabs` gets the index of the key letter for each tab, and draws the label as an `egui::text::LayoutJob` with an underline on that letter.
- The keyboard help (the shortcut list in `src/app.rs`) shows "O: Model or Optimize tab" and "R: Model or Render tab".

### The OpenSCAD step: Build

Decision (owner): the OpenSCAD step is called **Build**, so that "Render" names only the Render tab. The user-facing text changes:

| Place | Now | New |
|---|---|---|
| Model tab, first card | Render | Build |
| Model tab, button | Render | Build |
| Model tab, switch | Auto render, "Render when a parameter changes" | Auto build, "Build when a parameter changes" |
| Model tab, hint | "Render the model and export it." | "Build the model and export it." |
| Model menu | Render now (`F5`), Auto render | Build now (`F5`), Auto build |
| Keyboard help | F5: Render now | F5: Build now |
| Status bar | "Rendering 1.2 s", "Rendering the named parts" | "Building 1.2 s", "Building the named parts" |
| CLI help | `cocad export`: "Render and write OUT" | "Build and write OUT" |
| MCP | The `render` tool, the `render` value of `set_params`, "Auto render switch" | `build`, with `render` kept as an alias, "Auto build switch" |

- The stored key `auto_render` in `models.json` stays. The user does not see it, and a new name needs a migration.
- "OpenGL renderer" (the 3D view) is not the OpenSCAD step. It stays.
- The wiki uses "render" for the OpenSCAD step on many pages, for example "when the model renders again". Change each one to "build". `Glossary.md` gets the entry **Build**. The tour (`src/app/tour.rs`) and the website change the same way.

## Feature 8: Print area

The print area belongs to CoCAD, not to the model. Today each scad-models model has its own `bed_x` and `bed_y`, and the CoCAD slicer has no bed at all.

Decision (owner): a **Printer** section in the general settings with presets, and the print area of each model in the Optimize tab. No printer list and no import from a slicer.

### The values

| Value | Used by |
|---|---|
| Width and depth (mm) | Plate layout, bed fit check, brim limits, turn about z, split, the bed grid in the 3D view |
| Height (mm) | Bed fit check, split |

More printer values (for example the nozzle diameter) can join this section later. The print settings of the slice preview (layer height, first layer, line width, walls, infill, top and bottom layers, speed) stay per model, as today.

### Settings: the Printer section

- A new **Printer** section in **File > Settings**.
- A **Preset** dropdown and three fields: width, depth and height.
- A preset fills the three fields.
- When the user changes a field, the dropdown shows **Custom**. The fields keep the values of the user. When the values are the same as a preset again, the dropdown shows that preset.
- This is the default print area for a model that CoCAD opens for the first time (no entry in `models.json`).
- Storage: in `<config>/config`, as the other settings: the preset name or `custom`, and the three values.

Presets: the printers with confirmed values in Prior art.

### Optimize tab: the print area of the model

- The **Printer** card of the Optimize tab shows the print area of the model: the same dropdown (preset or **Custom**) and the same three fields. The user can type a different area here.
- CoCAD stores it per model in `models.json`, with the other Optimize tab values.
- A change in the settings does not change models that already have a stored print area.
- CLI: `--bed WxDxH` for `cocad export` and `cocad slice`. Without it, the CLI uses the stored value of the model, then the default from the settings. MCP: the print area is part of the print options.

## Feature 9: Auto split

A part that does not fit the print area is cut into pieces. The pieces join with a joint. Today two models do this in OpenSCAD:

- `book-cover`, `split = true`: the board and the spine are cut at mid height, with dovetail tabs across the cut (`tabs_2d()`, pitch `split_tab_pitch` 30 mm, gap `split_clearance` 0.2 mm, 0.1 mm on each side). The joint needs glue.
- `open-case-frame`: the spine is cut at `z_split` into `spine_low` and `spine_high`, with a half-lap joint (`lap_l` 14 mm) and five M3 screws with head counterbores and nut pockets. The cut position keeps away from the board holes.

### When CoCAD splits

- Step 4 of the pipeline: after the print parts, before the orientation. CoCAD tests the fit with the orientation of the part, and with a turn about z. When the part does not fit, but a different orientation fits (feature 2, Auto), the Plate card offers that orientation first.
- The Plate card marks a part that does not fit: "Too large for the bed. Split?". A click shows the proposed cuts. CoCAD does not split without the user.
- The user can also split a part that fits, for example to print a long part flat.

### Where to cut

The proposal for version 1: parallel planar cuts across one axis.

1. Take the axis of the part that does not fit: the long side of its smallest bounding box.
2. The number of pieces is the smallest number for which each piece fits the bed, with the room for the joint.
3. Search the cut positions along the axis. Prefer a position where:
   - the section is one region with enough area for the joint,
   - the section does not cut a hole, a thin wall or a small feature (the open-case-frame rule: away from the board holes),
   - the pieces have near the same length.
4. The user can move each cut in the 3D view. The Section card already draws a cut plane. The split can use the same plane drawing.

Cuts at an angle and cuts across two axes come later.

### Joints

| Joint | Geometry | After the print | From |
|---|---|---|---|
| Flat | Two flat faces | Glue | |
| Pins | Holes in both faces. Pins of filament, metal, or printed pins on the same plate | Push the pins in, glue | |
| Plug | A plug on one face, a socket in the other | Push together, glue | |
| Snap (later) | A plug with a bulge that clicks into the socket | Push together | PrusaSlicer, OrcaSlicer, Bambu Studio |
| Dovetail | Tabs across the cut, in the plane of the part | Slide or press together, glue | `book-cover` |
| Lap with screws | Each piece is half thick over the overlap. Screw holes, head counterbores, nut pockets | Screws and nuts | `open-case-frame` |

Common values: the clearance (default 0.2 mm, 0.1 mm on each side, as `split_clearance`), the size and the number or pitch of the tabs, pins or screws.

Decisions (owner):

- The first version has flat, pins and dovetail. Plug, lap and snap come later.
- CoCAD proposes a joint. The user can change the joint and its values for each cut.
- Printed pins are new print parts. They are not in the model. CoCAD adds them to the plate, with the count that the joint needs.

### Printable joints

- A dovetail prints well when its profile is in the bed plane, as in `book-cover`, where the board lies flat. The orientation of the pieces must keep this. CoCAD gives the dovetail a lower orientation cost when its profile is flat on the bed.
- A pin hole or socket in a cut face on the bed goes up: it prints well. A plug on a face on the bed points up: it prints well. A plug that points down needs support, so CoCAD puts the plug on the piece where it points up, or uses pins.
- The fixes of step 7 also apply here: a horizontal pin hole gets a teardrop, and a socket under a bridge gets a bridge fix.
- The bed chamfer at a cut face makes a small groove at the seam. Option: no bed chamfer on cut faces.

### Storage

CoCAD stores the cuts per model and per part in `models.json`: the axis, the positions, the joint and its values. When the model builds again, CoCAD applies the same cuts.

After this, `book-cover` shows the whole board, and CoCAD makes the pieces. The lap joint of `open-case-frame` has screws and nut pockets in places that the model chooses. It can stay in the model until the lap joint in CoCAD can do the same.

## Optimize tab layout

From top to bottom:

1. **Plate**: All parts or one part, the list of print parts, the plate number (feature 6).
2. **Orientation**: the Custom orientation switch and the choice for the selected part (feature 2).
3. **Print fixes**: one row for each fix, with a switch, its main value, and the number of places it found. For example "Teardrop holes, 4 holes" or "Bridges, 2 fixed, 1 too long". Decision (owner): all fixes are off by default. When the user turns a fix on, CoCAD stores it with the other values of the model in `models.json`, as it stores the panel settings today.
4. **Check** (proposal): the list of problems that remain: overhangs that need support, long bridges, arcs in bridges, parts too large for the bed. A click selects the part and shows the place. This replaces `overhang.py`.
5. **Layers**: the current slice preview (Estimate, Layer, legend). A view switch in the tab changes the 3D view from the plate to the layers.
6. **Printer** (feature 8): the print area of the model, a preset or Custom.
7. **Print settings**: layer height, first layer, line width, walls, infill, top and bottom layers, speed, overhang limit, bridge limit.

8. Fixed at the bottom: **Export** (feature 1).

A part that does not fit the bed shows a **Split** section in the Plate card, with the cuts and the joint (feature 9).

The 3D view of the Optimize tab shows the bed with a grid and the parts in print position. With the Check on, faces that a fix changed show in one color, and problem faces in a second color.

## Settings and storage

- Default print area: global, in `<config>/config` (feature 8).
- Print area of a model: per model in `models.json` (feature 8).
- Cuts and joints: per model and per part, in `models.json` (feature 9).
- Optimize tab values: per model in `models.json`, as the slice settings today. New keys: part selection, copies, the custom orientation switch and choice of each part, the fix switches and values. A new model starts with custom orientation off and all fixes off.
- Card open state: global (feature 7).
- `models.json` today stores `"mode": "slice"` or `"photo"`. Read them as `"optimize"` and `"render"`.

## What changes in scad-models

The suspicion is correct for most parts: a model needs only one view, the assembled model. After the move:

- Remove the `part` dropdown, `print_layout()`, `print_part()`, `print_box()`, `bed_cut()`, `bed_brim()`, `bridge_top()`, the teardrop modules, `layer_h`, `bed_chamfer`, `brim_w`, `brim_gap`, `bed_x`, `bed_y` and `sac`.
- Make each print part a module call in `assembly()`, so that the module split finds it (see `docs/wiki/Write-models-for-CoCAD.md`).
- Mark each part that is not printed with `%`.
- In the scad-models `CLAUDE.md`, rules 1, 4, 5 and 7 and the outline part of rule 2 become CoCAD settings. Rules 3 (round corners) and 6 (layer multiples), and the overhang and bridge length part of rule 2, stay design rules. The CoCAD Check tests them.

Cases in the current models that need more than the assembled view:

| Model | Now | Why CoCAD cannot do it | Proposal |
|---|---|---|---|
| `metal-wire-reel` | `band_print()` prints flat, `band_flat()` wraps around the hub in use | The print shape is not the use shape | Keep a model value for this part |
| `book-cover` | `board_a`, `board_b`, `spine_a`, `spine_b` when `split = true` | The model cuts the board in two pieces with joint tabs, to fit the bed | CoCAD splits the board with a dovetail joint (feature 9). The model shows the whole board. |
| `book-cover` | `open`, `back`, `clash` | Display and check views, not print parts | A parameter (for example `open_angle`), or remove |
| `clip-on-case` | `open`, `half`, `liner` | `open` is a display state. `half` and `liner` are print parts. | `open` becomes a parameter. CoCAD finds the parts. |
| `open-case-frame` | `show_hardware` | Hardware is in the assembled view | `%hardware();` (feature 6) |
| `book-cover` | `show_pages` | Pages are in the assembled view | `%pages();` (feature 6) |
| `metal-wire-reel` | `print_tuned` | Print-only geometry changes | Check each change. CoCAD does the chamfer and the teardrops. Keep the others in the model. |

## Other places to change

- `src/app.rs`: `Mode::Slice` becomes `Mode::Optimize` and `Mode::Photo` becomes `Mode::Render`, with the tab labels, the keys and the keyboard help (see Tabs and keys).
- `src/csg_split.rs` and `src/assembly.rs`: build the `%` subtrees as reference parts (feature 6).
- Settings window: the new Printer section (feature 8).
- `src/app/state.rs`: the new keys. Read the stored `"mode"` value `"slice"` as `"optimize"` and `"photo"` as `"render"`.
- `src/telemetry.rs`: `Screen::Slice` sends `"slice"` and `Screen::Photo` sends `"photo"`. The new values `"optimize"` and `"render"` can be new data: check `docs/telemetry-consent.md` (rule 14).
- `src/mcp.rs`: the `slice` tool, the `set_view` mode list, and print options for `export_model` (parts, plate, orientation, fixes, cuts, print area).
- CLI: `cocad export` and `cocad slice` get print options, for example `--part NAME`, `--plate N`, `--bed WxDxH` and switches for the fixes. `cocad --slice` becomes `--optimize` and `--photo` becomes `--render`. Keep the old flags and the `cocad photo` command as aliases, so that scripts do not break.
- `scripts/package-deb.sh`: remove OpenSCAD from `Depends`.
- `src/mcp.rs`: the `set_view` modes become `model`, `optimize`, `render`. Keep `slice` and `photo` as aliases.
- `src/app/tour.rs`: the "Slice preview" and the photo steps, and their screenshots (rules 5 and 6).
- Wiki: `Slice-preview.md` becomes `Optimize.md`, and `Photo-render.md` becomes `Render.md`. Also `Export.md`, `Parts.md` (reference parts), `Write-models-for-CoCAD.md` (the `%` mark), `Settings.md` (Printer section), `Keyboard-and-mouse.md`, `Command-line.md`, `MCP-reference.md`, `_Sidebar.md`.
- `examples/`: one test model for each fix: a horizontal hole, a counterbore on the bed with a hole above, a blind hole up, a small bed island, a part with a bed pocket.

## Prior art

Checked on 2026-11-13, mostly in the source code and release notes of each tool. Some slicers do parts of this list at the slice level. CoCAD puts each fix in the mesh, so it works in every slicer, and the slice preview and the Check see the result.

Licenses: Tweaker-3 is GPL-3.0 and CoCAD is MIT. Use the methods below, not the code. Check the license before you use code from any other source.

### Orientation (feature 2)

- **Tweaker-3** (the Cura plugin "Auto-Orientation"). The candidates are the 10 face normals with the largest summed area. An extended mode adds random and fixed directions and removes duplicates within 5 degrees. The cost uses the bottom area B, the overhang area O and the contour length C, with weights that an evolutionary algorithm found on 100 objects. [MeshTweaker.py](https://github.com/ChristophSchranz/Tweaker-3/blob/master/MeshTweaker.py)
- **Bambu Studio and OrcaSlicer "Auto orient"** use the same structure. Candidates: the original pose, the 10 largest mesh normals, the 14 largest convex hull normals and 18 fixed directions. Extra cost terms: the area of the bottom convex hull, faces near horizontal, and a penalty when the bottom area is less than 0.1 mm². [Orient.cpp (Bambu Studio)](https://github.com/bambulab/BambuStudio/blob/master/src/libslic3r/Orient.cpp), [Orca wiki: auto orient](https://www.orcaslicer.com/wiki/print_prepare/prepare_auto_orient)
- **PrusaSlicer** has "Optimize orientation" for SLA only. Its "Place on face" takes the planes of the convex hull, joins coplanar facets and ignores planes smaller than 5 mm². This is the model for "Select face". [GLGizmoFlatten.cpp](https://github.com/prusa3d/PrusaSlicer/blob/version_2.8.0/src/slic3r/GUI/Gizmos/GLGizmoFlatten.cpp)
- **Cura "Lay flat"** turns the part until two low vertices are level. It has no cost function. [LayFlatOperation.py](https://github.com/Ultimaker/Uranium/blob/main/UM/Operations/LayFlatOperation.py)

The CoCAD proposal follows Bambu and Orca: hull faces plus the original pose as candidates, and a cost from overhang, bed area and height. The new part is the "fixable overhang" term, because CoCAD knows which overhangs its fixes repair.

### Teardrop holes (feature 3)

- **Cura "Make Overhang Printable"** (`conical_overhang_enabled`, off by default; maximum model angle 50 degrees) and **OrcaSlicer** `make_overhang_printable` (since 1.6.4-beta) change the sliced layers, not the mesh. Each layer becomes the union with the layer above, made smaller by the angle. This changes all overhangs of the part, not only the holes. [ConicalOverhang.cpp](https://github.com/Ultimaker/CuraEngine/blob/main/src/ConicalOverhang.cpp), [Orca wiki: overhangs](https://www.orcaslicer.com/wiki/print_settings/quality/quality_settings_overhangs)
- PrusaSlicer 2.9.7 has no such option. No tool was found that puts teardrop holes into the mesh.

### Bridges (feature 4)

- **OrcaSlicer "Bridge counterbore holes"** (`counterbore_hole_bridging`, since 2.0.0-beta, from SuperSlicer): None (default), Partially bridged, Sacrificial layer. [Orca wiki: bridging](https://www.orcaslicer.com/wiki/print_settings/quality/quality_settings_bridging)
- **Bambu Studio "Counterbore hole bridging"** (since 2.8.0 beta, 2026-06-25). Partially bridged bridges only the ring around the hole and leaves the center open. Sacrificial layer makes one full bridged layer over the hole. [Release notes](https://github.com/bambulab/BambuStudio/releases/tag/v02.08.00.50)
- **Stepped bridging by hand**: the first bridge layers leave a slot as wide as the bore, the next layers at 90 degrees leave a square, then the round bore starts on straight edges. A large bore can need three steps at 60 degrees. One documented recipe uses a 0.2 mm slot step and a 0.4 mm square step. [Hackaday 2023](https://hackaday.com/2023/05/28/3d-printing-bores-without-support/), [Pantheon Design](https://pantheondesign.atlassian.net/wiki/spaces/docs/pages/676069672/Support+Free+Inverted+Counterbores)

### Brim on small islands (feature 4.1)

- **OrcaSlicer** brim types: Auto (default), Mouse ear, Painted, Outer brim only, Inner brim only, Outer and inner, No-brim. `brim_object_gap` is 0 by default, 2 mm maximum. Auto brim works per object, not per first-layer island. [Brim.cpp](https://github.com/OrcaSlicer/OrcaSlicer/blob/main/src/libslic3r/Brim.cpp)
- **PrusaSlicer** brim types: No brim, Outer only (default), Inner only, Outer and inner, with `brim_separation`.
- No slicer was found that puts a brim only on the small islands of a part. Painted brim ears in Orca are the manual way.

### Bed chamfer (feature 5)

- **PrusaSlicer** `elefant_foot_compensation` makes the first layer smaller in XY, 0 by default. The Prusa help suggests about 0.2 mm for a 0.4 mm nozzle. [Prusa help](https://help.prusa3d.com/article/elephant-foot-compensation_114487)
- **OrcaSlicer** has the same setting (0 by default) and `elefant_foot_compensation_layers` (1 by default): the offset gets smaller over N layers.
- **Cura** "Initial Layer Horizontal Expansion" (`xy_offset_layer_0`, 0 by default).
- No slicer was found that makes a 45 degree chamfer of the bed edges. The slicer defaults are 0, so a double correction happens only when the user sets a value.

### Plate layout (feature 6)

- **OrcaSlicer** uses libnest2d with no-fit polygons. With rotation on, it tries 0, 45, 90 and 135 degrees from the smallest bounding box. [Arrange.cpp](https://github.com/OrcaSlicer/OrcaSlicer/blob/main/src/libslic3r/Arrange.cpp)
- **PrusaSlicer 2.9.7** has its own arrange module with no-fit polygons, spacing, and rotation as an option.
- The CoCAD proposal (rows of rectangles) is simpler. It is enough for the preview, the brim limits and the estimate. The slicer can arrange the parts again.

### Print area presets (feature 8)

Width × depth × height in mm, from the maker's spec page:

| Printer | Print area | Source |
|---|---|---|
| Prusa MK4S | 250 × 210 × 220 | [Prusa](https://www.prusa3d.com/product/original-prusa-mk4s-3d-printer-5/) |
| Prusa CORE One | 250 × 220 × 270 | [Prusa](https://www.prusa3d.com/product/prusa-core-one/) |
| Prusa MINI+ | 180 × 180 × 180 | [Prusa](https://www.prusa3d.com/product/original-prusa-mini-semi-assembled-3d-printer-9/) |
| Creality K1C | 220 × 220 × 250 | [Creality](https://www.creality.com/products/k1c-carbon-3d-printer) |
| Creality Ender-3 V3 | 220 × 220 × 250 | [Creality](https://www.creality.com/products/creality-ender-3-v3) |

The PrusaSlicer profiles give the same values for the three Prusa printers ([PrusaResearch.ini](https://github.com/prusa3d/PrusaSlicer/blob/version_2.9.2/resources/profiles/PrusaResearch.ini)).

Not confirmed, so not in the first list: the Bambu Studio profiles give X1C and P1S 256 × 256 × 250, A1 256 × 256 × 256, A1 mini 180 × 180 × 180 ([profiles](https://github.com/bambulab/BambuStudio/tree/master/resources/profiles/BBL/machine)). The Bambu Lab spec pages could not be read, and other sites give 256 mm for the X1C height. Add them after a check of the spec pages.

### Split and joints (feature 9)

- **PrusaSlicer Cut tool.** Connectors and a free cut plane since 2.6.0. The dovetail mode and the Snap connector since 2.6.1. [2.6.0-alpha2](https://github.com/prusa3d/PrusaSlicer/releases/tag/version_2.6.0-alpha2), [2.6.1-rc1](https://github.com/prusa3d/PrusaSlicer/releases/tag/version_2.6.1-rc1), [Prusa help](https://help.prusa3d.com/article/cut-tool_1779)
  - Modes: Planar and Dovetail.
  - Connector types: Plug (added to one side, cut from the other), Dowel (holes in both sides and a separate pin object), Snap. Styles: Prism, Frustum. Shapes: triangle, square, hexagon, circle. Placement: auto or manual.
  - Connector defaults: depth 3 mm, size 2.5 mm, depth tolerance 0.1 mm, size tolerance 0 mm. The tolerance only makes the hole larger. [CutUtils.cpp](https://github.com/prusa3d/PrusaSlicer/blob/version_2.9.2/src/libslic3r/CutUtils.cpp)
  - Dovetail defaults: width 4 × depth, flap angle 60 degrees (30 to 120), groove angle 0 degrees (0 to 15), depth and width tolerance 0.1 mm each. [CutUtils.hpp](https://github.com/prusa3d/PrusaSlicer/blob/version_2.9.2/src/libslic3r/CutUtils.hpp)
  - After the cut, each piece can keep its orientation, lie on the cut face, or turn upside down.
- **OrcaSlicer** has the same tool, ported from PrusaSlicer in 1.9.0, with more than one dovetail in a cut since 2.4.0. **Bambu Studio** has connectors since 1.6 and dovetail and Snap since 1.8, with 0.1 mm for all tolerances. [Orca PR #2603](https://github.com/SoftFever/OrcaSlicer/pull/2603), [Orca 2.4.0-alpha](https://github.com/SoftFever/OrcaSlicer/releases/tag/v2.4.0-alpha), [Bambu 1.8.0](https://github.com/bambulab/BambuStudio/releases/tag/v01.08.00.57)
- In these slicers the user places the cut. CoCAD proposes the cuts, and the user can move them.
- **Chopper** (Luo, Baran, Rusinkiewicz, Matusik, ACM TOG 31(6), 2012) splits a model automatically. It builds a tree of cut planes with a beam search (width 4). Each step cuts the largest part, with 129 plane directions plus the box axes of the part, and offsets every 5 mm. It stops when all parts fit. The cost has terms for the number of parts, the use of the print volume, room for connectors in each section, strength, fragile pieces, seam visibility and symmetry. [Paper](https://gfx.cs.princeton.edu/pubs/Luo_2012_CPM/chopper.pdf). An MIT Python version that is not by the authors: [pychop3d](https://github.com/gregstarr/pychop3d) (last change 2023-07-06).
- The CoCAD proposal is a small part of Chopper: one axis, parallel cuts, a cost for the section and the features it cuts. A Chopper search can come later.
- **Clearance.** The Prusa guide says that printers are accurate to at least 0.2 mm, and suggests at least 0.3 mm for moving parts. There is no single value. [Prusa help](https://help.prusa3d.com/article/modeling-with-3d-printing-in-mind_164135). The slicers use 0.1 mm. For a glued joint, 0.2 mm total (0.1 mm on each side, as `book-cover`) is a good default. The user can change it.

### Mesh booleans in Rust (option B)

| Crate | Version (date) | Build | License | Note |
|---|---|---|---|---|
| `manifold3d` (now on `manifold-csg`) | 0.4.1 (2026-09-09) | C++ Manifold, needs CMake and a C++ compiler | Apache-2.0 or MIT | Maintained |
| `boolmesh` | 0.2.2 (2026-10-27) | Pure Rust, based on the Manifold method | MPL-2.0 | Maintained. The input must be manifold. |
| `csgrs` | 0.20.1 (2025-07-24) | Pure Rust, BSP trees | MIT | Commits in 2026-11 |
| `manifold-rs` | 0.7.0 (2026-02-24) | C++ through cxx and CMake | | Repository archived |

`manifold3d` gives the same engine as OpenSCAD, but it adds a C++ build to the three CI targets. `boolmesh` is pure Rust, but young. Not chosen: the owner chose option A with an OpenSCAD nightly. This table stays for a later change.

### OpenSCAD Manifold backend

- `--backend=manifold` is in the development snapshots since 2024-09-28. [Mailing list](https://lists.openscad.org/empathy/thread/D6KV3ZLXHLBHSITSQ5GPUZUKHURU4ABE)
- Manifold is the default of the snapshots since 2025-08-17. `--backend=cgal` turns it off. [Mailing list](https://lists.openscad.org/empathy/thread/TMJEJCZINIJNYJX2YF7IDNBAPQY66KIF)
- The last stable release is 2021.01. The pre-release 2026.11-RC0 is from 2026-11-07. Whether it uses Manifold by default is not confirmed. [Releases](https://github.com/openscad/openscad/releases)

## Decisions

The owner answered the first open questions on 2026-11-13:

| # | Question | Decision |
|---|---|---|
| 1 | Export content | STL and 3MF give the print plate. STEP gives the model. |
| 2 | Parts that are not printed | They are never on the bed. The `.scad` file marks them with `%` (feature 6). |
| 3 | Fixes on by default | No. All fixes are off by default. When the user turns one on, CoCAD stores it per model with the existing mechanism (`models.json`). |
| 4 | Orientation | Custom orientation is an option, off by default. With it on: Auto, or the user selects the face on the bed. No hint in the model. |
| 5 | More than one plate | Yes, in the first version. |
| 6 | Tab names and keys | The tab is **Optimize**, key `O`. The Photo tab becomes **Render**, key `R`. The tab label underlines the key letter. |
| | Name of the OpenSCAD step | **Build**, so that "Render" names only the Render tab. |
| 7 | OpenSCAD in the `.deb` | Remove it. A check at start shows how to get the nightly. |
| 8 | Printers | A Printer section in the settings, with presets and a default print area for new models. A changed value shows **Custom**. The Optimize tab can set the area of each model. |
| 9 | Joints in the first version | Flat, pins and dovetail. The user can change the joint and its values. |
| 10 | Printed pins | Correct: they are new print parts on the plate. |
| | Engine | OpenSCAD pass (option A). CoCAD requires an OpenSCAD nightly. |

## Open questions

None at this time.

## Order of work

1. OpenSCAD nightly requirement: the check at start, the settings, the package and the docs.
2. Rename Slice to Optimize, Photo to Render and the OpenSCAD step to Build, with the keys and the underlined letters. Collapsible cards. Export card at the bottom (features 7 and 1). UI only.
3. Print area: the Printer section in the settings, with presets, and the Printer card (feature 8). The plate and the fit check need it.
4. Print parts and plates: selection, `%` reference parts, same parts, overlap join, plate layout with more plates, 3MF with separate objects (feature 6).
5. Custom orientation: Auto and Select face (feature 2).
6. The OpenSCAD pass for the fixes, with the bed chamfer first (feature 5). It is the simplest fix and tests the engine.
7. Brim on small islands (feature 4.1).
8. Teardrop holes (feature 3).
9. Bridges (feature 4). The most complex analysis.
10. Check card (port of `overhang.py`).
11. Auto split with flat, pins and dovetail joints (feature 9). Then plug and lap.

Then move the smallest model (`lantern`, 172 lines) to one view as the first test, then the other models.
