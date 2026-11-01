---
title: SCAD Viewer
subtitle: Desktop viewer for OpenSCAD models with Customizer parameters, export and a slice preview.
slug: scad-viewer
brand: cgeosoft
category: software
status: in-progress
tags: [openscad, 3d-printing, viewer, slicer, rust, egui]
started: 2026-11-01
dates_from: "first commit date"
version: "0.1.2"
---

# SCAD Viewer

SCAD Viewer opens an OpenSCAD `.scad` file and shows the model in 3D. It reads the Customizer parameters of the file and shows them as sliders, lists, check boxes and text fields. When you change a value, the model renders again. You can export the model as STL, 3MF or STEP. A simple slicer shows the layers, walls and infill, with a rough print time and filament use.

OpenSCAD does the geometry. SCAD Viewer runs the `openscad` CLI in the background and reads the result.

## Features

- 3D view with orbit, pan and zoom, view presets, wireframe and bed grid.
- Customizer panel: groups, sliders, dropdowns, check boxes, text, vectors. Reset per value or all.
- Auto render on parameter change (debounced). A new render cancels the old one.
- Auto reload when the `.scad` file changes on disk. Edited values stay.
- Export: binary STL, 3MF (with parameters in the metadata), STEP AP214.
- STEP export merges coplanar triangles into planar faces. A closed mesh becomes one `FACETED_BREP` solid.
- Slice preview: layers, outer and inner walls, sparse and solid infill. Layer slider, time, length and weight.
- Mesh check: size, volume, triangle count, open edges. Back faces show in purple.
- CLI for batch export and slicing.

The slicer is for preview only. It has no travel moves, supports, bridges or real top-surface detection. It does not write G-code.

## Stack

- Rust 2024, `eframe`/`egui` 0.31 with the `glow` (OpenGL 3.3) backend.
- Own OpenGL renderer in an egui paint callback.
- `glam` for math, `zip` for 3MF, `rfd` (GTK3) for file dialogs.
- OpenSCAD 2021.01 or later at run time.

## Layout

| Path | Content |
|---|---|
| `src/main.rs` | CLI and window start |
| `src/app.rs` | egui application: panels, camera, jobs |
| `src/render.rs` | OpenGL shaders and buffers |
| `src/params.rs` | Customizer parameter parser |
| `src/openscad.rs` | Runs `openscad` and reads the STL |
| `src/mesh.rs` | Indexed mesh, STL reader, checks |
| `src/export.rs` | STL, 3MF and STEP writers |
| `src/slicer.rs` | Preview slicer |
| `examples/` | Demo model with every Customizer widget |
| `scripts/` | Build and packaging scripts |
| `packaging/` | Desktop entry, icon, MIME type for the `.deb` |

## Build

Requirements: Rust (stable), OpenSCAD, and the GTK3 and OpenGL development files.

```bash
sudo apt install openscad libgtk-3-dev libxkbcommon-dev libgl-dev
```

Build and test:

```bash
scripts/build.sh            # release build, runs the tests first
scripts/build.sh --debug    # debug build
```

## Debian package

`scripts/package-deb.sh` builds the release binary and writes `dist/scad-viewer_<version>_<arch>.deb`. The package installs the binary, a desktop entry, an icon, the `.scad` MIME type and the example.

```bash
scripts/package-deb.sh
sudo apt install ./dist/scad-viewer_0.1.2_amd64.deb
```

## Usage

```bash
scad-viewer model.scad
scad-viewer --slice model.scad
```

| Input | Action |
|---|---|
| Left drag | Orbit |
| Right or middle drag, Shift + left drag | Pan |
| Wheel | Zoom |
| Double click, `F` | Fit the model |
| `F5` | Render now |
| Settings button | OpenSCAD binary, arguments, Manifold |
| `W` | Wireframe |
| `S` | Model or slice view |
| Up / Down, PgUp / PgDn | Change the slice layer |
| Ctrl+O | Open a file. You can also drop a file on the window. |

Batch commands:

```bash
scad-viewer params model.scad
scad-viewer export model.scad -o model.3mf -D width=80 -D 'shape="square"'
scad-viewer slice model.scad --layer-height 0.2 --infill 20 --walls 3
```

## OpenSCAD engine

OpenSCAD 2021.01 (the Debian package) uses the CGAL backend. CGAL is slow on large models: `clip-on-case.scad` takes 13 s. OpenSCAD development builds have the Manifold backend, which is much faster.

- With no binary set, SCAD Viewer uses `openscad-nightly` when it is on the PATH, else `openscad`.
- Open **Settings** to set the OpenSCAD binary (for example a development AppImage) and extra arguments.
- When the binary supports Manifold, SCAD Viewer adds `--backend=manifold`. Turn this off in Settings, or set your own `--backend` argument.
- The settings are saved in `~/.config/scad-viewer/config`. The CLI commands use them too.
- `$OPENSCAD` and `$OPENSCAD_ARGS` override the saved values.
- The 3D view shows the backend after the render time.

## Customizer syntax

SCAD Viewer reads the same comments as the OpenSCAD Customizer. Parsing stops at the first top-level `module` or `function`.

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

## Checks

- `cargo test`: parser, mesh, STL round trip, STEP faces, 3MF zip, slicer.
- The STEP export of `examples/demo-box.scad` loads in OpenCASCADE as one valid solid. Its volume is the same as the STL volume.
