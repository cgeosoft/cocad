// A box and a screw that is not printed: a reference part.
// The % mark keeps the screw out of the export, as in OpenSCAD. CoCAD shows
// it in transparent gray in the Model tab, and the Parts card marks it
// "not printed". It is not in the export, the slice or the render.

module box() cube([20, 20, 10]);
module screw() translate([10, 10, 0]) cylinder(d = 3, h = 30, $fn = 16);

box();
%screw();
