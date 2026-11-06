---
trimmedAt: 2026-11-05
---

# CLAUDE.md

## Project

CoCAD (cgeosoft) is a Rust desktop app for OpenSCAD models. It edits Customizer parameters, exports STL, 3MF and STEP, and shows slice and photo previews. The `openscad` CLI makes the geometry. The UI is egui with an own OpenGL renderer.

## Modules

- `src/`: the app. See the "Source layout" table in `README.md`.
- `examples/`: test models. `// expect: error` on the first line marks a model that must fail.
- `scripts/`: build, package, test and asset scripts. Packages go to `dist/`.
- `packaging/`: desktop entry, icon and MIME type for the `.deb`.
- `images/`: README images. The binary embeds the tour screenshots in `images/tour/`.
- `.github/workflows/build.yml`: builds Linux, Windows and macOS on a `v*` tag or a manual start. A tag also makes the release.
- `extras/website/`: static site for cocad.cgeosoft.com.

## Docs and skills

- `README.md` and `extras/website/README.md`: features, CLI, MCP tools, build, CI, tests and deploy.
- `gui-test` skill: screenshots, click tests and MCP tests on a private display.

## Commands

```bash
cargo test
target/debug/cocad params|export|slice|photo model.scad   # without the GUI
scripts/test-models.sh --release
cargo clippy --all-targets -- -D warnings
cargo check --target x86_64-pc-windows-msvc
cargo check --target aarch64-apple-darwin
```

## Rules

1. Run `cargo test` after changes.
2. Run clippy before you tag a release. Keep it clean.
3. Linux (Debian) comes first. Check Windows and macOS with the `cargo check` commands.
4. Do not send mouse or key input to the user's desktop. Use the `gui-test` skill.
5. When a major feature or its UI changes, update the steps in `src/app/tour.rs`.
6. Then run `scripts/tour-shots.sh`. Build again after it.
7. Keep `Depends` in `scripts/package-deb.sh` in sync with new runtime libraries.
8. The repo is public (MIT). Do not commit secrets, personal paths or host names.
9. Git LFS stores JPG, PNG, MP4, STL and 3MF files. Keep them small.
10. `images/`, `/preview.jpg` and website PNG files are plain git.
11. Warn before you delete files. Push only when the owner asks.
12. `origin` is the private main repo. `github` is the public mirror. The branch is `main`.
13. A release bumps the minor version by default. Bump the patch only when the owner says so.
14. When `src/telemetry.rs` sends new data, update the consent text in `src/app.rs`, `README.md` and the website privacy page.
15. Write all repo text in ASD-STE100. Do not use em-dashes, emojis or hype. State only verified facts.
