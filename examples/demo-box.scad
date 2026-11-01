// Demo model for SCAD Viewer. It uses every Customizer widget type.

/* [Size] */
// Outer width of the box
width = 60; // [20:150]
// Outer depth of the box
depth = 40; // [20:150]
// Outer height of the box
height = 25; // [5:100]
// Wall thickness
wall = 2; // [0.8:0.2:6]

/* [Style] */
// Corner style
corners = "round"; // [round, square]
// Number of compartments
compartments = 2; // [1:One, 2:Two, 3:Three, 4:Four]
// Add a lid lip
lip = true;
// Label on the front
label = "CGEO"; // [10]
// Label offset (x, z)
label_offset = [0, 0]; // [-20:20]

/* [Hidden] */
$fn = 48;
radius = corners == "round" ? 5 : 0.01;

module rounded_box(size, r) {
    hull() for (x = [r, size.x - r], y = [r, size.y - r])
        translate([x, y, 0]) cylinder(r = r, h = size.z);
}

difference() {
    rounded_box([width, depth, height], radius);
    translate([wall, wall, wall])
        rounded_box([width - 2 * wall, depth - 2 * wall, height], max(radius - wall, 0.01));
    if (lip)
        translate([wall / 2, wall / 2, height - 2])
            rounded_box([width - wall, depth - wall, 3], max(radius - wall / 2, 0.01));
    translate([width / 2 + label_offset[0], 0.6, height / 2 + label_offset[1]])
        rotate([90, 0, 0]) linear_extrude(1)
            text(label, size = min(height / 3, 8), halign = "center", valign = "center");
}

// Dividers
inner = width - 2 * wall;
for (i = [1 : compartments - 1])
    translate([wall + i * inner / compartments - wall / 4, wall, 0])
        cube([wall / 2, depth - 2 * wall, height - 3]);
