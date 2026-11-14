# Build from source

*Advanced topic. This page is for developers.*

## Requirements

- Rust (stable, 1.85 or later)
- An OpenSCAD development snapshot with the Manifold backend. See [Install](Install#step-1-install-an-openscad-development-snapshot).
- Debian and Ubuntu: the GTK 3 and OpenGL development files

```bash
sudo apt install libgtk-3-dev libxkbcommon-dev libgl-dev
```

- Windows: the Visual Studio C++ build tools
- macOS: the Xcode command line tools

## Build and run

```bash
cargo run --release -- examples/demo-box.scad
```

On Linux, the scripts build and test in one step:

```bash
scripts/build.sh            # release build, runs the tests first
scripts/build.sh --debug    # debug build
scripts/package-deb.sh      # writes dist/cocad_<version>_<arch>.deb
```

The `.deb` installs the binary, a desktop entry, an icon, the `.scad` MIME type and the examples. It does not depend on OpenSCAD, because Debian has no OpenSCAD package with Manifold. The notice at the start of the app tells the user how to install a development snapshot.

## Analytics key

A build reads the PostHog project key from `POSTHOG_API_KEY` at compile time. A debug build or a build with no key sends nothing and does not ask the privacy questions. CI uses the `POSTHOG_API_KEY` repository secret for tag builds only. `scripts/build.sh` also reads it from `extras/website/.env` for a release build.

## Continuous integration

GitHub Actions (`.github/workflows/build.yml`) builds and tests on a version tag, or when you start it by hand:

| System | Steps |
|---|---|
| Linux | clippy, unit tests, all test models, a `.deb` and a `.tar.gz` |
| Windows | unit tests and a `.zip` (`scripts/package-windows.ps1`) |
| macOS | unit tests and a universal `.app` in a `.zip` (`scripts/package-macos.sh`) |

A tag `v*` (for example `v0.2.0`) also makes a GitHub release with all files attached. A push to `main` starts no build.

## Tests

- `cargo test`: parser, mesh, STL round trip, STEP faces, 3MF zip, parameter sets, slicer skin, path tracer BVH, GPU tracer data and CPU fallback, parts of the 3D view, and the print preparation: plate layout, same parts, orientation, fixes, check and split.
- `cargo test showcase -- --ignored --nocapture`: the Optimize features on `examples/desk-dock.scad`. These tests need OpenSCAD. With `COCAD_SHOWCASE_OUT=<dir>`, they also write the part meshes before and after each change as STL files.
- `scripts/test-models.sh`: builds every model in `examples/`, exports STL, 3MF and STEP, and slices it. STL and 3MF get the print plate, STEP gets the model as built. Then two print preparation runs: all fixes on `bracket.scad`, and `lid-box.scad` on a small print area, with all parts on one plate. Use `--release` for speed, or give model files to test only those.
- The STEP export of `examples/demo-box.scad` loads in OpenCASCADE as one valid solid. Its volume is the same as the STL volume.

| Model | Tests |
|---|---|
| `demo-box.scad` | Every Customizer widget type, text, hull |
| `gear.scad` | Involute teeth from polygon math, many facets |
| `knob.scad` | rotate_extrude, ribs, D-shaft bore, text |
| `vase.scad` | linear_extrude with twist and scale, sloped walls, 450 layers |
| `bracket.scad` | Holes, slots, a gusset, overhangs |
| `honeycomb-tray.scad` | Many holes per layer, thin webs, `include <lib/hex.scad>` |
| `multicolor-badge.scad` | SVG inlays, one part for each color in touching solids |
| `desk-dock.scad` | The showcase of the Optimize tab: a part too large for the bed, horizontal holes, counterbores with holes above, blind holes, a thin foot, three same parts in a tilted rotation, `%` reference parts |
| `reference-part.scad` | A `%` reference part that is not printed or exported |
| `part-selector.scad` | A `part` selector list: each value builds on its own, and the parts get the labels as names |
| `open-mesh.scad` | Open mesh: open-edge warning, STEP as an open shell |
| `flat-2d.scad` | 2D only: must fail with a clear error |
| `empty.scad` | No geometry: must fail with a clear error |

Models with `// expect: error` on the first line must fail.

## Source layout

| Path | Content |
|---|---|
| `src/main.rs` | CLI and window start |
| `src/app.rs` | egui application: panels, camera, jobs |
| `src/app/remote.rs` | MCP tools inside the window |
| `src/app/select.rs` | Select a part in the 3D view, part menu, focus, color window of one part |
| `src/app/tour.rs` | Welcome tour |
| `src/app/start.rs` | Start page when no model is open, new model and its template |
| `src/app/thumb.rs` | Model thumbnails: drawn when a model opens and closes, start page cache of thumbnails, titles and descriptions |
| `src/thumbs.rs` | Thumbnail files in the settings folder (`thumbs/`) |
| `src/app/optimize_tab.rs` | The Optimize tab: cards, plate view, pipeline runs in the background, plate export, MCP Optimize state |
| `src/app/fix_info.rs` | The info window of each print fix: text and the pictures in `assets/fixes/` |
| `src/mcp.rs` | MCP server, tool list, window listener |
| `src/app/view3d.rs` | 3D view options for the parts: part colors, explode, Parts and Explode cards, part colors window |
| `src/app/state.rs` | State of each model: right panel settings and last parameter values, in `models.json` |
| `src/app/section.rs` | Section cut: planes on X, Y and Z in model or bed coordinates, the Section menu of the 3D view, MCP section state |
| `src/render.rs` | OpenGL shaders and buffers. Draws each part with its own color, offset and transparency, and the section cut with stencil cut faces |
| `src/assembly.rs` | Parts of the 3D view: shells, named parts, part palette, moved mesh for the photo |
| `src/csg_split.rs` | Module split: finds the module calls in the CSG tree and renders each one |
| `src/ui.rs` | Dark and light palettes and widgets |
| `src/params.rs` | Customizer parameter parser |
| `src/definition.rs` | Model definition `<model>.yml`: text, part names and colors, parameter sets. Reads the OpenSCAD `.json` format of older models |
| `src/presets.rs` | Parameter sets: apply, save, delete, the set that loads at open |
| `src/recent.rs` | List of recent models |
| `src/examples.rs` | Example models in the File menu |
| `src/telemetry.rs` | Optional analytics and error reports (PostHog), final allowlist |
| `src/telemetry/consent.rs` | Consent record and migration |
| `src/telemetry/report.rs` | Error report fields, safe stack frames, rate limit |
| `src/update.rs` | Update check on GitHub, list of changes, dismissed version |
| `src/net.rs` | HTTP client setup |
| `src/openscad.rs` | Runs `openscad` and reads the STL |
| `src/mesh.rs` | Indexed mesh, STL reader, checks |
| `src/part_param.rs` | The part list: from the model definition, else from the dropdown list of the part parameter. Builds each value of a part list |
| `src/parts.rs` | Multicolor part lists: renders each part with its color |
| `src/export.rs` | STL, 3MF, multicolor 3MF and STEP writers |
| `src/slicer.rs` | Preview slicer |
| `src/optimize/` | Print preparation: `pipeline.rs` (print parts, plate, cache), `plate.rs` (same parts, overlap, layout), `orient.rs`, `split.rs`, `fixes.rs` with `teardrop.rs`, `holes.rs`, `bridges.rs`, `brim.rs` and `ears.rs`, `check.rs`, `scad.rs` (OpenSCAD runs on part meshes), `headless.rs` (CLI print options and report) |
| `src/printer.rs` | Printer presets and the print area editor |
| `src/pathtrace.rs` | Path tracer for photo renders (BVH, plastic material, studio light, section cut). A job runs on the GPU or on all CPU cores |
| `src/pathtrace/gpu.rs` | GPU path tracer: scene textures, tiles for each frame, CPU fallback when the GPU stops |
| `src/pathtrace/trace.glsl` | Fragment shader of the GPU path tracer. It follows the CPU code in `pathtrace.rs` |
| `examples/` | Test models. The binary embeds the models that render. |
| `scripts/` | Build and packaging scripts |
| `packaging/` | Desktop entry, icon, MIME type for the `.deb` |
| `assets/tour/` | Tour screenshots, embedded in the binary |
| `assets/examples/` | Thumbnails of the examples (`scripts/example-thumbs.sh`), embedded in the binary |
| `assets/guide/` | Screenshots (`scripts/guide-shots.sh`) and before and after pictures of the guide [Prepare a model for printing](Prepare-a-model-for-printing) |
| `docs/wiki/` | The source of these wiki pages. `scripts/publish-wiki.sh` copies them to the GitHub wiki. |

Stack: Rust 2024, `eframe`/`egui` 0.31 with the `glow` (OpenGL 3.3) backend and an own renderer in an egui paint callback. `glam` for math, `zip` for 3MF, `rfd` for file dialogs.

## Change these pages

Edit the files in `docs/wiki/` and run `scripts/publish-wiki.sh`. Do not edit the pages on the wiki website, because the script replaces them.
