# CLAUDE.md

## What this repo is

SCAD Viewer (cgeosoft). A Rust desktop app that shows OpenSCAD models, edits Customizer parameters, exports STL/3MF/STEP and shows a rough slice preview. OpenSCAD does the geometry through its CLI. The UI is egui with an own OpenGL renderer.

## Layout

- `README.md` - write-up.
- `src/` - the app. See the layout table in `README.md`.
- `examples/` - test models. `scripts/test-models.sh` runs them all. `// expect: error` marks models that must fail.
- `scripts/build.sh` - tests and release build. `scripts/package-deb.sh`, `package-windows.ps1`, `package-macos.sh` - packages in `dist/`.
- `.github/workflows/build.yml` - builds Linux, Windows and macOS on `v*` tags and makes the release. A push to `main` starts no build.
- `scripts/tour-shots.sh` - makes the welcome tour screenshots in `images/tour/`. The binary embeds them, so build again after a new run.
- `packaging/` - desktop entry, icon and MIME type for the `.deb`.
- `extras/website/` - website at scadviewer.cgeosoft.com (Cloudflare Pages, static, see its `README.md`). `scripts/website-assets.py` makes its meshes and images.

## Working

- Run `cargo test` after changes. Use `target/debug/scad-viewer export|slice|params` to test without the GUI.
- The GUI may run on the user's live desktop. Do not send mouse or key input to it. For screenshots, start it with `--slice` or `--photo` when needed and capture the window.
- `scad-viewer mcp` talks to a running window (see `src/mcp.rs`). To test it, run both on a private display with the same separate `XDG_CONFIG_HOME`, or the bridge finds the window of the user.
- To test clicks, start a private display (`Xephyr :9 -screen 1600x1040 -ac`), run the app there with `DISPLAY=:9 LIBGL_ALWAYS_SOFTWARE=1`, and send input with `xdotool` on `:9` only. Use separate `XDG_DATA_HOME` and `XDG_CONFIG_HOME`, so the user's window state and recent list stay as they are.
- The welcome tour opens at the first start with a new settings folder, and covers the window. For screenshots and click tests, create an empty `scad-viewer/tour_seen` in the private `XDG_CONFIG_HOME`.
- Update the steps in `src/app/tour.rs` and run `scripts/tour-shots.sh` when a major feature or its UI changes.
- Targets: Linux (Debian first), Windows and macOS. GitHub Actions builds all three on a version tag. Check other platforms locally with `cargo check --target x86_64-pc-windows-msvc` and `--target aarch64-apple-darwin`.
- Keep `Depends` in `scripts/package-deb.sh` in sync with new runtime libraries.
- CI runs `cargo clippy --all-targets -- -D warnings` on release tags. Run it before you tag, and keep it clean.
- The repo is public on GitHub (MIT). Do not commit secrets, personal paths or host names.
- `dist/` and `target/` are not in git.

## Conventions

- The project metadata is in the `scad-viewer` entry of `/mnt/files/Projects/cgeofab/src/_data/library.yml`, not in the README.

- Git LFS stores images, video, STL and 3MF files (`.gitattributes`). Exception: `images/` (README images) is plain git, so GitHub shows it. Keep these files small.
- Never take dates from file modification times. Record the source in `dates_from`.
- Warn before deleting files or pushing.
- Releases bump the minor version by default (0.2.0, 0.3.0). Bump the patch only when the owner says so.
- Remote `origin` is the private main repo; `github` is the public mirror. Branch `main`. Push only when the owner asks.

## Writing style

Plain technical English (ASD-STE100), short sentences, active voice. No em-dashes, no emojis, no hype. State only verified facts.
