#!/usr/bin/env python3
# Makes the before and after pictures of the Optimize fixes on the showcase
# model examples/desk-dock.scad, for the wiki guide (assets/guide/).
#
# Usage: scripts/showcase-shots.sh   (runs the showcase tests, then this)
#        python3 -I scripts/showcase-shots.py <stl dir> <out dir>
#
# The STL files come from `cargo test showcase -- --ignored` with
# COCAD_SHOWCASE_OUT=<stl dir>. Needs openscad-nightly and ImageMagick.
import os, struct, subprocess, sys
src, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
work = os.path.join(out, "scad")
os.makedirs(work, exist_ok=True)
env = {k: v for k, v in os.environ.items() if k != "DISPLAY"}

def bounds(name):
    d = open(os.path.join(src, name + ".stl"), "rb").read()
    n = struct.unpack("<I", d[80:84])[0]
    lo = [1e9] * 3; hi = [-1e9] * 3
    for i in range(n):
        f = struct.unpack("<12f", d[84 + i * 50:84 + i * 50 + 48])
        for k in range(3):
            for v in (f[3 + k], f[6 + k], f[9 + k]):
                lo[k] = min(lo[k], v); hi[k] = max(hi[k], v)
    return lo, hi

def imp(name):
    return 'import("%s/%s.stl", convexity = 10);' % (src, name)

def render(tag, body, cam, size=(800, 600), ortho=False, bed=None):
    scad = os.path.join(work, tag + ".scad")
    head = ""
    if bed:
        (x0, y0, x1, y1) = bed
        head = 'color("#c9cbd1") translate([%g, %g, -0.6]) cube([%g, %g, 0.6]);\n' % (x0, y0, x1 - x0, y1 - y0)
    open(scad, "w").write(head + 'color("#f28e2b") ' + body + "\n")
    png = os.path.join(work, tag + ".png")
    args = ["openscad-nightly", "--backend=manifold", "-o", png, "--imgsize=%d,%d" % size,
            "--camera=" + ",".join(str(c) for c in cam), "--colorscheme=Tomorrow", "--render=force"]
    if ortho:
        args.append("--projection=ortho")
    r = subprocess.run(args + [scad], env=env, capture_output=True, text=True)
    if r.returncode != 0 or not os.path.exists(png):
        print("FAILED", tag, r.stderr[-400:])
    return png

def montage(name, panels, title):
    files = []
    for label, png in panels:
        files += ["-label", label, png]
    target = os.path.join(out, name + ".png")
    subprocess.run(["montage"] + files + ["-tile", "%dx1" % len(panels), "-geometry", "+6+6",
                    "-pointsize", "26", "-font", "DejaVu-Sans", "-title", title, target], check=True)
    print("wrote", target)

def cut(name, box):
    (x0, y0, z0, x1, y1, z1) = box
    return "intersection() { %s translate([%g, %g, %g]) cube([%g, %g, %g]); }" % (
        imp(name), x0, y0, z0, x1 - x0, y1 - y0, z1 - z0)

def slab(name, axis, at, lo, hi):
    """A slice 0.1 mm thick across `axis` at `at`, inside the box lo..hi of
    the other two axes, and big enough on its own axis."""
    b0 = list(lo); b1 = list(hi)
    b0.insert(axis, at - 0.05); b1.insert(axis, at + 0.05)
    return cut(name, (b0[0], b0[1], b0[2], b1[0], b1[1], b1[2]))

# 1. Plate bridges: a slice through the screw counterbore (x -96, y 13),
#    seen from the front.
cb = (-96, 13)
s1 = lambda n: slab(n, 1, cb[1], (cb[0] - 9, -1), (cb[0] + 9, 7))
cam = [cb[0], cb[1], 3, 90, 0, 0, 22]
montage("1-plate-bridges", [
    ("Before", render("p-b", s1("plate-before"), cam, ortho=True)),
    ("Bridges: sacrificial layers", render("p-s", s1("plate-bridges-sacrificial"), cam, ortho=True)),
    ("Bridges: stepped", render("p-t", s1("plate-bridges-stepped"), cam, ortho=True)),
], "Base plate: screw counterbore, bed side down (section)")

# 2. The layer under the counterbore ceiling, from above.
s2 = lambda n: slab(n, 2, 2.5, (cb[0] - 10, cb[1] - 10), (cb[0] + 10, cb[1] + 10))
cam2 = [cb[0], cb[1], 2.5, 0, 0, 0, 30]
montage("2-plate-last-layer", [
    ("Before: round outline", render("q-b", s2("plate-before"), cam2, ortho=True)),
    ("Bridges: square last layer", render("q-s", s2("plate-bridges-sacrificial"), cam2, ortho=True)),
], "Plate: layer under the counterbore ceiling")

