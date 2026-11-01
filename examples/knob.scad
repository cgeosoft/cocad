// Control knob with grip ribs, a D-shaft bore and a pointer.
// Tests rotate_extrude, small features and text.

/* [Knob] */
diameter = 30; // [10:1:80]
height = 16; // [6:1:40]
ribs = 18; // [0:1:60]
// Top edge rounding
chamfer = 2; // [0:0.5:5]

/* [Shaft] */
shaft = 6; // [3:0.5:10]
// Flat of the D shaft (0 = round)
flat = 1.5; // [0:0.1:3]
depth = 10; // [3:1:30]

/* [Marking] */
pointer = true;
label = "VOL"; // [8]

/* [Hidden] */
$fn = 96;

module body() {
    rotate_extrude() polygon([
        [0, 0], [diameter / 2, 0], [diameter / 2, height - chamfer],
        [diameter / 2 - chamfer, height], [0, height]
    ]);
    for (i = [0 : ribs - 1])
        rotate(i * 360 / max(ribs, 1))
            translate([diameter / 2, 0, 0]) cylinder(d = 1.6, h = height - chamfer, $fn = 12);
}

difference() {
    body();
    translate([0, 0, -0.01]) difference() {
        cylinder(d = shaft, h = depth);
        if (flat > 0) translate([shaft / 2 - flat, -shaft, 0]) cube([shaft, shaft * 2, depth]);
    }
    if (pointer) translate([diameter / 2 - chamfer - 4, -0.6, height - 0.6]) cube([4, 1.2, 1]);
    translate([0, -diameter / 6, height - 0.6])
        linear_extrude(1) text(label, size = diameter / 8, halign = "center", valign = "center");
}
