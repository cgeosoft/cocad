// Shared helper for honeycomb-tray.scad. Tests include<> with a relative path.

module hex_grid(length, width, cell, web, h) {
    dx = cell + web;
    dy = dx * sqrt(3) / 2;
    for (row = [0 : ceil(width / dy)], col = [0 : ceil(length / dx)])
        translate([col * dx + (row % 2) * dx / 2, row * dy, 0])
            cylinder(d = cell * 2 / sqrt(3), h = h, $fn = 6);
}
