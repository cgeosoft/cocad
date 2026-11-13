# Install

CoCAD needs two programs on your computer:

1. **OpenSCAD**, a development snapshot. This program calculates the shape of the model. CoCAD uses it in the background. You do not need to open it.
2. **CoCAD.** This is the app with the window that you use.

## Step 1: install an OpenSCAD development snapshot

CoCAD builds the models with the **Manifold** backend of OpenSCAD. The development snapshots (nightly builds) of OpenSCAD have it. The release 2021.01 does not. The `openscad` package of Debian 13 is the release 2021.01, so it does not work with CoCAD.

The download page is <https://openscad.org/downloads.html#snapshots>.

### Debian and Ubuntu

1. Add the OpenSCAD repository on `download.opensuse.org` (`home:/t-paul`) to the package sources. On Debian 13, the source line is:

   ```
   deb https://download.opensuse.org/repositories/home:/t-paul/Debian_13/ ./
   ```

   Other releases use another folder name.
2. Add the signing key of the repository, as the OpenSCAD download page shows.
3. Install the package `openscad-nightly`:

   ```bash
   sudo apt update && sudo apt install openscad-nightly
   ```

### Windows

1. Download a development snapshot for Windows from the OpenSCAD download page.
2. Install it. CoCAD looks in `C:\Program Files\OpenSCAD (Nightly)` and `C:\Program Files\OpenSCAD`.

### macOS

1. Download a development snapshot for macOS from the OpenSCAD download page.
2. Install it in the **Applications** folder.

## Step 2: install CoCAD

Download the file for your system from the [releases page](https://github.com/cgeosoft/cocad/releases).

| System | File | How to install |
|---|---|---|
| Debian, Ubuntu | `cocad_<version>_amd64.deb` | Run `sudo apt install ./cocad_<version>_amd64.deb` in a terminal. The package does not install OpenSCAD. |
| Other Linux | `cocad-<version>-linux-x86_64.tar.gz` | Unpack the file and run `cocad`. The system must have GTK 3 and OpenGL. |
| Windows | `cocad-<version>-windows-x86_64.zip` | Unpack the file and run `cocad.exe`. |
| macOS (Apple Silicon and Intel) | `cocad-<version>-macos-universal.zip` | Unpack the file and move **CoCAD** to Applications. |

**macOS:** the app is not signed. The first time, right-click the app and choose **Open**.

## Step 3: start CoCAD

Start CoCAD. At the first start, CoCAD asks about your [privacy choices](Privacy) and shows a welcome tour.

Look at the bottom of the window. The status bar shows the OpenSCAD version when CoCAD found OpenSCAD.

## If CoCAD does not find OpenSCAD with Manifold

When CoCAD finds no OpenSCAD with the Manifold backend, it shows the notice **CoCAD needs OpenSCAD with Manifold** at the start. CoCAD does not build a model until it finds a development snapshot. The notice shows:

- **Found**: the OpenSCAD version that CoCAD found, or "not found".
- **Binary**: the program that CoCAD tried.
- **Steps**: the install steps for your system, the same as in step 1. Click **Copy** to copy a command.
- A link to the development snapshots on the OpenSCAD download page.

After you install a snapshot, click **Check again**.

If you installed the snapshot in a different folder:

1. Click **OpenSCAD settings** in the notice, or open **File > Settings** and select the **OpenSCAD** page.
2. Click **Browse** and select the OpenSCAD program.
3. Click **Save**.

## Updates

CoCAD does not install updates. When a newer version exists, a small notice shows at the bottom right of the window. Click the notice to see the changes and to open the download page. See [Settings](Settings#updates).

## Next step

Go to [First steps](First-steps).

---

**For technical readers.** CoCAD looks for `openscad-nightly`, then `openscad`, on the `PATH`, then in `C:\Program Files\OpenSCAD (Nightly)`, `C:\Program Files\OpenSCAD`, `/Applications/OpenSCAD (Nightly).app`, `/Applications/OpenSCAD.app` and Homebrew. It uses the first program that has the Manifold backend. See [OpenSCAD engine and files](OpenSCAD-engine-and-files). To build the app yourself, see [Build from source](Build-from-source).
