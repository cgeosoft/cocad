// L bracket with slots, holes and a gusset. Tests overhangs, holes and bridges.

/* [Size] */
width = 30; // [10:1:100]
leg_a = 40; // [10:1:120]
leg_b = 30; // [10:1:120]
thickness = 4; // [1.5:0.5:10]

/* [Holes] */
hole = 4.5; // [2:0.1:10]
holes_per_leg = 2; // [1:1:4]
slots = true;
slot_length = 8; // [2:1:30]

/* [Gusset] */
gusset = true;
gusset_size = 15; // [4:1:60]

/* [Hidden] */
$fn = 40;

module slot(len) {
    hull() for (x = [0, slots ? len : 0]) translate([x, 0, 0]) cylinder(d = hole, h = thickness * 3, center = true);
}

difference() {
    union() {
        cube([leg_a, width, thickness]);
        cube([thickness, width, leg_b]);
        if (gusset)
            translate([thickness, width / 2 + thickness / 2, thickness]) rotate([90, 0, 0])
                linear_extrude(thickness) polygon([[0, 0], [gusset_size, 0], [0, gusset_size]]);
    }
    for (i = [1 : holes_per_leg]) {
        translate([thickness + (leg_a - thickness) * i / (holes_per_leg + 1) - slot_length / 2, width / 2 + (gusset ? width / 4 : 0), 0])
            slot(slot_length);
        translate([0, width / 2 + (gusset ? width / 4 : 0), thickness + (leg_b - thickness) * i / (holes_per_leg + 1)])
            rotate([0, 90, 0]) cylinder(d = hole, h = thickness * 3, center = true);
    }
}
