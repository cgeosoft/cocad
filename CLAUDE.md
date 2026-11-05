# CLAUDE.md

## What this repo is

SCAD Viewer (cgeosoft). A Rust desktop app that shows OpenSCAD models, edits Customizer parameters, exports STL/3MF/STEP and shows a rough slice preview. OpenSCAD does the geometry through its CLI. The UI is egui with an own OpenGL renderer.

## Layout

- `README.md` - write-up.
- `src/` - the app. See the layout table in `README.md`.
- `examples/` - test models. `scripts/test-models.sh` runs them all. `// expect: error` marks models that must fail.
- `scripts/build.sh` - tests and release build. `scripts/package-deb.sh`, `package-windows.ps1`, `package-macos.sh` - packages in `dist/`.
- `.github/workflows/build.yml` - CI for Linux, Windows and macOS; releases on `v*` tags.
- `packaging/` - desktop entry, icon and MIME type for the `.deb`.

## Working

- Run `cargo test` after changes. Use `target/debug/scad-viewer export|slice|params` to test without the GUI.
- The GUI may run on the user's live desktop. Do not send mouse or key input to it. For screenshots, start it with `--slice` or `--photo` when needed and capture the window.
- To test clicks, start a private display (`Xephyr :9 -screen 1600x1040 -ac`), run the app there with `DISPLAY=:9 LIBGL_ALWAYS_SOFTWARE=1`, and send input with `xdotool` on `:9` only. Use separate `XDG_DATA_HOME` and `XDG_CONFIG_HOME`, so the user's window state and recent list stay as they are.
- Targets: Linux (Debian first), Windows and macOS. GitHub Actions builds all three. Check other platforms locally with `cargo check --target x86_64-pc-windows-msvc` and `--target aarch64-apple-darwin`.
- Keep `Depends` in `scripts/package-deb.sh` in sync with new runtime libraries.
- CI runs `cargo clippy --all-targets -- -D warnings`. Keep it clean.
- The repo is public on GitHub (MIT). Do not commit secrets, personal paths or host names.
- `dist/` and `target/` are not in git.

## Conventions

- The project metadata is in the `scad-viewer` entry of `/mnt/files/Projects/cgeofab/src/_data/library.yml`, not in the README.

- Git LFS stores images, video, STL and 3MF files (`.gitattributes`). Exception: `images/` (README images) is plain git, so GitHub shows it. Keep these files small.
- Never take dates from file modification times. Record the source in `dates_from`.
- Warn before deleting files or pushing.
- Remote `origin` is the private main repo; `github` is the public mirror. Branch `main`. Push only when the owner asks.

## Writing style

Plain technical English (ASD-STE100), short sentences, active voice. No em-dashes, no emojis, no hype. State only verified facts.
