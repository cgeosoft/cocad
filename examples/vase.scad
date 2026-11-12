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

// Edge length of the outline. Short edges keep a twisted wall smooth.
step = 2.5;
corner = 4;

// Regular polygon with round corners. The straight sides have points each
// `step`, because linear_extrude twists only the points of the outline.
module shape(r) {
    half = 180 / sides;
    inner = r - corner / cos(half);
    side = 2 * inner * sin(half);
    arc_n = max(2, ceil(2 * half * PI / 180 * corner / step));
    side_n = max(1, ceil(side / step));
    function centre(i) = inner * [cos(i * 2 * half), sin(i * 2 * half)];
    function arc(i, k) = let(a = (2 * i - 1) * half + 2 * half * k / arc_n)
        centre(i) + corner * [cos(a), sin(a)];
    polygon([for (i = [0 : sides - 1]) each concat(
        [for (k = [0 : arc_n]) arc(i, k)],
        [for (k = [1 : 1 : side_n - 1])
            arc(i, arc_n) + (arc(i + 1, 0) - arc(i, arc_n)) * k / side_n])]);
}

// The cavity starts at the floor with the turn and the scale that the outer
// wall has there. Thus the two walls stay parallel at each twist value.
floor_scale = 1 + (taper - 1) * floor_thickness / height;

difference() {
    linear_extrude(height, twist = twist, scale = taper, slices = 90) shape(radius);
    translate([0, 0, floor_thickness]) rotate([0, 0, -twist * floor_thickness / height])
        linear_extrude(height, twist = twist,
                       scale = (floor_scale + taper - 1) / floor_scale, slices = 90)
            scale(floor_scale) shape(radius - wall);
}
