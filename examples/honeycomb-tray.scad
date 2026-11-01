// Tray with a honeycomb floor. Tests many holes per layer and thin walls.
include <lib/hex.scad>

/* [Tray] */
length = 100; // [30:1:200]
width = 70; // [30:1:200]
height = 20; // [5:1:80]
wall = 2; // [0.8:0.2:5]
floor_thickness = 1.6; // [0.6:0.2:5]

/* [Honeycomb] */
cell = 10; // [4:0.5:30]
web = 1.6; // [0.8:0.2:5]

difference() {
    cube([length, width, height]);
    translate([wall, wall, floor_thickness]) cube([length - 2 * wall, width - 2 * wall, height]);
    // Honeycomb cut-outs, clipped to the floor inside the walls.
    intersection() {
        translate([wall * 2, wall * 2, -1]) cube([length - 4 * wall, width - 4 * wall, floor_thickness + 2]);
        translate([0, 0, -1]) hex_grid(length, width, cell, web, floor_thickness + 2);
    }
}
