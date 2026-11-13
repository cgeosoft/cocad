#!/usr/bin/env python3
"""Makes the generated files of the website in extras/website/public/assets/.

- mesh/<model>/<variant>.bin and mesh/models.json: real meshes of the example
  models. The app CLI builds and slices them as built (`--as-built`, without
  the print preparation of the Optimize tab), so the sizes, times and
  estimates on the site are measured values.
- img/: site images from extras/website/images/, as WebP.

Needs a release build, OpenSCAD and ImageMagick.
Usage: scripts/website-assets.py [--meshes-only | --images-only]
"""
import itertools
import json
import os
import re
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target/release/cocad"
OUT = ROOT / "extras/website/public/assets"

# Only these values are in the web demo. The first file of a model is its default.
MODELS = {
    "vase": {
        "title": "Twisted vase",
        "file": "examples/vase.scad",
        "groups": [
            ("Shape", [
                {"name": "sides", "label": "Sides", "type": "slider", "values": [3, 5, 7], "hint": "[3:1:16]"},
                {"name": "twist", "label": "Twist", "type": "slider", "values": [0, 120, 240, 360], "hint": "[0:5:360]"},
            ]),
        ],
        "defaults": {"sides": 7, "twist": 120},
    },
    "gear": {
        "title": "Spur gear",
        "file": "examples/gear.scad",
        "groups": [
            ("Gear", [
                {"name": "teeth", "label": "Teeth", "type": "slider", "values": [12, 16, 20, 24, 32, 40], "hint": "[8:1:80]"},
            ]),
            ("Hub", [
                {"name": "hub", "label": "Hub", "type": "switch", "values": [True, False]},
                {"name": "hub_height", "label": "Hub height", "type": "slider", "values": [3, 6, 12], "hint": "[0:0.5:30]"},
            ]),
        ],
        "defaults": {"teeth": 24, "hub": True, "hub_height": 6},
    },
    "box": {
        "title": "Demo box",
        "file": "examples/demo-box.scad",
        "groups": [
            ("Size", [
                {"name": "width", "label": "Width", "type": "slider", "values": [40, 60, 90], "hint": "[20:150]"},
            ]),
            ("Style", [
                {"name": "corners", "label": "Corners", "type": "select", "values": ["round", "square"]},
                {"name": "compartments", "label": "Compartments", "type": "select", "values": [1, 2, 3, 4]},
            ]),
        ],
        "defaults": {"width": 60, "corners": "round", "compartments": 2},
    },
}

IMAGES = {
    "extras/website/images/screen-model.jpg": ("screen-model", 1600),
    "extras/website/images/screen-optimize.jpg": ("screen-optimize", 1600),
    "extras/website/images/screen-fixes.jpg": ("screen-fixes", 1600),
    "extras/website/images/screen-layers.jpg": ("screen-layers", 1600),
    "extras/website/images/screen-render.jpg": ("screen-render", 1600),
    "extras/website/images/screen-menu.jpg": ("screen-menu", 1600),
    "extras/website/images/screen-sets.jpg": ("screen-sets", 1600),
    "extras/website/images/screen-export.jpg": ("screen-export", 1600),
    "extras/website/images/screen-agent.jpg": ("screen-agent", 1600),
    "extras/website/images/render-gear.jpg": ("render-gear", 1000),
    "extras/website/images/render-honeycomb.jpg": ("render-honeycomb", 1000),
    "extras/website/images/render-knob.jpg": ("render-knob", 1000),
    "extras/website/images/render-vase.jpg": ("render-vase", 1000),
}


def read_stl(path):
    data = path.read_bytes()
    count = struct.unpack_from("<I", data, 80)[0]
    if len(data) != 84 + count * 50:
        raise SystemExit(f"{path}: not a binary STL")
    verts, index, lookup = [], [], {}
    for i in range(count):
        tri = struct.unpack_from("<9f", data, 84 + i * 50 + 12)
        for j in range(3):
            key = tri[j * 3 : j * 3 + 3]
            if key not in lookup:
                lookup[key] = len(verts)
                verts.append(key)
            index.append(lookup[key])
    return verts, index