# 3. Stand teardrops: a slice at x -30 (arch) and the front face (cable hole).
lo, hi = bounds("stand-before")
s3 = lambda n: slab(n, 0, -30, (lo[1] - 1, -1), (hi[1] + 1, 30))
cam_end = [-30, lo[1] + 16, 12, 90, 0, 90, 48]
cam_front = [0, lo[1], 11, 90, 0, 0, 40]
montage("3-stand-teardrops", [
    ("Before: round arch", render("s-be", s3("stand-before"), cam_end, ortho=True)),
    ("Teardrop holes", render("s-te", s3("stand-teardrops"), cam_end, ortho=True)),
    ("Before: round cable hole", render("s-bf", imp("stand-before"), cam_front, ortho=True)),
    ("Teardrop holes", render("s-tf", imp("stand-teardrops"), cam_front, ortho=True)),
], "Stand: cable arch (section) and cable hole (front)")

# 4. Stand pilot holes: a slice through the pilot hole at x -21.
s4 = lambda n: slab(n, 0, -21, (5, -1), (25, 16))
cam_p = [-21, 15, 7, 90, 0, 90, 26]
montage("4-stand-pilot-holes", [
    ("Before: flat round ceiling", render("c-b", s4("stand-before"), cam_p, ortho=True)),
    ("Bridges: 45 degree cone", render("c-a", s4("stand-bridges-sacrificial"), cam_p, ortho=True)),
], "Stand: blind pilot hole (section)")

# 5. Stand brim: the first layer, from above.
fl = (lo[0] - 8, lo[1] - 8, -1, hi[0] + 8, hi[1] + 8, 0.3)
cam_f = [0, (lo[1] + hi[1]) / 2 - 6, 0, 0, 0, 0, 175]
montage("5-stand-brim", [
    ("Before: thin front foot", render("b-b", cut("stand-before", fl), cam_f, ortho=True)),
    ("Brim on the small island", render("b-a", cut("stand-brim", fl), cam_f, ortho=True)),
], "Stand: first layer, front foot below")

# 6. Bed chamfer: a slice through the plate edge, very close.
plo, phi = bounds("plate-before")
s6 = lambda n: slab(n, 1, 0, (phi[0] - 3, -1), (phi[0] + 1, 3))
cam_c = [phi[0] - 1, 0, 0.8, 90, 0, 0, 5]
montage("6-bed-chamfer", [
    ("Before", render("e-b", s6("plate-before"), cam_c, ortho=True)),
    ("Bed chamfer 0.4 mm (2 layers)", render("e-a", s6("plate-chamfer"), cam_c, ortho=True)),
], "Plate: bed edge (section)")

# 7. Clip orientation.
mlo, mhi = bounds("clip-model")
alo, ahi = bounds("clip-auto")
montage("7-clip-orientation", [
    ("Model rotation: on an edge", render("o-m", imp("clip-model"), [0, 0, 7, 70, 0, 30, 70], bed=(-20, -20, 20, 20))),
    ("Custom orientation: Auto", render("o-a", imp("clip-auto"), [0, 0, 7, 70, 0, 30, 70], bed=(-20, -20, 20, 20))),
], "Clip: 5 problems as modeled, 0 with Auto")

# 8. Split: the joint of each split, from above, and the pin face.
def pair(j, gap):
    a, b = "split-%s-1" % j, "split-%s-2" % j
    alo2, ahi2 = bounds(a); blo2, bhi2 = bounds(b)
    return "union() { translate([%g, 0, 0]) %s translate([%g, 0, 0]) %s }" % (
        -ahi2[0] - gap, imp(a), -blo2[0] + gap, imp(b))
top = [0, 0, 0, 0, 0, 0, 150]
pins = ""
k = 3
while os.path.exists(os.path.join(src, "split-pins-%d.stl" % k)):
    pins += " translate([%g, -52, 0]) %s" % (-12 + 12 * (k - 3), imp("split-pins-%d" % k))
    k += 1
blo2, bhi2 = bounds("split-pins-2")
face = "union() { translate([%g, 0, 0]) %s %s }" % (-blo2[0] + 6, imp("split-pins-2"), pins)
montage("8-plate-split", [
    ("Flat", render("x-flat", pair("flat", 3), top, ortho=True)),
    ("Dovetail", render("x-dove", pair("dovetail", 3), top, ortho=True)),
], "Plate split at x 130: flat and dovetail")

# 9. Combination: all fixes on the stand and the plate.
cam_s = [0, -5, 48, 72, 0, 320, 380]
cam_pl = [0, 0, 0, 145, 0, 20, 360]
montage("9-all-fixes", [
    ("Stand before", render("a-sb", imp("stand-before"), cam_s)),
    ("Stand, all fixes", render("a-sa", imp("stand-all-sacrificial"), cam_s)),
], "Stand: all fixes together")
