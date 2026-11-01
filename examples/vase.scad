// Twisted vase. Tests linear_extrude with twist and scale: many sloped faces.

/* [Vase] */
sides = 7; // [3:1:16]
radius = 30; // [10:1:80]
height = 90; // [20:1:200]
twist = 120; // [0:5:360]
// Top radius as a fraction of the base
taper = 1.3; // [0.5:0.05:2]
wall = 1.6; // [0.8:0.1:5]
floor_thickness = 2; // [0.4:0.2:6]

/* [Hidden] */
$fn = 48;

module shape(r) {
    offset(r = 4) offset(delta = -4) circle(r = r, $fn = sides);
}

difference() {
    linear_extrude(height, twist = twist, scale = taper, slices = 60) shape(radius);
    translate([0, 0, floor_thickness])
        linear_extrude(height, twist = twist * (height - floor_thickness) / height,
                       scale = taper, slices = 60)
            shape(radius - wall);
}