def measure(verts, index):
    volume = 0.0
    edges = {}
    for i in range(0, len(index), 3):
        a, b, c = (verts[index[i + j]] for j in range(3))
        volume += (
            a[0] * (b[1] * c[2] - b[2] * c[1])
            - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0])
        ) / 6
        for p, q in ((index[i], index[i + 1]), (index[i + 1], index[i + 2]), (index[i + 2], index[i])):
            key = (min(p, q), max(p, q))
            edges[key] = edges.get(key, 0) + 1
    low = [min(v[k] for v in verts) for k in range(3)]
    high = [max(v[k] for v in verts) for k in range(3)]
    return low, high, abs(volume) / 1000, sum(1 for n in edges.values() if n != 2)


def write_mesh(path, verts, index, low, high):
    """Header: count of vertices and indices, box low, box size. Then Uint16
    positions, scaled to the box, and Uint16 indices."""
    if len(verts) > 65535:
        raise SystemExit(f"{path}: too many vertices")
    size = [max(h - l, 1e-6) for l, h in zip(low, high)]
    out = bytearray(struct.pack("<II6f", len(verts), len(index), *low, *size))
    for v in verts:
        out += struct.pack("<3H", *(round((v[k] - low[k]) / size[k] * 65535) for k in range(3)))
    out += struct.pack(f"<{len(index)}H", *index)
    path.write_bytes(out)


def variant_name(values):
    return "-".join(str(int(v) if isinstance(v, bool) else v) for v in values)


def defines(names, values):
    args = []
    for name, value in zip(names, values):
        text = json.dumps(value)
        args += ["-D", f"{name}={text}"]
    return args


def run(*args):
    done = subprocess.run([str(BIN), *args], capture_output=True, text=True, cwd=ROOT)
    if done.returncode:
        raise SystemExit(f"{' '.join(args)}\n{done.stderr}")
    return done.stdout + done.stderr


def make_meshes(work):
    manifest = {}
    for slug, model in MODELS.items():
        params = [p for _, group in model["groups"] for p in group]
        names = [p["name"] for p in params]
        folder = OUT / "mesh" / slug
        folder.mkdir(parents=True, exist_ok=True)
        variants = {}
        for values in itertools.product(*(p["values"] for p in params)):
            name = variant_name(values)
            args = defines(names, values)
            stl = work / "out.stl"
            log = run("export", model["file"], "-o", str(stl), "--as-built", *args)
            seconds = float(re.search(r"in ([\d.]+) s", log).group(1))
            verts, index = read_stl(stl)
            low, high, volume, open_edges = measure(verts, index)
            write_mesh(folder / f"{name}.bin", verts, index, low, high)
            log = run("slice", model["file"], "--as-built", *args, "--layer-height", "0.2", "--infill", "20", "--walls", "3")
            variants[name] = {
                "seconds": seconds,
                "triangles": len(index) // 3,
                "size": [round(h - l, 1) for l, h in zip(low, high)],
                "volume": round(volume, 2),
                "open_edges": open_edges,
                "layers": int(re.search(r"layers\s+(\d+)", log).group(1)),
                "filament_m": float(re.search(r"filament\s+([\d.]+) m", log).group(1)),
                "grams": float(re.search(r", ([\d.]+) g", log).group(1)),
                "print_time": re.search(r"print time\s+(.+?) \(rough\)", log).group(1),
                "slice_ms": int(re.search(r"sliced in\s+(\d+) ms", log).group(1)),
            }
            print(slug, name, variants[name]["triangles"], "triangles", seconds, "s", flush=True)
        manifest[slug] = {
            "title": model["title"],
            "file": Path(model["file"]).name,
            "groups": [{"name": n, "params": g} for n, g in model["groups"]],
            "defaults": model["defaults"],
            "variants": variants,
        }
    (OUT / "mesh/models.json").write_text(json.dumps(manifest, separators=(",", ":")) + "\n")


def make_images():
    img = OUT / "img"
    img.mkdir(parents=True, exist_ok=True)
    for source, (name, width) in IMAGES.items():
        subprocess.run(
            ["magick", str(ROOT / source), "-resize", f"{width}x>", "-quality", "82", str(img / f"{name}.webp")],
            check=True,
        )
        print(name)


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else ""
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        # Private settings: the run must not depend on or change the saved ones.
        os.environ["XDG_CONFIG_HOME"] = str(work / "config")
        os.environ["XDG_DATA_HOME"] = str(work / "data")
        if only != "--images-only":
            make_meshes(work)
        if only != "--meshes-only":
            make_images()


if __name__ == "__main__":
    main()
