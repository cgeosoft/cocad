// Spur gear with a hub and a bore. Tests polygon math and many facets.

/* [Gear] */
// Number of teeth
teeth = 24; // [8:1:80]
// Module (tooth size)
gear_module = 2; // [0.5:0.1:5]
// Face width
thickness = 6; // [2:0.5:30]
// Pressure angle in degrees
pressure_angle = 20; // [14.5, 20, 25]

/* [Hub] */
bore = 5; // [0:0.5:30]
hub = true;
hub_height = 6; // [0:0.5:30]

/* [Hidden] */
$fn = 64;

function involute(r, t) = r * [cos(t) + t * PI / 180 * sin(t), sin(t) - t * PI / 180 * cos(t)];

module gear2d() {
    pitch_r = teeth * gear_module / 2;
    base_r = pitch_r * cos(pressure_angle);
    outer_r = pitch_r + gear_module;
    root_r = pitch_r - 1.25 * gear_module;
    max_t = sqrt(pow(outer_r / base_r, 2) - 1) * 180 / PI;
    // Half tooth angle at the pitch circle, plus the involute offset there.
    t_pitch = sqrt(pow(pitch_r / base_r, 2) - 1) * 180 / PI;
    inv_pitch = atan2(involute(base_r, t_pitch)[1], involute(base_r, t_pitch)[0]);
    half = 90 / teeth + inv_pitch;
    flank = [for (t = [0 : max_t / 8 : max_t]) involute(base_r, t)];
    union() {
        circle(r = root_r);
        for (i = [0 : teeth - 1])
            rotate(i * 360 / teeth)
                polygon(concat(
                    [[0, 0]],
                    [for (p = flank) [p[0] * cos(-half) - p[1] * sin(-half), p[0] * sin(-half) + p[1] * cos(-half)]],
                    [for (j = [len(flank) - 1 : -1 : 0]) let(p = [flank[j][0], -flank[j][1]])
                        [p[0] * cos(half) - p[1] * sin(half), p[0] * sin(half) + p[1] * cos(half)]]
                ));
    }
}

difference() {
    union() {
        linear_extrude(thickness) gear2d();
        if (hub) cylinder(d = max(bore * 2.2, bore + 8), h = thickness + hub_height);
    }
    if (bore > 0) translate([0, 0, -1]) cylinder(d = bore, h = thickness + hub_height + 2);
}
