---
title: SCAD Viewer
subtitle: Desktop viewer for OpenSCAD models with Customizer parameters, export and a slice preview.
slug: scad-viewer
brand: cgeosoft
category: software
status: done
tags: [openscad, 3d-printing, viewer, slicer, rust, egui]
started: 2026-11-01
finished: 2026-11-01
dates_from: "started: first commit date; finished: Christos marked it complete on 2026-11-01"
version: "0.1.10"
site: /b/scad-viewer/
preview: preview.jpg
showcase: true
---

# SCAD Viewer

[![Build](https://github.com/cgeosoft/scad-viewer/actions/workflows/build.yml/badge.svg)](https://github.com/cgeosoft/scad-viewer/actions/workflows/build.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-orange.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-555.svg)

A fast desktop viewer for OpenSCAD models. Open a `.scad` file, change its Customizer parameters with sliders, and see the model update. Export STL, 3MF or STEP, check the print layers, and render a photo of the part.

![SCAD Viewer: slice preview of the honeycomb tray example](images/screen-slice.jpg)

OpenSCAD does the geometry. SCAD Viewer runs the `openscad` CLI in the background, reads the mesh and does the rest. With an OpenSCAD development build (Manifold backend), most models render in less than a second.

## Highlights

- **Customizer panel.** Sliders, dropdowns, switches, text and vectors, in collapsible groups. The model renders again when you change a value.
- **Parameter sets.** Save and load named sets in any `.json` file. OpenSCAD reads the same format.
- **Export.** Binary STL, 3MF with the parameters in the metadata, and STEP AP214 with real planar faces.
- **Slice preview.** Walls, infill and top and bottom skin as round strands, with a layer slider and a time, length and weight estimate.
- **Photo render.** A path tracer makes a studio photo of the current view, in a plastic color and finish you choose.
- **Mesh check.** Size, volume, triangles and open edges. Back faces show in purple.
- **CLI.** Batch export, slice and photo without the window.
- Linux, Windows and macOS. Dark and light themes.

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

## Usage

```bash
scad-viewer model.scad            # open in the model view
scad-viewer --slice model.scad    # open in the slice view
scad-viewer --photo model.scad    # open in the photo view
```

You can also open a file with Ctrl+O or drop it on the window. When the `.scad` file changes on disk, the model reloads and your edited values stay.

| Input | Action |
|---|---|
| Left drag | Orbit |
| Right or middle drag, Shift + left drag | Pan |
| Wheel | Zoom |
| Double click, `F` | Fit the model |
| `F5` | Render now |
| `W` | Wireframe |
| `S` | Model or slice view |
| `P` | Photo view |
| Up / Down, PgUp / PgDn | Change the slice layer |
| Ctrl+O | Open a file |

The status bar at the bottom shows the render state and time, the OpenSCAD version and backend, the model size, triangles and volume, open edges, the slice or photo state, and the **Console** button with the OpenSCAD output.

## Parameter sets

The **Parameter set** card at the top of the panel holds named sets of values.

- The first line shows the sets file. **Open file** loads sets from any `.json` file. **New file** picks a place and name for a new sets file. It is written on the first save.
- The app remembers the sets file for each model. Without one, it uses `<model>.json` next to `<model>.scad`.
- Select a set to load it. **Model defaults** sets all values back to the `.scad` defaults.
- **Save** writes the values to the selected set. With no set selected, it asks for a name.
- **Save as** saves the values as a new set. **Delete** removes the selected set (click two times).
- The file uses the OpenSCAD Customizer format. OpenSCAD reads it too: `openscad -p sets.json -P big`.

## Slice preview

The **Slice** view cuts the model into layers and shows the paths a printer would follow: outer and inner walls, sparse infill, and solid top and bottom skin. The strands have the real line width and layer height. Most models slice in less than 50 ms.

Settings: layer height, first layer height, line width, walls, infill, top and bottom layers, and speed. The card on top shows the print time, the filament length and the weight.

The slicer is for preview only. It has no travel moves, supports or bridges, and it does not write G-code.

## Photo render

![Photo view: path-traced render of the gear example](images/screen-photo.jpg)

The **Photo** view path-traces the current camera view on all CPU cores. The image gets cleaner with each sample. When you move the view or change a setting, the render waits until the view rests, then starts again.

| | | | |
|---|---|---|---|
| ![Gear](images/render-gear.jpg) | ![Honeycomb tray](images/render-honeycomb.jpg) | ![Knob, light studio](images/render-knob.jpg) | ![Twisted vase](images/render-vase.jpg) |

- **Material**: 8 color swatches or a color picker. Finish: matte, satin or gloss plastic.
- **Print layer lines**: ripples the sides at the slice layer height, so the part looks printed.
- **Studio**: dark or light backdrop with a soft key light and floor shadows. Exposure adjusts the brightness.
- **Quality**: samples per pixel (16 to 2048) and the image size. Presets: 720p, 1080p, 1440p, 4K, site preview 4:3 (1600 × 1200), square, portrait 4:5 and story 9:16. You can also type the width and height. **Swap** turns landscape into portrait.
- The view dims everything outside the image frame, so you see what the render will show.
- **Save PNG** writes the image at its full size and the current sample count.

## Export

| Format | Content |
|---|---|
| STL | Binary STL. |
| 3MF | Mesh, with the parameter values in the metadata. |
| STEP | AP214. Coplanar triangles become planar faces. A closed mesh becomes one `FACETED_BREP` solid that CAD tools can edit. |

## Command line

```bash
scad-viewer params model.scad
scad-viewer export model.scad -o model.3mf -D width=80 -D 'shape="square"'
scad-viewer export model.scad -o big.stl -P big          # saved set 'big' from model.json
scad-viewer slice model.scad --layer-height 0.2 --infill 20 --walls 3
scad-viewer photo model.scad -o model.png --size 1600x1200 --samples 256 --color eb6f40 --finish satin --layer-lines
```

- `-P name` loads a saved set. `-p file.json` picks the sets file. Without `-p`, the CLI uses the file the app remembers, else `<model>.json`.
- `-D name=value` values apply after the set.
- `photo` renders the iso view to a PNG.

## OpenSCAD engine

OpenSCAD 2021.01 (the Debian package) uses the CGAL backend. CGAL is slow on large models: a model that takes 13 s with CGAL can take 0.3 s with Manifold. OpenSCAD development builds have the Manifold backend.

- With no binary set, SCAD Viewer looks for `openscad-nightly`, then `openscad`, on the PATH and in the usual install folders.
- Open **Settings** to set the OpenSCAD binary (for example a development AppImage), extra arguments and the theme.
- When the binary supports Manifold, SCAD Viewer adds `--backend=manifold`. Turn this off in Settings, or set your own `--backend` argument.
- `$OPENSCAD` and `$OPENSCAD_ARGS` override the saved values.

The settings are saved in the settings folder. The CLI commands use them too.

| System | Settings folder |
|---|---|
| Linux | `~/.config/scad-viewer` |
| Windows | `%APPDATA%\scad-viewer` |
| macOS | `~/Library/Application Support/scad-viewer` |

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
scripts/package-deb.sh      # writes dist/scad-viewer_<version>_<arch>.deb
```

The `.deb` installs the binary, a desktop entry, an icon, the `.scad` MIME type and the examples.

### Continuous integration

GitHub Actions (`.github/workflows/build.yml`) builds and tests on every push:

- Linux: clippy, unit tests, all test models, a `.deb` and a `.tar.gz`.
- Windows: unit tests and a `.zip` (`scripts/package-windows.ps1`).
- macOS: unit tests and a universal `.app` in a `.zip` (`scripts/package-macos.sh`).

A tag `v*` (for example `v0.2.0`) makes a GitHub release with all files attached.

### Source layout

| Path | Content |
|---|---|
| `src/main.rs` | CLI and window start |
| `src/app.rs` | egui application: panels, camera, jobs |
| `src/render.rs` | OpenGL shaders and buffers |
| `src/ui.rs` | Dark and light palettes and widgets |
| `src/params.rs` | Customizer parameter parser |
| `src/presets.rs` | Parameter sets (OpenSCAD `.json` format) |
| `src/openscad.rs` | Runs `openscad` and reads the STL |
| `src/mesh.rs` | Indexed mesh, STL reader, checks |
| `src/export.rs` | STL, 3MF and STEP writers |
| `src/slicer.rs` | Preview slicer |
| `src/pathtrace.rs` | Path tracer for photo renders (BVH, plastic material, studio light) |
| `examples/` | Test models |
| `scripts/` | Build and packaging scripts |
| `packaging/` | Desktop entry, icon, MIME type for the `.deb` |
| `images/` | README images |

Stack: Rust 2024, `eframe`/`egui` 0.31 with the `glow` (OpenGL 3.3) backend and an own renderer in an egui paint callback. `glam` for math, `zip` for 3MF, `rfd` for file dialogs.

## Tests

- `cargo test`: parser, mesh, STL round trip, STEP faces, 3MF zip, parameter sets, slicer skin, path tracer BVH.
- `scripts/test-models.sh`: renders every model in `examples/`, exports STL, 3MF and STEP, and slices it. Use `--release` for speed, or give model files to test only those.
- The STEP export of `examples/demo-box.scad` loads in OpenCASCADE as one valid solid. Its volume is the same as the STL volume.

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

## License

MIT. See [LICENSE](LICENSE).
