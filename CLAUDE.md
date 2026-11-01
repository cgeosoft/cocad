# CLAUDE.md

## What this repo is

SCAD Viewer (cgeosoft). A Rust desktop app that shows OpenSCAD models, edits Customizer parameters, exports STL/3MF/STEP and shows a rough slice preview. OpenSCAD does the geometry through its CLI. The UI is egui with an own OpenGL renderer.

## Layout

- `README.md` - write-up with front matter (see the NoteBook skill `project-repo`, `frontmatter.md`).
- `src/` - the app. See the layout table in `README.md`.
- `examples/demo-box.scad` - test model that uses every Customizer widget.
- `scripts/build.sh` - tests and release build. `scripts/package-deb.sh` - `.deb` in `dist/`.
- `packaging/` - desktop entry, icon and MIME type for the `.deb`.

## Working

- Run `cargo test` after changes. Use `target/debug/scad-viewer export|slice|params` to test without the GUI.
- The GUI runs on Christos's live display (`:0`). Do not send mouse or key input to it. For screenshots, start it with `--slice` when needed and use `import -window <id>`.
- Debian (trixie, amd64) is the first target. Keep `Depends` in `scripts/package-deb.sh` in sync with new runtime libraries.
- `dist/` and `target/` are not in git.

## Conventions

- Git LFS stores images, video, STL and 3MF files (`.gitattributes`).
- Never take dates from file modification times. Record the source in `dates_from`.
- Warn before deleting files or pushing.
- Remote `origin` is `cg@cronus:/mnt/repos/scad-viewer.git`. Branch `main`. Push only when Christos asks.

## Writing style

Plain technical English (ASD-STE100), short sentences, active voice. No em-dashes, no emojis, no hype. State only verified facts.
