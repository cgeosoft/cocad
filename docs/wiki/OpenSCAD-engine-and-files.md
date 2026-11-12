# OpenSCAD engine and files

*Advanced topic. This page is for technical readers.*

OpenSCAD does the geometry. CoCAD runs the `openscad` command in the background, reads the mesh that it writes, and does the rest.

## Backends: CGAL and Manifold

OpenSCAD 2021.01 (the Debian package) uses the CGAL backend. CGAL is slow on large models. OpenSCAD development builds have the Manifold backend. A model that takes 13 s with CGAL can take 0.3 s with Manifold.

When the binary supports Manifold, CoCAD adds `--backend=manifold`. To stop this, turn off **Manifold backend** in **File > Settings > OpenSCAD**, or set your own `--backend` argument.

The status bar shows the OpenSCAD version and the backend.

## Which binary CoCAD uses

1. `$OPENSCAD`, if it is set.
2. The **Binary** of **File > Settings > OpenSCAD**, if it is set.
3. `openscad-nightly`, then `openscad`, on the `PATH` and in the usual install folders: `C:\Program Files\OpenSCAD*`, `/Applications/OpenSCAD*.app` and Homebrew.

**Extra arguments** in the settings go to each OpenSCAD run. `$OPENSCAD_ARGS` overrides them.

The [command line](Command-line) uses the same settings.

## The settings folder

| System | Settings folder |
|---|---|
| Linux | `~/.config/cocad` |
| Windows | `%APPDATA%\cocad` |
| macOS | `~/Library/Application Support/cocad` |

| Item | Content |
|---|---|
| `config` | The settings |
| `recent` | The list of recent models |
| `sets-files` | The sets file of each model |
| `last-sets` | The set that you used last for each model |
| `mcp/` | The addresses of the open windows, for the [MCP server](MCP-reference) |
| `examples/` | The copies of the example models |
| `tour_seen` | The marker of the welcome tour. Delete it to show the tour at the next start. |

## Updates

When the window opens and once a day after that, CoCAD asks GitHub for the list of releases.

- For a latest version of 0.4.5, the list of changes shows 0.4.0 to 0.4.5, plus all versions between your version and 0.4.0.
- The release workflow writes the notes from the commit subjects since the last tag. It leaves out `Version` and `Website:` commits. Each `*` line in the notes is one bullet in the app.
