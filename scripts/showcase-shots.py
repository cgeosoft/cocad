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

ORANGE, BLUE = "#f28e2b", "#2f6fd6"

def render(tag, body, cam, size=(800, 600), ortho=False, bed=None, paint=ORANGE):
    """`paint=None`: the body sets its own colors."""
    scad = os.path.join(work, tag + ".scad")
    head = ""
    if bed:
        (x0, y0, x1, y1) = bed
        head = 'color("#c9cbd1") translate([%g, %g, -0.6]) cube([%g, %g, 0.6]);\n' % (x0, y0, x1 - x0, y1 - y0)
    open(scad, "w").write(head + ('color("%s") ' % paint if paint else "") + body + "\n")
    png = os.path.join(work, tag + ".png")
    args = ["openscad-nightly", "--backend=manifold", "-o", png, "--imgsize=%d,%d" % size,
            "--camera=" + ",".join(str(c) for c in cam), "--colorscheme=Tomorrow", "--render=force"]
    if ortho:
        args.append("--projection=ortho")
    r = subprocess.run(args + [scad], env=env, capture_output=True, text=True)
    if r.returncode != 0 or not os.path.exists(png):
        print("FAILED", tag, r.stderr[-400:])
    return png

def montage(name, panels, title, folder=None):
    files = []
    for label, png in panels:
        files += ["-label", label, png]
    target = os.path.join(folder or out, name + ".png")
    subprocess.run(["montage"] + files + ["-tile", "%dx1" % len(panels), "-geometry", "+6+6",
                    "-pointsize", "26", "-font", "DejaVu-Sans", "-title", title, target], check=True)
    print("wrote", target)

def cut(name, box):
    (x0, y0, z0, x1, y1, z1) = box
    return "intersection() { %s translate([%g, %g, %g]) cube([%g, %g, %g]); }" % (
        imp(name), x0, y0, z0, x1 - x0, y1 - y0, z1 - z0)

def removed(before, after):
    """`after` in orange, and the material that the fix removed in blue."""
    return 'color("%s") %s\ncolor("%s") difference() { %s %s }' % (ORANGE, after, BLUE, before, after)

def added(before, after):
    """`before` in orange, and the material that the fix added in blue."""
    return 'color("%s") %s\ncolor("%s") difference() { %s %s }' % (ORANGE, before, BLUE, after, before)

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

# 3. Stand teardrops. Auto lays the stand on its side: the cable hole goes
#    along y at x 39, z 35, and the pilot holes go along x from the end at
#    x 50, at y 15. Sections across y at 0 and across x at 48.
lo, hi = bounds("stand-before")
s3h = lambda n: slab(n, 1, 0, (29, 25), (49, 45))
cam_h = [39, 0, 35, 90, 0, 0, 30]
s3p = lambda n: slab(n, 0, 48, (5, 5), (25, 25))
cam_p = [48, 15, 15.5, 90, 0, 90, 22]
montage("3-stand-teardrops", [
    ("Before: round cable hole", render("s-bh", s3h("stand-before"), cam_h, ortho=True)),
    ("Teardrop holes", render("s-th", s3h("stand-teardrops"), cam_h, ortho=True)),
    ("Before: round pilot hole", render("s-bp", s3p("stand-before"), cam_p, ortho=True)),
    ("Teardrop holes", render("s-tp", s3p("stand-teardrops"), cam_p, ortho=True)),
], "Stand on its side: cable hole and pilot hole (sections)")

