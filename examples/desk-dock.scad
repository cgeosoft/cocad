// Desk dock: a phone stand on a base plate, with three cable clips.
// The model shows the dock as it stands on the desk. It has no print code:
// open the Optimize tab to prepare it for the printer.
//
// - The base plate is 260 mm long, so it does not fit most beds. The
//   Optimize tab proposes a split with a dovetail joint.
// - Two screws from below hold the stand. Under the plate: round head
//   counterbores with the screw holes above them, and magnet pockets. In
//   the stand: blind pilot holes that go up. Turn on Bridges.
// - The stand stands on a cable arch and has a cable hole across it. Turn
//   on Teardrop holes. Its front foot is 2.4 mm thin: turn on the Brim.
// - The three clips are the same part (x3). They sit tilted on the stand.
//   Turn on Custom orientation and use Auto to put them flat on the bed.
// - The phone, the magnets and the screws have the % mark: they show in
//   gray and are not printed.
// - Turn on the Bed chamfer for all parts.

/* [Plate] */
// Length of the base plate
plate_l = 260; // [180:5:320]
plate_w = 80; // [60:1:120]
plate_t = 6; // [4:0.2:10]
corner_r = 6; // [1:0.5:15]

/* [Stand] */
stand_w = 70; // [50:1:100]
// Lean of the back support from the vertical, in degrees
lean = 18; // [5:1:30]
// Diameter of the cable hole across the stand
cable_d = 8; // [5:0.5:12]

/* [Clips] */
clip_hole = 7; // [4:0.5:10]

/* [Hardware] */
magnet_d = 10; // [6:0.5:15]
magnet_h = 3; // [1:0.5:4]
screw_d = 4; // [3:0.5:6]
// Pilot hole in the stand for the screw
pilot_d = 3.4; // [2:0.1:5]
pilot_h = 10; // [5:0.5:15]
head_d = 8.6; // [6:0.2:12]
head_h = 2.6; // [1:0.2:4]
// Gap around the magnets and the screws
fit = 0.2; // [0:0.05:0.5]

/* [Hidden] */
$fn = 48;
eps = 0.01;
// Place of the stand on the plate (front left corner)
stand_at = [20, 8, 0];
// Stand profile in the y-z plane (y back, z up)
base_d = 60;   // depth of the stand base
base_h = 22;   // height of the stand base
arch_r = 12;   // radius of the cable arch under the base
foot = 2.4;    // width of the front foot
lip = [4, 10]; // front lip: depth and height above the base
sup_y = 16;    // front of the back support at the base
sup_t = 14;    // thickness of the back support
sup_h = 78;    // height of the back support above the base
rise = sup_h * tan(lean); // how far the support goes back

module rounded_rect(size, r) {
    translate([r, r]) offset(r = r) square([size[0] - 2 * r, size[1] - 2 * r]);
}

// ---------------------------------------------------------------- plate
magnets = [[16, 16], [plate_l - 16, 16], [16, plate_w - 16], [plate_l - 16, plate_w - 16]];
// The screws go up through the plate into the rear foot of the stand.
screws = [for (x = [0.2, 0.8]) [stand_at[0] + stand_w * x, stand_at[1] + 45]];

module plate() {
    difference() {
        linear_extrude(plate_t) rounded_rect([plate_l, plate_w], corner_r);
        // magnet pockets from below
        for (p = magnets) translate([p[0], p[1], -eps])
            cylinder(d = magnet_d + fit, h = magnet_h + eps);
        // screw holes with a head counterbore from below
        for (p = screws) translate([p[0], p[1], 0]) {
            translate([0, 0, -eps]) cylinder(d = head_d + fit, h = head_h + eps);
            translate([0, 0, -eps]) cylinder(d = screw_d + fit, h = plate_t + 2 * eps);
        }
    }
}

// ---------------------------------------------------------------- stand
module stand_profile() {
    difference() {
        union() {
            square([base_d, base_h]);
            translate([0, base_h]) square(lip);
            polygon([[sup_y, base_h], [sup_y + sup_t, base_h],
                     [sup_y + sup_t + rise, base_h + sup_h], [sup_y + rise, base_h + sup_h]]);
        }
        // cable arch: the front foot stays foot wide
        translate([foot + arch_r, 0]) circle(r = arch_r);
    }
}

module stand() {
    translate(stand_at + [0, 0, plate_t]) difference() {
        rotate([90, 0, 90]) linear_extrude(stand_w) stand_profile();
        // pilot holes for the screws from below
        for (p = screws) translate([p[0] - stand_at[0], p[1] - stand_at[1], -eps])
            cylinder(d = pilot_d, h = pilot_h + eps);
        // cable hole across the base, from the front to the back
        translate([stand_w / 2, -1, base_h / 2]) rotate([-90, 0, 0])
            cylinder(d = cable_d, h = base_d + 2);
        // slot from the cable hole up to the phone
        translate([stand_w / 2 - cable_d / 2, lip[0] + 2, base_h / 2])
            cube([cable_d, 5, base_h]);
    }
}

// ---------------------------------------------------------------- clips
// A clip in its own frame: a 14 x 14 x 3 base on z = 0, and a ring with
// a horizontal hole for a cable.
module clip() {
    difference() {
        union() {
            translate([-7, -7, 0]) cube([14, 14, 3]);
            translate([-7, 0, 10]) rotate([0, 90, 0]) cylinder(d = 14, h = 14);
            translate([-7, -7, 3]) cube([14, 14, 7]);
        }
        translate([-8, 0, 10]) rotate([0, 90, 0]) cylinder(d = clip_hole, h = 16);
    }
}

// The clips sit on the back face of the support, at t = 0..1 along it.
module clip_at(t) {
    y = sup_y + sup_t + rise * t;
    z = base_h + sup_h * t;
    translate(stand_at + [stand_w / 2, y, plate_t + z]) rotate([-(90 + lean), 0, 0]) clip();
}

// ---------------------------------------------------------------- hardware
module phone() {
    translate(stand_at + [(stand_w - 72) / 2, lip[0] + 0.5, plate_t + base_h])
        rotate([-lean, 0, 0]) color("#26272a")
            translate([0, 0, 0]) cube([72, 8.5, 150]);
}

module magnets() {
    for (p = magnets) translate([p[0], p[1], 0]) color("silver") cylinder(d = magnet_d, h = magnet_h);
}

module screws() {
    for (p = screws) translate([p[0], p[1], 0]) color("silver") {
        cylinder(d = head_d, h = head_h);
        cylinder(d = screw_d, h = plate_t + pilot_h - 1);
    }
}

// ---------------------------------------------------------------- model
module assembly() {
    plate();
    stand();
    clip_at(0.3);
    clip_at(0.55);
    clip_at(0.8);
    %phone();
    %magnets();
    %screws();
}

assembly();
