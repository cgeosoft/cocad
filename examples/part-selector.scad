// A box with a lid and four feet, with a part parameter.
// The part parameter works in plain OpenSCAD too: pick one part to see or
// export only that part. CoCAD builds each value on its own. The part names
// and colors are in part-selector.yml. The lid sits on the box, so in the
// "all" build they touch, but they stay separate parts. The four feet are one
// value: CoCAD names them "Foot 1" to "Foot 4". The coin has the % mark: it
// shows only with "all", and it is not printed.

/* [Parts] */
part = "all"; // [all, box, lid, feet]

/* [Size] */
width = 40; // [20:1:80]
depth = 30; // [20:1:80]
height = 20; // [10:1:60]
wall = 2; // [1.2:0.2:4]
lid_t = 3; // [2:0.5:6]
foot_d = 6; // [3:0.5:12]

$fn = 32;

module box()
    difference() {
        cube([width, depth, height]);
        translate([wall, wall, wall]) cube([width - 2 * wall, depth - 2 * wall, height]);
    }

// In its place in use: on top of the box, with a lip that goes into it.
module lid()
    translate([0, 0, height]) {
        cube([width, depth, lid_t]);
        translate([wall + 0.3, wall + 0.3, -2])
            cube([width - 2 * wall - 0.6, depth - 2 * wall - 0.6, 2]);
    }

// Below the box, one at each corner.
module feet()
    for (x = [foot_d, width - foot_d], y = [foot_d, depth - foot_d])
        translate([x, y, -3]) cylinder(d = foot_d, h = 3);

module coin() translate([width / 2, depth / 2, wall]) cylinder(d = 20, h = 2);

if (part == "all" || part == "box") box();
if (part == "all" || part == "lid") lid();
if (part == "all" || part == "feet") feet();
if (part == "all") %coin();