# 4. Hole size: the counterbore section of picture 1, and the layer at
#    z 1.5 through a magnet pocket (x -114, y -24), from above. Blue: the
#    material that the fix removes.
mg = (-114, -24)
s4 = lambda n: slab(n, 2, 1.5, (mg[0] - 8, mg[1] - 8), (mg[0] + 8, mg[1] + 8))
cam4 = [mg[0], mg[1], 1.5, 0, 0, 0, 26]
montage("4-plate-hole-size", [
    ("Before", render("h-b", s1("plate-before"), cam, ortho=True)),
    ("Hole size +0.2 mm (blue: removed)", render("h-a", removed(s1("plate-before"), s1("plate-holes")), cam, ortho=True, paint=None)),
    ("Before: magnet pocket", render("h-mb", s4("plate-before"), cam4, ortho=True)),
    ("Hole size +0.2 mm (blue: removed)", render("h-ma", removed(s4("plate-before"), s4("plate-holes")), cam4, ortho=True, paint=None)),
], "Base plate: screw counterbore (section) and magnet pocket (layer at z 1.5)")

# 5. Brim: the stand upright (custom orientation off), first layer, from
#    above.
ulo, uhi = bounds("stand-upright-before")
fl = (ulo[0] - 8, ulo[1] - 8, -1, uhi[0] + 8, uhi[1] + 8, 0.3)
cam_f = [0, (ulo[1] + uhi[1]) / 2 - 6, 0, 0, 0, 0, 175]
montage("5-stand-brim", [
    ("Before: thin front foot", render("b-b", cut("stand-upright-before", fl), cam_f, ortho=True)),
    ("Brim on the small island", render("b-a", cut("stand-upright-brim", fl), cam_f, ortho=True)),
], "Stand upright: first layer, front foot below")

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
cam_s = [10, 0, 28, 60, 0, 50, 370]
cam_pl = [0, 0, 0, 145, 0, 20, 360]
montage("9-all-fixes", [
    ("Stand before", render("a-sb", imp("stand-before"), cam_s)),
    ("Stand, all fixes", render("a-sa", imp("stand-all-sacrificial"), cam_s)),
], "Stand on its side: all fixes together")

# 10. Mouse ears: the stand on its side. The first layer from above, and
#     the part. Blue: the ears.
slo, shi = bounds("stand-before")
fl10 = (slo[0] - 10, slo[1] - 10, -1, shi[0] + 10, shi[1] + 10, 0.3)
cam10 = [(slo[0] + shi[0]) / 2, (slo[1] + shi[1]) / 2, 0, 0, 0, 0, 240]
montage("10-stand-mouse-ears", [
    ("Before", render("m-b", cut("stand-before", fl10), cam10, ortho=True)),
    ("Mouse ears (blue)", render("m-a", added(cut("stand-before", fl10), cut("stand-ears", fl10)), cam10, ortho=True, paint=None)),
    # Only the first layer of the difference: a difference of the whole
    # parts also has slivers on the faces that the fix did not change.
    ("The part with the ears", render("m-p", added(imp("stand-before"), cut("stand-ears", fl10)), cam_s, paint=None)),
], "Stand on its side: first layer and part")

# The pictures of the fix info windows in the app: before and after of one
# view for each fix. showcase-shots.sh makes them smaller and puts them in
# assets/fixes/.
app = os.path.join(out, "app")
os.makedirs(app, exist_ok=True)
W = os.path.join(work, "%s.png")
montage("teardrops", [("Before: round hole", W % "s-bh"), ("Teardrop holes", W % "s-th")],
        "Stand on its side: cable hole (section)", app)
montage("hole-size", [("Before", W % "h-mb"), ("Hole size +0.2 mm (blue: removed)", W % "h-ma")],
        "Base plate: magnet pocket (layer at z 1.5)", app)
montage("bridges", [("Before", W % "p-b"), ("Sacrificial layers", W % "p-s"), ("Stepped", W % "p-t")],
        "Base plate: screw counterbore, bed side down (section)", app)
montage("bed-chamfer", [("Before", W % "e-b"), ("Bed chamfer 0.4 mm (2 layers)", W % "e-a")],
        "Base plate: bed edge (section)", app)
montage("brim", [("Before: thin front foot", W % "b-b"), ("Brim on the small island", W % "b-a")],
        "Stand upright: first layer", app)
montage("mouse-ears", [("Before", W % "m-b"), ("Mouse ears (blue)", W % "m-a")],
        "Stand on its side: first layer", app)
