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
version: "0.1.10"
showcase: true
---

# SCAD Viewer

[![Build](https://github.com/cgeosoft/scad-viewer/actions/workflows/build.yml/badge.svg)](https://github.com/cgeosoft/scad-viewer/actions/workflows/build.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-orange.svg)](LICENSE)

SCAD Viewer opens an OpenSCAD `.scad` file and shows the model in 3D. It reads the Customizer parameters of the file and shows them as sliders, lists, check boxes and text fields. When you change a value, the model renders again. You can export the model as STL, 3MF or STEP. A simple slicer shows the layers, walls and infill, with a rough print time and filament use.

OpenSCAD does the geometry. SCAD Viewer runs the `openscad` CLI in the background and reads the result.

It runs on Linux, Windows and macOS.

## Install

1. Install OpenSCAD. A development snapshot is much faster than the 2021 release: <https://openscad.org/downloads.html#snapshots>.
2. Download SCAD Viewer for your system from the [releases page](https://github.com/cgeosoft/scad-viewer/releases):

| System | File | How to install |
|---|---|---|
| Debian, Ubuntu | `scad-viewer_<version>_amd64.deb` | `sudo apt install ./scad-viewer_<version>_amd64.deb` |
| Other Linux | `scad-viewer-<version>-linux-x86_64.tar.gz` | Unpack and run `scad-viewer`. Needs GTK 3 and OpenGL. |
| Windows | `scad-viewer-<version>-windows-x86_64.zip` | Unpack and run `scad-viewer.exe`. |
| macOS (Apple Silicon and Intel) | `scad-viewer-<version>-macos-universal.zip` | Unpack and move **SCAD Viewer** to Applications. The app is not signed: the first time, right-click it and choose **Open**. |

SCAD Viewer finds OpenSCAD on the `PATH` and in the usual install folders (`C:\Program Files\OpenSCAD*`, `/Applications/OpenSCAD*.app`, Homebrew). If it does not, set the binary in **Settings**.

## Features

- 3D view with orbit, pan and zoom, view presets, wireframe and bed grid.
- Customizer panel: groups, sliders, dropdowns, switches, text, vectors. Reset per value or all.
- Groups collapse one by one, or all with one button. A group shows how many of its values you changed.
- Parameter sets: save, load and delete named sets in a `.json` file that you pick. The file uses the OpenSCAD Customizer format.
- Auto render on parameter change (debounced). A new render cancels the old one.
- Auto reload when the `.scad` file changes on disk. Edited values stay.
- Export: binary STL, 3MF (with parameters in the metadata), STEP AP214.
- STEP export merges coplanar triangles into planar faces. A closed mesh becomes one `FACETED_BREP` solid.
- Slice preview: layers, outer and inner walls, sparse infill, and top and bottom skin, drawn as rounded strands at line width and layer height. Layer slider, time, length and weight.
- Photo render: a path tracer renders the current view as a photo. Plastic material in 8 colors or any color, matte, satin or gloss finish, printed layer lines, dark or light studio. Save as PNG.
- Mesh check: size, volume, triangle count, open edges. Back faces show in purple.
- CLI for batch export and slicing.
- Status bar: render state and time, OpenSCAD version and backend, model size, triangles and volume, open edges, slice layer and estimate, photo progress, parameter set, and the Console button.
- The header stays dark in both themes.
- Dark and soft light themes. Choose System, Dark or Light in **Settings**. System follows the desktop.
- The window position and size, and the panel widths, are kept between runs.

The slicer is for preview only. Skin goes on each area that has no material in one of the top/bottom layers above or below it. It has no travel moves, supports or bridges, and it does not write G-code.

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
| `src/ui.rs` | Dark and light palettes, cards, pills, switch, sliders, segmented control |
| `src/params.rs` | Customizer parameter parser |
| `src/presets.rs` | Parameter sets (OpenSCAD `.json` format) |
| `src/openscad.rs` | Runs `openscad` and reads the STL |
| `src/mesh.rs` | Indexed mesh, STL reader, checks |
| `src/export.rs` | STL, 3MF and STEP writers |
| `src/slicer.rs` | Preview slicer |
| `src/pathtrace.rs` | Path tracer for photo renders (BVH, plastic material, studio light) |
| `examples/` | Test models (see below) |
| `scripts/` | Build and packaging scripts |
| `packaging/` | Desktop entry, icon, MIME type for the `.deb` |

## Build

Requirements: Rust (stable, 1.85 or later) and OpenSCAD.

- Debian and Ubuntu also need the GTK 3 and OpenGL development files:

```bash
sudo apt install libgtk-3-dev libxkbcommon-dev libgl-dev
```

- Windows needs the Visual Studio C++ build tools. macOS needs the Xcode command line tools.

Build and run:

```bash
cargo run --release -- examples/demo-box.scad
```

On Linux, the scripts build and test in one step:

```bash
scripts/build.sh            # release build, runs the tests first
scripts/build.sh --debug    # debug build
```

## Packages

GitHub Actions (`.github/workflows/build.yml`) builds and tests on every push:

- Linux: clippy, unit tests, all test models, a `.deb` and a `.tar.gz`.
- Windows: unit tests and a `.zip` (`scripts/package-windows.ps1`).
- macOS: unit tests and a universal `.app` in a `.zip` (`scripts/package-macos.sh`).

A tag `v*` (for example `v0.2.0`) makes a GitHub release with all files attached.

### Debian package

`scripts/package-deb.sh` builds the release binary and writes `dist/scad-viewer_<version>_<arch>.deb`. The package installs the binary, a desktop entry, an icon, the `.scad` MIME type and the example.

```bash
scripts/package-deb.sh
sudo apt install ./dist/scad-viewer_0.1.10_amd64.deb
```

## Usage

```bash
scad-viewer model.scad
scad-viewer --slice model.scad
scad-viewer --photo model.scad
```

| Input | Action |
|---|---|
| Left drag | Orbit |
| Right or middle drag, Shift + left drag | Pan |
| Wheel | Zoom |
| Double click, `F` | Fit the model |
| `F5` | Render now |
| Settings button | Theme (System, Dark, Light), OpenSCAD binary, arguments, Manifold |
| `W` | Wireframe |
| `S` | Model or slice view |
| `P` | Photo view |
| Up / Down, PgUp / PgDn | Change the slice layer |
| Ctrl+O | Open a file. You can also drop a file on the window. |

Batch commands:

```bash
scad-viewer params model.scad
scad-viewer export model.scad -o model.3mf -D width=80 -D 'shape="square"'
scad-viewer export model.scad -o big.stl -P big          # saved set 'big' from model.json
scad-viewer slice model.scad --layer-height 0.2 --infill 20 --walls 3
scad-viewer photo model.scad -o model.png --size 1600x1200 --samples 256 --color eb6f40 --finish satin --layer-lines
```

## Photo render

The **Photo** view path-traces the current camera view on all CPU cores. The image gets cleaner with each sample. When you move the view or change a setting, the render waits until the view rests, then starts again.

- **Material**: 8 color swatches or a color picker. Finish: matte, satin or gloss plastic.
- **Print layer lines**: ripples the sides at the slice layer height, so the part looks printed.
- **Studio**: dark or light backdrop with a soft key light and floor shadows. Exposure adjusts the brightness.
- **Quality**: samples per pixel (16 to 2048) and the image size. Presets: 720p, 1080p, 1440p, 4K, site preview 4:3 (1600 × 1200), square, portrait 4:5 and story 9:16. Width and height can also be typed, and **Swap** turns landscape into portrait.
- The view dims everything outside the image frame, so you see what the render will show.
- **Save PNG** writes the image at its full size and the current sample count.

The `photo` CLI command renders the iso view to a PNG without the window.

## OpenSCAD engine

OpenSCAD 2021.01 (the Debian package) uses the CGAL backend. CGAL is slow on large models: a model that takes 13 s with CGAL can take 0.3 s with Manifold. OpenSCAD development builds have the Manifold backend.

- With no binary set, SCAD Viewer looks for `openscad-nightly`, then `openscad`, on the PATH and in the usual install folders.
- Open **Settings** to set the OpenSCAD binary (for example a development AppImage) and extra arguments.
- When the binary supports Manifold, SCAD Viewer adds `--backend=manifold`. Turn this off in Settings, or set your own `--backend` argument.
- The settings are saved in `config` in the settings folder: `~/.config/scad-viewer` (Linux), `%APPDATA%\\scad-viewer` (Windows) or `~/Library/Application Support/scad-viewer` (macOS). The CLI commands use them too.
- `$OPENSCAD` and `$OPENSCAD_ARGS` override the saved values.
- The 3D view shows the backend after the render time.

## Parameter sets

The **Parameter set** card at the top of the panel holds named sets of values.

- The first line shows the sets file. **Open file** loads sets from any `.json` file. **New file** picks a place and name for a new sets file. It is written on the first save.
- The app remembers the sets file for each model in `sets-files` in the settings folder. Without one, it uses `<model>.json` next to `<model>.scad`.
- Select a set to load it. **Model defaults** sets all values back to the `.scad` defaults.
- **Save** writes the values to the selected set. With no set selected, it asks for a name.
- **Save as** saves the values as a new set. **Delete** removes the selected set (click two times).
- The file uses the OpenSCAD Customizer format. OpenSCAD reads it too: `openscad -p sets.json -P big`.
- The CLI takes `-P name` and `-p file.json`. Without `-p` it uses the file the app remembers, else `<model>.json`. `-D` values apply after the set.

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

## Test models

`scripts/test-models.sh` renders every model in `examples/`, exports STL, 3MF and STEP, and slices it. Use `--release` for speed, or give model files to test only those.

| Model | Tests |
|---|---|
| `demo-box.scad` | Every Customizer widget type, text, hull |
| `gear.scad` | Involute teeth from polygon math, many facets |
| `knob.scad` | rotate_extrude, ribs, D-shaft bore, text |
| `vase.scad` | linear_extrude with twist and scale, sloped walls, 450 layers |
| `bracket.scad` | Holes, slots, a gusset, overhangs |
| `honeycomb-tray.scad` | Many holes per layer, thin webs, `include <lib/hex.scad>` |
| `open-mesh.scad` | Open mesh: open-edge warning, STEP as an open shell |
| `flat-2d.scad` | 2D only: must fail with a clear error |
| `empty.scad` | No geometry: must fail with a clear error |

Models with `// expect: error` on the first line must fail.

## Checks

- `cargo test`: parser, mesh, STL round trip, STEP faces, 3MF zip, parameter sets, slicer skin.
- `scripts/test-models.sh`: all models in `examples/` through render, export and slice.
- The STEP export of `examples/demo-box.scad` loads in OpenCASCADE as one valid solid. Its volume is the same as the STL volume.

## License

MIT. See [LICENSE](LICENSE).
