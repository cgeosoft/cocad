---
trimmedAt: 2026-11-05
---

# CLAUDE.md

## Project

CoCAD (cgeosoft) is a Rust desktop app for OpenSCAD models. It edits Customizer parameters, exports STL, 3MF and STEP, and shows slice and photo previews. The `openscad` CLI makes the geometry. The UI is egui with an own OpenGL renderer.

## Modules

- `src/`: the app. See the "Source layout" table in `docs/wiki/Build-from-source.md`.
- `examples/`: test models. `// expect: error` on the first line marks a model that must fail. Each menu model has a `<model>.yml` definition with its title, part names and parameter sets. The `.scad` files hold only OpenSCAD code.
- `scripts/`: build, package, test and asset scripts. Packages go to `dist/`.
- `packaging/`: desktop entry, icon and MIME type for the `.deb`.
- `assets/preview.png`: the only README image.
- `assets/tour/`: the tour screenshots. The binary embeds them.
- `assets/fixes/`: the pictures of the print fix info windows. The binary embeds them. `scripts/showcase-shots.sh` makes them.
- `assets/examples/`: thumbnails of the examples. The binary embeds them. Run `scripts/example-thumbs.sh` when an example changes.
- `extras/website/images/`: full-size screenshots and renders for the website.
- `.github/workflows/build.yml`: builds Linux, Windows and macOS on a `v*` tag or a manual start. A tag also makes the release.
- `extras/website/`: static site for cocad.cgeosoft.com.
- `extras/website/public/schema/model-1.json`: the published JSON Schema of the model definition (`src/definition.rs`).

## Docs and skills

- `README.md`: short overview with links to the wiki.
- `docs/wiki/`: source of the GitHub wiki. Features, CLI, MCP tools, build, CI and tests. The user pages are for non-technical readers. `scripts/check-wiki.sh` checks the links. `scripts/publish-wiki.sh` pushes the pages to the wiki.
- `extras/website/README.md`: website and deploy.
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
5. When a major feature or its UI changes, update the steps in `src/app/tour.rs` and the pages in `docs/wiki/`.
6. Then run `scripts/tour-shots.sh`. Build again after it.
7. Keep `Depends` in `scripts/package-deb.sh` in sync with new runtime libraries.
8. The repo is public (MIT). Do not commit secrets, personal paths or host names.
9. Git LFS stores JPG, PNG, MP4, STL and 3MF files. Keep them small.
10. `/assets/preview.png`, `assets/tour/`, `assets/fixes/`, `assets/examples/`, `extras/website/images/` and website PNG files are plain git.
11. Warn before you delete files. Push only when the owner asks.
12. `origin` is the private main repo. `github` is the public mirror. The branch is `main`.
13. A release bumps the minor version by default. Bump the patch only when the owner says so.
14. When `src/telemetry.rs` sends new data, follow `docs/telemetry-consent.md`. Raise `NOTICE_VERSION` and update the consent text in `src/app.rs`, `README.md`, `docs/wiki/Privacy.md` and the website Privacy Notice.
15. Write all repo text in ASD-STE100. Do not use em-dashes, emojis or hype. State only verified facts.
16. A change of the model definition format that old files cannot read needs a new `VERSION` in `src/definition.rs` and a new schema file. Keep the reader and `schema/model-1.json` in sync: a test compares their keys. Update `docs/wiki/Model-definition.md`.
