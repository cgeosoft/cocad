// Small box with a lid and a knob: an assembly of parts that touch.
// OpenSCAD joins the parts into one solid. The part parameter selects one
// part, also in plain OpenSCAD. CoCAD builds each part on its own and takes
// the part names and colors from lid-box.yml.
// Try Part colors, Explode and Section in the Model tab.

/* [Parts] */
part = "all"; // [all, box, lid, knob]

/* [Box] */
width = 60; // [30:1:120]
depth = 40; // [20:1:120]
height = 30; // [10:1:80]
wall = 2; // [1:0.2:5]
corner = 5; // [0:0.5:12]

/* [Lid] */
lid_height = 4; // [2:0.5:12]
// Depth of the lip that holds the lid in the box
lip = 4; // [0:0.5:10]
// Gap between the lip and the box wall
clearance = 0.3; // [0:0.05:1]
knob_d = 14; // [6:1:30]
knob_h = 8; // [3:1:20]

$fn = 64;

// A rounded rectangle, centered on the origin, from z = 0 to z = h.
module rounded(w, d, h, r) {
    r = max(0.01, min(r, min(w, d) / 2 - 0.01));
    linear_extrude(h) offset(r = r) square([w - 2 * r, d - 2 * r], center = true);
}

module box() {
    difference() {
        rounded(width, depth, height, corner);
        translate([0, 0, wall])
            rounded(width - 2 * wall, depth - 2 * wall, height, corner - wall);
    }
}

// union() keeps the plate and the lip together as one part in CoCAD.
module lid() {
    inner = 2 * (wall + clearance);
    union() {
        translate([0, 0, height]) rounded(width, depth, lid_height, corner);
        if (lip > 0)
            translate([0, 0, height - lip])
                difference() {
                    rounded(width - inner, depth - inner, lip, corner - wall);
                    translate([0, 0, -1])
                        rounded(width - inner - 2 * wall, depth - inner - 2 * wall, lip + 2,
                                corner - 2 * wall);
                }
    }
}

module knob() {
    translate([0, 0, height + lid_height])
        cylinder(d1 = knob_d, d2 = knob_d * 0.8, h = knob_h);
}

// Shows the children when this part is selected.
module show(name) if (part == "all" || part == name) children();

show("box") box();
show("lid") lid();
show("knob") knob();
