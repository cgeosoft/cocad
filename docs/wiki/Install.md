# Install

CoCAD needs two programs on your computer:

1. **OpenSCAD.** This program calculates the shape of the model. CoCAD uses it in the background. You do not need to open it.
2. **CoCAD.** This is the app with the window that you use.

## Step 1: install OpenSCAD

Download OpenSCAD from <https://openscad.org/downloads.html#snapshots>.

Use a **development snapshot** if you can. A development snapshot is much faster than the 2021 release. With a development snapshot, most models show in less than a second.

## Step 2: install CoCAD

Download the file for your system from the [releases page](https://github.com/cgeosoft/cocad/releases).

| System | File | How to install |
|---|---|---|
| Debian, Ubuntu | `cocad_<version>_amd64.deb` | Run `sudo apt install ./cocad_<version>_amd64.deb` in a terminal. |
| Other Linux | `cocad-<version>-linux-x86_64.tar.gz` | Unpack the file and run `cocad`. The system must have GTK 3 and OpenGL. |
| Windows | `cocad-<version>-windows-x86_64.zip` | Unpack the file and run `cocad.exe`. |
| macOS (Apple Silicon and Intel) | `cocad-<version>-macos-universal.zip` | Unpack the file and move **CoCAD** to Applications. |

**macOS:** the app is not signed. The first time, right-click the app and choose **Open**.

## Step 3: start CoCAD

Start CoCAD. At the first start, CoCAD asks about your [privacy choices](Privacy) and shows a welcome tour.

Look at the bottom of the window. The status bar shows the OpenSCAD version when CoCAD found OpenSCAD.

## If CoCAD does not find OpenSCAD

CoCAD looks for OpenSCAD in the usual install folders. If it does not find the program:

1. Open **File > Settings**.
2. Select the **OpenSCAD** page.
3. Click **Browse** and select the OpenSCAD program.
4. Click **Save**.

## Updates

CoCAD does not install updates. When a newer version exists, a small notice shows at the bottom right of the window. Click the notice to see the changes and to open the download page. See [Settings](Settings#updates).

## Next step

Go to [First steps](First-steps).

---

**For technical readers.** CoCAD looks for `openscad-nightly`, then `openscad`, on the `PATH`, in `C:\Program Files\OpenSCAD*`, in `/Applications/OpenSCAD*.app` and in Homebrew. See [OpenSCAD engine and files](OpenSCAD-engine-and-files). To build the app yourself, see [Build from source](Build-from-source).
