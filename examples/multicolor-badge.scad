// Badge with two SVG inlays, one part for each color.
// Tests SVG import and parts that touch. Each part is a separate solid that
// fills a pocket in the plate.
// CoCAD reads the part names from cocad_part and renders each one. The list
// order sets the filament number. The echo sets the filament color.

/* [Plate] */
size = 60; // [46:1:100]
thickness = 3; // [1:0.2:8]

/* [Inlay] */
// Two or three print layers are enough
depth = 0.4; // [0.2:0.2:1.2]

/* [Parts] */
cocad_part = "all"; // [all, plate, star, ring]

// Shows the children when this part is selected.
module part(name, col) {
    if (cocad_part == "all" || cocad_part == name) {
        echo(cocad_color = col);
        color(col) children();
    }
}

// One SVG file for each color, with its top at the top of the plate.
module art(file) {
    translate([size / 2, size / 2, thickness - depth])
        linear_extrude(depth) import(file, center = true);
}

part("plate", "white") difference() {
    cube([size, size, thickness]);
    art("lib/badge-star.svg");
    art("lib/badge-ring.svg");
}
part("star", "red") art("lib/badge-star.svg");
part("ring", "black") art("lib/badge-ring.svg");
