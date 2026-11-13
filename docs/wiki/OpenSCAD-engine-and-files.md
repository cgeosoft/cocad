# OpenSCAD engine and files

*Advanced topic. This page is for technical readers.*

OpenSCAD does the geometry. CoCAD runs the `openscad` command in the background, reads the mesh that it writes, and does the rest. CoCAD calls this step **build**.

## The Manifold backend

CoCAD needs an OpenSCAD binary with the Manifold backend. The development snapshots have it. The release 2021.01 (the Debian 13 package) has only the CGAL backend, which is slow on large models. A model that takes 13 s with CGAL can take 0.3 s with Manifold.

- CoCAD runs the binary with `--version` and `--help`. The binary has Manifold when its `--help` text names the `--backend` option and `manifold`.
- CoCAD adds `--backend=manifold` to each OpenSCAD run, unless **Extra arguments** has a `--backend` argument. With `--backend=cgal`, the settings page shows a warning.
- When the binary has no Manifold, the window shows a notice with the install steps and builds no model. See [Install](Install#if-cocad-does-not-find-openscad-with-manifold). `cocad export`, `cocad slice`, `cocad photo` and `cocad mcp` stop with an error that shows the same steps.
- The status bar shows the OpenSCAD version. It is red when the binary has no Manifold.

The print fixes and the splits of the [Optimize](Optimize) tab also run OpenSCAD: CoCAD writes a small `.scad` file that imports the part mesh and adds or cuts simple shapes. This is why CoCAD needs the fast Manifold backend.

## Which binary CoCAD uses

1. `$OPENSCAD`, if it is set.
2. The **Binary** of **File > Settings > OpenSCAD**, if it is set.
3. `openscad-nightly`, then `openscad`, on the `PATH`, then the usual install folders: `C:\Program Files\OpenSCAD (Nightly)`, `C:\Program Files\OpenSCAD`, `/Applications/OpenSCAD (Nightly).app`, `/Applications/OpenSCAD.app`, `/opt/homebrew/bin` and `/usr/local/bin`. CoCAD uses the first binary that has Manifold. When none has it, CoCAD uses the first binary that it found.

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
| `config` | The settings, with the default print area and the cards that you closed |
| `recent` | The list of recent models |
| `last-sets` | The set that you used last for each model |
| `models.json` | For each model: the settings of the right panel, the Optimize values (print area, fixes, orientation, splits, copies), the closed parameter groups and your last parameter values (last 100 models) |
| `mcp/` | The addresses of the open windows, for the [MCP server](MCP-reference) |
| `examples/` | The copies of the example models |
| `thumbs/` | A 512 x 512 picture of the Iso view of each model, for the start page |
| `tour_seen` | The marker of the welcome tour. Delete it to show the tour at the next start. |

## Updates

When the window opens and each hour after that, CoCAD asks GitHub for the list of releases.

- For a latest version of 0.4.5, the list of changes shows 0.4.0 to 0.4.5, plus all versions between your version and 0.4.0.
- The release workflow writes the notes from the commit subjects since the last tag. It leaves out `Version` and `Website:` commits. Each `*` line in the notes is one bullet in the app.
