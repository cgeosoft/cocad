//! Auto split (feature 9 of `docs/optimize-tab.md`): cut a part that does not
//! fit the print area into pieces, with joints.
//!
//! Version 1 cuts with parallel planes across one axis of the model frame.
//! `propose` finds the axis, the number of pieces and the cut positions.
//! `apply` makes the pieces with OpenSCAD, one run for each piece.
//!
//! The joint frame of a cut ("local" below) has its origin on the cut plane:
//!
//! - x is along the cut line: the larger of the two other sides of the
//!   bounding box,
//! - y is across the cut: the cut axis,
//! - z goes through the thinnest side.
//!
//! A dovetail profile is in the local x-y plane and goes through z, as in
//! `book-cover`. At each cut, the lower piece (local y < 0) gets the tabs and
//! the upper piece gets the sockets.

use super::{Area, bounds_or_zero, loop_area, scad, section, transformed};
use crate::mesh::{Bounds, Mesh};
use crate::openscad::Config;
use anyhow::{Result, bail};
use glam::{Mat4, Vec2, Vec3, Vec4};
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Grid step of the cut positions and of the section samples, in mm.
const STEP: f32 = 0.5;
/// `propose` looks for each cut within this part of the piece length from
/// the position that gives equal lengths.
const WINDOW: f32 = 0.2;
/// The "near a change" cost looks this far (mm) on each side of a cut, or
/// as far as the joint goes into a piece.
const NEAR: f32 = 2.0;
/// A section region with less area (mm²) is a small feature.
const SMALL_AREA: f32 = 25.0;
/// A section region with a mean width (2 x area / perimeter) less than this
/// (mm) is a thin wall.
const THIN: f32 = 2.0;
/// Material that a joint keeps around a hole or a socket, in mm.
const WALL: f32 = 1.0;
/// Radius of the wide corners of a dovetail tab (book-cover: 0.5 mm).
const TAB_ROUND: f32 = 0.5;
/// A tab goes this far into its own piece, so that it joins the piece
/// (book-cover: 1 mm).
const TAB_ROOT: f32 = 1.0;
/// Chamfer at the two ends of a printed pin, in mm.
const PIN_CHAMFER: f32 = 0.3;
/// Tolerance of the size tests, in mm.
const FIT_TOL: f32 = 1e-3;
/// `propose` tries this many piece counts past the smallest count.
const MORE_PIECES: usize = 4;
/// The OpenSCAD resolution for the joint shapes.
const HEADER: &str = "$fa = 6;\n$fs = 0.25;\n";

/// The joint between two pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Joint {
    /// Two flat faces. Glue them.
    Flat,
    /// Holes in the two faces and printed pins.
    Pins,
    /// Dovetail tabs across the cut, in the plane of the part, as in
    /// `book-cover`.
    Dovetail,
}

impl Joint {
    pub const ALL: [Joint; 3] = [Joint::Flat, Joint::Pins, Joint::Dovetail];

    pub fn name(self) -> &'static str {
        match self {
            Joint::Flat => "flat",
            Joint::Pins => "pins",
            Joint::Dovetail => "dovetail",
        }
    }

    pub fn from_name(s: &str) -> Option<Joint> {
        Joint::ALL.into_iter().find(|j| j.name() == s)
    }

    /// The length that the joint adds to the lower piece of a cut, past the
    /// cut: the dovetail tabs.
    fn room(self, v: &JointValues) -> f32 {
        match self {
            Joint::Dovetail => v.tab_depth,
            Joint::Flat | Joint::Pins => 0.0,
        }
    }

    /// How far the joint goes into a piece from the cut.
    fn reach(self, v: &JointValues) -> f32 {
        match self {
            Joint::Flat => 0.0,
            Joint::Pins => v.hole_depth() + WALL,
            Joint::Dovetail => v.tab_depth + v.clearance + WALL,
        }
    }
}

/// The sizes of the joints, in mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointValues {
    /// The total gap between the two sides of a joint. Each side gets one
    /// half. Pins: the holes are larger by this value, the pins are not
    /// smaller.
    pub clearance: f32,
    /// Diameter of a pin.
    pub pin_d: f32,
    /// Depth of a pin in each piece.
    pub pin_depth: f32,
    /// Section area for each pin, in mm². A region gets at least 2 pins
    /// when it has room for them, else 1.
    pub pin_area: f32,
    /// The maximum number of pins in one region of a section.
    pub pin_max: usize,
    /// A printed pin is this much shorter than 2 x `pin_depth`.
    pub pin_short: f32,
    /// Width of a dovetail tab at the cut line (book-cover: 8).
    pub tab_root: f32,
    /// Width at the end of a tab (book-cover: 12).
    pub tab_tip: f32,
    /// Length of a tab past the cut (book-cover: 5).
    pub tab_depth: f32,
    /// Distance between the tabs (book-cover: 30).
    pub tab_pitch: f32,
}

impl Default for JointValues {
    fn default() -> Self {
        JointValues {
            clearance: 0.2,
            pin_d: 4.0,
            pin_depth: 5.0,
            pin_area: 400.0,
            pin_max: 8,
            pin_short: 0.4,
            tab_root: 8.0,
            tab_tip: 12.0,
            tab_depth: 5.0,
            tab_pitch: 30.0,
        }
    }
}

impl JointValues {
    /// Diameter of a pin hole.
    pub fn hole_d(&self) -> f32 {
        self.pin_d + self.clearance
    }

    /// Depth of a pin hole in each piece.
    fn hole_depth(&self) -> f32 {
        self.pin_depth + self.clearance
    }

    /// Length of a printed pin.
    fn pin_len(&self) -> f32 {
        2.0 * self.pin_depth - self.pin_short
    }
}

/// A cut plane across axis `axis` (0, 1, 2 for x, y, z) at the coordinate
/// `at`, in the model frame of the part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cut {
    pub axis: usize,
    pub at: f32,
}

/// How to cut one part. Version 1: all cuts go across the same axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    pub cuts: Vec<Cut>,
    pub joint: Joint,
    pub values: JointValues,
}

impl Split {
    /// The number of pieces, without the pins.
    pub fn pieces(&self) -> usize {
        self.cuts.len() + 1
    }
}

/// One result of `apply`.
#[derive(Clone, Debug)]
pub struct Piece {
    /// "1/3", "2/3", "3/3", or "pin".
    pub label: String,
    /// A piece of the part is in the model frame. A pin stands on z = 0 at
    /// the origin.
    pub mesh: Mesh,
    /// True for a printed pin.
    pub pin: bool,
}

/// The joint shapes of one cut, in the joint frame of the cut.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Places {
    /// Dovetail: the centers of the tabs along the cut line (local x).
    pub tabs: Vec<f32>,
    /// Pins: the centers of the pins in the cut face (local x and z).
    pub pins: Vec<Vec2>,
}

/// True when the bounding box of `mesh` fits the print area. Any side of
/// the box can be the height.
pub fn fits(mesh: &Mesh, area: &Area) -> bool {
    mesh.bounds().is_none_or(|b| fits_size(b.size(), area))
}

/// True when a box of `size` fits `area` in one of the axis permutations.
/// When the sorted sides fit the sorted area sides, a permutation fits.
fn fits_size(size: Vec3, area: &Area) -> bool {
    let mut s = size.to_array();
    let mut a = [area.w, area.d, area.h];
    s.sort_by(f32::total_cmp);
    a.sort_by(f32::total_cmp);
    s.iter().zip(&a).all(|(s, a)| *s <= *a + FIT_TOL)
}

/// The proposal for a part that does not fit `area`, with the default joint
/// values. See `propose_with`.
pub fn propose(mesh: &Mesh, area: &Area, joint: Joint) -> Option<Split> {
    propose_with(mesh, area, joint, JointValues::default())
}

/// The proposal for a part that does not fit `area`: parallel cuts across
/// one axis. Returns `None` when the part fits, and when no set of parallel
/// cuts makes pieces that fit (for example a plate that is too large in two
/// directions).
///
/// The axis is the long side of the bounding box (then the other sides).
/// The number of pieces is the smallest number for which each piece, with
/// the room for the joint, fits. Each cut is within 20 percent of the piece
/// length from equal lengths, on a 0.5 mm grid. The cost prefers a section
/// with few regions and a large area, with no small regions, holes or thin
/// walls, away from a change of the section, near equal lengths.
pub fn propose_with(mesh: &Mesh, area: &Area, joint: Joint, values: JointValues) -> Option<Split> {
    let b = mesh.bounds()?;
    let size = b.size();
    if fits_size(size, area) {
        return None;
    }
    let mut axes = [0, 1, 2];
    axes.sort_by(|&i, &j| size[j].total_cmp(&size[i]));
    let room = joint.room(&values);
    let min_len = 2.0 * joint.reach(&values) + 2.0;
    for axis in axes {
        let mut s = Search::new(mesh, axis);
        s.near = ((NEAR.max(joint.reach(&values))) / STEP).ceil() as i64;
        let Some(first) = s.min_pieces(area, room) else {
            continue;
        };
        let first = first.max(2);
        for n in first..=first + MORE_PIECES {
            if let Some(ats) = s.best(n, area, room, min_len) {
                return Some(Split {
                    cuts: ats.into_iter().map(|at| Cut { axis, at }).collect(),
                    joint,
                    values,
                });
            }
        }
    }
    None
}

/// The bounding box sizes of the pieces of `split`, in model axes, with the
/// dovetail tabs. Without the clearance, so a little too large.
pub fn piece_sizes(mesh: &Mesh, split: &Split) -> Vec<Vec3> {
    let Some(first) = split.cuts.first() else {
        return mesh.bounds().map(|b| b.size()).into_iter().collect();
    };
    let s = Search::new(mesh, first.axis);
    let (lo, hi) = (s.fr.b.min.z, s.fr.b.max.z);
    let room = split.joint.room(&split.values);
    let mut ats: Vec<f32> = split.cuts.iter().map(|c| c.at).collect();
    ats.sort_by(f32::total_cmp);
    let mut ends = vec![lo];
    ends.extend(ats);
    ends.push(hi);
    let along = s.fr.along;
    let thin = 3 - first.axis - along;
    ends.windows(2)
        .enumerate()
        .map(|(i, w)| {
            let (a, b) = (w[0].max(lo), w[1].min(hi));
            let extra = if i + 2 < ends.len() { room } else { 0.0 };
            let c = s.prof.cross(a, b).unwrap_or(Vec2::ZERO);
            let mut size = Vec3::ZERO;
            size[first.axis] = (b - a).max(0.0) + extra;
            size[along] = c.x;
            size[thin] = c.y;
            size
        })
        .collect()
}

/// The joint frame of `cut` on `mesh`: the matrix from local coordinates to
/// the model frame.
pub fn cut_frame(mesh: &Mesh, cut: &Cut) -> Mat4 {
    let (m, _) = rotation(cut.axis, bounds_or_zero(mesh).size());
    m * Mat4::from_translation(Vec3::new(0.0, cut.at, 0.0))
}

/// The places of the joint shapes for each cut of `split`, in the same
/// order as `split.cuts`. A cut with no room for a pin or a tab gets none:
/// it is a flat joint.
pub fn places(mesh: &Mesh, split: &Split) -> Vec<Places> {
    let mut frames: [Option<Frame>; 3] = [None, None, None];
    split
        .cuts
        .iter()
        .map(|c| {
            if split.joint == Joint::Flat || c.axis > 2 {
                return Places::default();
            }
            let fr = frames[c.axis].get_or_insert_with(|| Frame::new(mesh, c.axis));
            match split.joint {
                Joint::Flat => Places::default(),
                Joint::Pins => Places {
                    pins: pin_places(fr, c.at, &split.values),
                    tabs: Vec::new(),
                },
                Joint::Dovetail => Places {
                    tabs: tab_places(fr, c.at, &split.values),
                    pins: Vec::new(),
                },
            }
        })
        .collect()
}

/// Cuts `mesh` (one closed part in the model frame) as `split` tells. Runs
/// OpenSCAD once for each piece and once for the pin shape. Returns the
/// pieces in the order of the cut axis, then one `Piece` for each pin.
pub fn apply(
    cfg: &Config,
    mesh: &Mesh,
    split: &Split,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<Piece>> {
    let split = checked(split)?;
    let places = places(mesh, &split);
    let frames: Vec<Mat4> = split.cuts.iter().map(|c| cut_frame(mesh, c)).collect();
    let b = bounds_or_zero(mesh);
    let big =
        (2.0 * b.min.abs().max(b.max.abs()).max_element() + split.values.tab_depth + 20.0).ceil();
    let n = split.pieces();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        let m = scad::run(
            cfg,
            &[mesh],
            |imports| piece_scad(&imports[0], i, &split, &frames, &places, big),
            cancel,
        )?;
        out.push(Piece {
            label: format!("{}/{n}", i + 1),
            mesh: m,
            pin: false,
        });
    }
    let pins: usize = places.iter().map(|p| p.pins.len()).sum();
    if pins > 0 {
        let m = scad::run(cfg, &[], |_| pin_scad(&split.values), cancel)?;
        out.extend((0..pins).map(|_| Piece {
            label: "pin".into(),
            mesh: m.clone(),
            pin: true,
        }));
    }
    Ok(out)
}

/// `split` with the cuts in order. Fails when the cuts are not parallel.
fn checked(split: &Split) -> Result<Split> {
    let Some(first) = split.cuts.first() else {
        bail!("the split has no cuts");
    };
    if first.axis > 2 || split.cuts.iter().any(|c| c.axis != first.axis) {
        bail!("all cuts of a split must go across the same axis");
    }
    if split.cuts.iter().any(|c| !c.at.is_finite()) {
        bail!("a cut position is not a number");
    }
    let mut out = split.clone();
    out.cuts.sort_by(|a, b| a.at.total_cmp(&b.at));
    Ok(out)
}

// --- Frames and sections ---------------------------------------------------

/// The rotation from the joint frame of cuts across `axis` to the model
/// frame, for a part with bounding box size `size`. Also returns the model
/// axis of local x. The determinant is 1.
fn rotation(axis: usize, size: Vec3) -> (Mat4, usize) {
    let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
    let along = if size[b] > size[a] { b } else { a };
    let x = Vec3::AXES[along];
    let y = Vec3::AXES[axis];
    let m = Mat4::from_cols(
        x.extend(0.0),
        y.extend(0.0),
        x.cross(y).extend(0.0),
        Vec4::W,
    );
    (m, along)
}

/// From local (x, y, z) to section coordinates (x, z, y): the cut axis
/// becomes z, so that `section()` cuts across it.
const SWAP_YZ: Mat4 = Mat4::from_cols(Vec4::X, Vec4::Z, Vec4::Y, Vec4::W);

/// The part in section coordinates for cuts across one axis. A section at
/// `at` has its points in local x and local z.
struct Frame {
    /// The model axis of local x.
    along: usize,
    sec: Mesh,
    b: Bounds,
}

impl Frame {
    fn new(mesh: &Mesh, axis: usize) -> Frame {
        let (m, along) = rotation(axis, bounds_or_zero(mesh).size());
        // The rotation has only 0, 1 and -1, so the transpose is the exact
        // inverse.
        let sec = transformed(mesh, SWAP_YZ * m.transpose());
        let b = bounds_or_zero(&sec);
        Frame { along, sec, b }
    }

    fn section(&self, at: f32) -> Vec<Vec<Vec2>> {
        section(&self.sec, at)
    }
}

/// One connected region of a section: the outer loop first, then its holes.
struct Region {
    loops: Vec<Vec<Vec2>>,
    area: f32,
    perimeter: f32,
    lo: Vec2,
    hi: Vec2,
}

fn perimeter(p: &[Vec2]) -> f32 {
    (0..p.len())
        .map(|i| p[i].distance(p[(i + 1) % p.len()]))
        .sum()
}

/// Sorts the section loops into regions. A hole goes to the smallest outer
/// loop around it.
fn regions(loops: &[Vec<Vec2>]) -> Vec<Region> {
    let mut regs: Vec<Region> = loops
        .iter()
        .filter(|l| loop_area(l) > 0.0)
        .map(|l| {
            let lo = l.iter().fold(Vec2::INFINITY, |a, p| a.min(*p));
            let hi = l.iter().fold(Vec2::NEG_INFINITY, |a, p| a.max(*p));
            Region {
                loops: vec![l.clone()],
                area: loop_area(l),
                perimeter: perimeter(l),
                lo,
                hi,
            }
        })
        .collect();
    for h in loops.iter().filter(|l| loop_area(l) < 0.0) {
        let owner = regs
            .iter_mut()
            .filter(|r| inside(h[0], &r.loops[..1]))
            .min_by(|a, b| a.area.total_cmp(&b.area));
        if let Some(r) = owner {
            r.area += loop_area(h);
            r.perimeter += perimeter(h);
            r.loops.push(h.clone());
        }
    }
    regs
}

/// Even-odd test: true when `p` is inside the loops.
fn inside(p: Vec2, loops: &[Vec<Vec2>]) -> bool {
    let mut c = false;
    for lp in loops {
        for i in 0..lp.len() {
            let (a, b) = (lp[i], lp[(i + 1) % lp.len()]);
            if (a.y > p.y) != (b.y > p.y) {
                let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
                if p.x < x {
                    c = !c;
                }
            }
        }
    }
    c
}

/// The distance from `p` to the nearest edge of the loops.
fn dist(p: Vec2, loops: &[Vec<Vec2>]) -> f32 {
    let mut d = f32::INFINITY;
    for lp in loops {
        for i in 0..lp.len() {
            let (a, b) = (lp[i], lp[(i + 1) % lp.len()]);
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
            d = d.min(p.distance(a + ab * t));
        }
    }
    d
}

/// The values of a section that the cut cost uses.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Summary {
    area: f32,
    regions: usize,
    holes: usize,
    /// Regions with a small area.
    small: usize,
    /// Regions that are thin walls.
    thin: usize,
}

fn summarize(loops: &[Vec<Vec2>]) -> Summary {
    let regs = regions(loops);
    let total: f32 = regs.iter().map(|r| r.area).sum();
    let mut s = Summary {
        area: total,
        regions: regs.len(),
        holes: regs.iter().map(|r| r.loops.len() - 1).sum(),
        ..Summary::default()
    };
    for r in &regs {
        if r.area < SMALL_AREA.max(0.02 * total) {
            s.small += 1;
        } else if 2.0 * r.area / r.perimeter.max(1e-6) < THIN {
            s.thin += 1;
        }
    }
    s
}

// --- Cut search ------------------------------------------------------------

/// Bounds of the part in slabs of `STEP` along the cut axis, in a sparse
/// table, for fast size tests of the pieces. An entry is
/// [min x, min y, -max x, -max y] of the section coordinates, so that two
/// entries merge with `min`.
struct Profile {
    /// Position of grid line 0: a multiple of `STEP`.
    z0: f32,
    /// Level k has the merge of 2^k bins from each bin.
    table: Vec<Vec<[f32; 4]>>,
}

fn merge(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

/// The part of `poly` where `f` is not negative.
fn clip_half(poly: &[Vec3], f: impl Fn(Vec3) -> f32) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (u, w) = (poly[i], poly[(i + 1) % poly.len()]);
        let (fu, fw) = (f(u), f(w));
        if fu >= 0.0 {
            out.push(u);
        }
        if (fu >= 0.0) != (fw >= 0.0) {
            out.push(u + (w - u) * (fu / (fu - fw)));
        }
    }
    out
}

impl Profile {
    fn new(fr: &Frame) -> Profile {
        let z0 = (fr.b.min.z / STEP).floor() * STEP;
        let n = (((fr.b.max.z - z0) / STEP).ceil() as usize).max(1);
        let mut bins = vec![[f32::INFINITY; 4]; n];
        let bin = |z: f32| (((z - z0) / STEP).floor().max(0.0) as usize).min(n - 1);
        let add = |e: &mut [f32; 4], p: Vec3| {
            *e = merge(*e, [p.x, p.y, -p.x, -p.y]);
        };
        for i in 0..fr.sec.triangles.len() {
            let t = fr.sec.tri(i);
            let lo = t[0].z.min(t[1].z).min(t[2].z);
            let hi = t[0].z.max(t[1].z).max(t[2].z);
            let (i0, i1) = (bin(lo), bin(hi));
            if i0 == i1 {
                t.iter().for_each(|p| add(&mut bins[i0], *p));
                continue;
            }
            for (k, e) in bins.iter_mut().enumerate().take(i1 + 1).skip(i0) {
                let a = z0 + k as f32 * STEP;
                let b = a + STEP;
                let p = clip_half(&t, |v| v.z - a);
                let p = clip_half(&p, |v| b - v.z);
                p.iter().for_each(|p| add(e, *p));
            }
        }
        let mut table = vec![bins];
        let mut w = 1;
        while 2 * w <= n {
            let prev = table.last().unwrap();
            let next = (0..=n - 2 * w)
                .map(|i| merge(prev[i], prev[i + w]))
                .collect();
            table.push(next);
            w *= 2;
        }
        Profile { z0, table }
    }

    /// The size in section x and y of the part between `lo` and `hi` on the
    /// cut axis, or `None` when there is no material.
    fn cross(&self, lo: f32, hi: f32) -> Option<Vec2> {
        let n = self.table[0].len();
        let l = (((lo - self.z0) / STEP + 1e-3).floor().max(0.0) as usize).min(n - 1);
        let r = (((hi - self.z0) / STEP - 1e-3).ceil().max(0.0) as usize).clamp(l + 1, n);
        let k = (r - l).ilog2() as usize;
        let e = merge(self.table[k][l], self.table[k][r - (1 << k)]);
        (e[0] <= -e[2]).then(|| Vec2::new(-e[2] - e[0], -e[3] - e[1]))
    }
}

/// The cut search along one axis.
struct Search {
    fr: Frame,
    prof: Profile,
    /// Section summaries by grid index.
    cache: HashMap<i64, Summary>,
    /// Grid steps on each side that the "near a change" cost looks at.
    near: i64,
}

impl Search {
    fn new(mesh: &Mesh, axis: usize) -> Search {
        let fr = Frame::new(mesh, axis);
        let prof = Profile::new(&fr);
        Search {
            fr,
            prof,
            cache: HashMap::new(),
            near: (NEAR / STEP).ceil() as i64,
        }
    }

    fn pos(&self, g: i64) -> f32 {
        self.prof.z0 + g as f32 * STEP
    }

    /// The first grid index after `z`.
    fn next_grid(&self, z: f32) -> i64 {
        ((z - self.prof.z0) / STEP + 1e-3).floor() as i64 + 1
    }

    fn summary(&mut self, g: i64) -> Summary {
        if let Some(s) = self.cache.get(&g) {
            return *s;
        }
        let s = summarize(&self.fr.section(self.pos(g)));
        self.cache.insert(g, s);
        s
    }

    /// True when the piece between `lo` and `hi`, longer by `room`, fits.
    fn piece_fits(&self, lo: f32, hi: f32, room: f32, area: &Area) -> bool {
        let (lo, hi) = (lo.max(self.fr.b.min.z), hi.min(self.fr.b.max.z));
        match self.prof.cross(lo, hi) {
            Some(c) => fits_size(Vec3::new(hi - lo + room, c.x, c.y), area),
            None => true,
        }
    }

    /// The smallest number of pieces when the cuts can go anywhere on the
    /// grid. Each piece takes the longest length that fits. `None` when a
    /// thin slab of the part does not fit.
    fn min_pieces(&self, area: &Area, room: f32) -> Option<usize> {
        let (lo, hi) = (self.fr.b.min.z, self.fr.b.max.z);
        let mut start = lo;
        for n in 1.. {
            if self.piece_fits(start, hi, 0.0, area) {
                return Some(n);
            }
            let mut end = None;
            let mut g = self.next_grid(start);
            while self.pos(g) < hi && self.piece_fits(start, self.pos(g), room, area) {
                end = Some(self.pos(g));
                g += 1;
            }
            start = end?;
        }
        None
    }

    /// The cost of a cut at grid index `g`. `amax` is the largest section
    /// area near the cut.
    fn cost(&mut self, g: i64, amax: f32) -> f32 {
        let s = self.summary(g);
        let mut c = 1.0 - s.area / amax;
        c += 0.5 * s.regions.saturating_sub(1) as f32;
        c += 0.2 * s.holes as f32;
        c += 0.5 * (s.small + s.thin) as f32;
        let mut near = 0.0f32;
        for d in 1..=self.near {
            for h in [g - d, g + d] {
                let t = self.summary(h);
                let mut x = (t.area - s.area).abs() / amax;
                if t.regions != s.regions || t.holes != s.holes {
                    x += 0.25;
                }
                near = near.max(x);
            }
        }
        c + 0.5 * near.min(1.0)
    }

    /// The best cut positions for `n` pieces, or `None` when no positions in
    /// the windows make pieces that fit.
    fn best(&mut self, n: usize, area: &Area, room: f32, min_len: f32) -> Option<Vec<f32>> {
        let (lo, hi) = (self.fr.b.min.z, self.fr.b.max.z);
        let plen = (hi - lo) / n as f32;
        let win = (WINDOW * plen).max(STEP);
        // For each cut: (grid index, cost) of each candidate.
        let mut layers: Vec<Vec<(i64, f32)>> = Vec::with_capacity(n - 1);
        for j in 1..n {
            let ideal = lo + plen * j as f32;
            let g0 = ((ideal - win - self.prof.z0) / STEP).ceil() as i64;
            let g1 = ((ideal + win - self.prof.z0) / STEP).floor() as i64;
            let gs: Vec<i64> = (g0..=g1)
                .filter(|&g| {
                    let p = self.pos(g);
                    p > lo + min_len && p < hi - min_len
                })
                .collect();
            let (&first, &last) = (gs.first()?, gs.last()?);
            let mut amax = 0.0f32;
            for g in first - self.near..=last + self.near {
                amax = amax.max(self.summary(g).area);
            }
            if amax <= 0.0 {
                amax = 1.0;
            }
            let layer = gs
                .iter()
                .map(|&g| {
                    let len = 0.5 * (self.pos(g) - ideal).abs() / win;
                    (g, self.cost(g, amax) + len)
                })
                .collect();
            layers.push(layer);
        }
        // Dynamic programming over the cuts: (total cost, index in the
        // layer before).
        let mut totals: Vec<Vec<(f32, usize)>> = Vec::with_capacity(layers.len());
        for (j, layer) in layers.iter().enumerate() {
            let row = layer
                .iter()
                .map(|&(g, c)| {
                    let p = self.pos(g);
                    if j == 0 {
                        let ok = self.piece_fits(lo, p, room, area);
                        return (if ok { c } else { f32::INFINITY }, usize::MAX);
                    }
                    let mut best = (f32::INFINITY, usize::MAX);
                    for (k, &(gp, _)) in layers[j - 1].iter().enumerate() {
                        let t = totals[j - 1][k].0;
                        let q = self.pos(gp);
                        if t + c < best.0 && q + min_len <= p && self.piece_fits(q, p, room, area) {
                            best = (t + c, k);
                        }
                    }
                    best
                })
                .collect();
            totals.push(row);
        }
        let last = layers.len() - 1;
        let (mut k, _) = layers[last]
            .iter()
            .enumerate()
            .filter(|(_, (g, _))| self.piece_fits(self.pos(*g), hi, 0.0, area))
            .map(|(k, _)| (k, totals[last][k].0))
            .filter(|(_, t)| t.is_finite())
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        let mut ats = vec![0.0; layers.len()];
        for j in (0..layers.len()).rev() {
            ats[j] = self.pos(layers[j][k].0);
            k = totals[j][k].1;
        }
        Some(ats)
    }
}

// --- Joint places ----------------------------------------------------------

/// The pin centers for the cut at `at`: in each region of the section, a
/// grid of target points by the region area, each moved to the nearest
/// point with room. A point has room when it has at least the hole radius
/// plus `WALL` to the outline, in the cut and over the hole depth.
fn pin_places(fr: &Frame, at: f32, v: &JointValues) -> Vec<Vec2> {
    let r = v.hole_d() / 2.0;
    let hd = v.hole_depth();
    let main = fr.section(at);
    let others: Vec<Vec<Vec<Vec2>>> = [-hd, -hd / 2.0, hd / 2.0, hd]
        .iter()
        .map(|d| fr.section(at + d))
        .collect();
    let clear = r + WALL;
    let spacing = (3.0 * v.pin_d).max(2.0 * clear + WALL);
    let mut out = Vec::new();
    for reg in regions(&main) {
        let size = reg.hi - reg.lo;
        let step = (size.min_element() / 24.0)
            .clamp(0.25, 2.0)
            .max((size.x * size.y / 20000.0).sqrt());
        let (nx, ny) = ((size.x / step) as usize, (size.y / step) as usize);
        let mut cands = Vec::new();
        for i in 0..nx {
            for j in 0..ny {
                let p = reg.lo + Vec2::new(i as f32 + 0.5, j as f32 + 0.5) * step;
                let ok = inside(p, &reg.loops)
                    && dist(p, &main) >= clear
                    && others.iter().all(|o| inside(p, o) && dist(p, o) >= clear);
                if ok {
                    cands.push(p);
                }
            }
        }
        if cands.is_empty() {
            continue;
        }
        let k = ((reg.area / v.pin_area) as usize)
            .max(2)
            .min(v.pin_max.max(1));
        let cols = ((k as f32 * size.x / size.y.max(1e-3)).sqrt().round() as usize).clamp(1, k);
        let rows = k.div_ceil(cols);
        let mut chosen: Vec<Vec2> = Vec::new();
        for t in 0..k {
            let (c, rw) = (t % cols, t / cols);
            let target = reg.lo
                + size
                    * Vec2::new(
                        (c as f32 + 0.5) / cols as f32,
                        (rw as f32 + 0.5) / rows as f32,
                    );
            let best = cands
                .iter()
                .filter(|p| chosen.iter().all(|q| q.distance(**p) >= spacing))
                .min_by(|a, b| a.distance(target).total_cmp(&b.distance(target)));
            if let Some(p) = best {
                chosen.push(*p);
            }
        }
        out.extend(
            chosen
                .into_iter()
                .map(|p| Vec2::new(round(p.x), round(p.y))),
        );
    }
    out
}

/// `x` rounded to 0.01 mm, for short numbers in the OpenSCAD text.
fn round(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}

/// The material on the line x = `x` in a section: (number of intervals,
/// total length).
fn column(loops: &[Vec<Vec2>], x: f32) -> (usize, f32) {
    let mut zs = Vec::new();
    for lp in loops {
        for i in 0..lp.len() {
            let (a, b) = (lp[i], lp[(i + 1) % lp.len()]);
            if (a.x <= x) != (b.x <= x) {
                zs.push(a.y + (x - a.x) * (b.y - a.y) / (b.x - a.x));
            }
        }
    }
    zs.sort_by(f32::total_cmp);
    let total = zs.as_chunks::<2>().0.iter().map(|[a, b]| b - a).sum();
    (zs.len() / 2, total)
}

/// The tab centers for the cut at `at`, as `tabs_2d()` of book-cover: in
/// each run of positions with full material, `floor(span / pitch)` tabs
/// (at least 1) at equal distances. "Full material" means: across the tab
/// width plus the clearance and `WALL`, and from the tab root to `WALL`
/// behind the socket, each line through the thin side crosses one solid
/// interval, and no line has less than 80 percent of the material of the
/// others. Thus a tab does not go through a hole or a cavity.
fn tab_places(fr: &Frame, at: f32, v: &JointValues) -> Vec<f32> {
    let g = v.clearance / 2.0;
    let hw = v.tab_tip / 2.0 + g + WALL;
    let secs: Vec<Vec<Vec<Vec2>>> = [
        -(TAB_ROOT + g),
        0.0,
        v.tab_depth / 2.0,
        v.tab_depth + g + WALL,
    ]
    .iter()
    .map(|d| fr.section(at + d))
    .collect();
    let pts = secs[1].iter().flatten();
    let x0 = pts.clone().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let x1 = pts.map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    if x1 - x0 < 2.0 * hw {
        return Vec::new();
    }
    let n = ((x1 - x0) / STEP) as usize;
    let xs: Vec<f32> = (0..n).map(|i| x0 + (i as f32 + 0.5) * STEP).collect();
    // For each sample: the columns of all sections.
    let cols: Vec<Vec<(usize, f32)>> = xs
        .iter()
        .map(|&x| secs.iter().map(|s| column(s, x)).collect())
        .collect();
    let h = (hw / STEP).ceil() as usize;
    let valid = |i: usize| -> bool {
        if i < h || i + h >= n {
            return false;
        }
        let window = cols[i - h..=i + h].iter().flatten();
        let max = window.clone().map(|c| c.1).fold(0.0f32, f32::max);
        window
            .clone()
            .all(|c| c.0 == 1 && c.1 >= 0.8 * max && c.1 > 0.0)
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if !valid(i) {
            i += 1;
            continue;
        }
        let start = i;
        while i + 1 < n && valid(i + 1) {
            i += 1;
        }
        let (c0, c1) = (xs[start], xs[i]);
        let span = c1 - c0 + 2.0 * hw;
        let count = ((span / v.tab_pitch.max(1.0)) as usize)
            .min((span / (2.0 * hw)) as usize)
            .max(1);
        let pitch = span / count as f32;
        out.extend((0..count).map(|t| round(c0 - hw + (t as f32 + 0.5) * pitch)));
        i += 1;
    }
    out
}

// --- OpenSCAD text ---------------------------------------------------------

/// The OpenSCAD module `tab(gap)`: the 2D dovetail tab of book-cover, with
/// round wide corners, grown by `gap`.
fn tab_module(v: &JointValues) -> String {
    let (a, t, d) = (v.tab_root / 2.0, v.tab_tip / 2.0, v.tab_depth);
    let pts = vec![
        Vec2::new(-a, -TAB_ROOT),
        Vec2::new(a, -TAB_ROOT),
        Vec2::new(a, 0.0),
        Vec2::new(t, d),
        Vec2::new(-t, d),
        Vec2::new(-a, 0.0),
    ];
    let r = scad::num(TAB_ROUND);
    format!(
        "module tab(gap) offset(delta = gap) offset(r = {r}) offset(delta = -{r}) {}\n",
        scad::polygon(&[pts])
    )
}

/// The OpenSCAD statement for the side of a cut that keeps a piece: the
/// lower side (local y < 0) with the tabs, or the upper side without the
/// sockets. `big` is larger than the part in all directions.
fn side(split: &Split, frame: Mat4, places: &Places, big: f32, lower: bool) -> String {
    let g = if split.joint == Joint::Dovetail {
        split.values.clearance / 2.0
    } else {
        0.0
    };
    let b = scad::num(big);
    let b2 = scad::num(2.0 * big);
    let mut half = if lower {
        format!("translate([-{b}, -{b}]) square([{b2}, {b}]);")
    } else {
        format!("translate([-{b}, 0]) square([{b2}, {b}]);")
    };
    if g > 0.0 {
        let y = if lower { -g } else { g };
        half = format!("translate([0, {}]) {half}", scad::num(y));
    }
    let mut tabs = String::new();
    for x in &places.tabs {
        let gap = if lower { -g } else { g };
        let _ = write!(
            tabs,
            " translate([{}, 0]) tab({});",
            scad::num(*x),
            scad::num(gap)
        );
    }
    let shape = match (tabs.is_empty(), lower) {
        (true, _) => half,
        (false, true) => format!("union() {{ {half}{tabs} }}"),
        (false, false) => format!("difference() {{ {half}{tabs} }}"),
    };
    format!(
        "{} linear_extrude(height = {b2}, center = true) {shape}",
        scad::multmatrix(frame)
    )
}

/// The `.scad` text for piece `i` (0 is the lowest). `import` imports the
/// part. The cuts of `split` are in order.
fn piece_scad(
    import: &str,
    i: usize,
    split: &Split,
    frames: &[Mat4],
    places: &[Places],
    big: f32,
) -> String {
    let mut s = String::from(HEADER);
    if split.joint == Joint::Dovetail {
        s.push_str(&tab_module(&split.values));
    }
    let mut keep = vec![import.to_string()];
    if i > 0 {
        keep.push(side(split, frames[i - 1], &places[i - 1], big, false));
    }
    if i < split.cuts.len() {
        keep.push(side(split, frames[i], &places[i], big, true));
    }
    let mut body = String::from("intersection() {\n");
    for k in keep {
        let _ = writeln!(body, "    {k}");
    }
    body.push('}');
    let v = &split.values;
    let mut holes = String::new();
    for k in [i.wrapping_sub(1), i] {
        let Some(p) = places.get(k) else {
            continue;
        };
        for c in &p.pins {
            let _ = writeln!(
                holes,
                "    {} translate([{}, 0, {}]) rotate([-90, 0, 0]) cylinder(d = {}, h = {}, center = true);",
                scad::multmatrix(frames[k]),
                scad::num(c.x),
                scad::num(c.y),
                scad::num(v.hole_d()),
                scad::num(2.0 * v.hole_depth()),
            );
        }
    }
    if holes.is_empty() {
        let _ = writeln!(s, "{body}");
    } else {
        let _ = writeln!(s, "difference() {{\n{body}\n{holes}}}");
    }
    s
}

/// The `.scad` text for one printed pin: a cylinder that stands on z = 0,
/// with a small chamfer at the two ends.
fn pin_scad(v: &JointValues) -> String {
    let r = v.pin_d / 2.0;
    let l = v.pin_len();
    let c = PIN_CHAMFER.min(r * 0.5).min(l * 0.25);
    let pts = [
        Vec2::new(0.0, 0.0),
        Vec2::new(r - c, 0.0),
        Vec2::new(r, c),
        Vec2::new(r, l - c),
        Vec2::new(r - c, l),
        Vec2::new(0.0, l),
    ];
    format!(
        "{HEADER}rotate_extrude() {}\n",
        scad::polygon(&[pts.to_vec()])
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::cube;

    fn plate(x: f32, y: f32, z: f32) -> Mesh {
        transformed(&cube(1.0), Mat4::from_scale(Vec3::new(x, y, z)))
    }

    /// A plate along x, `w` wide in y, with its top at `tops[i]` at
    /// `xs[i]` and its bottom at z = 0.
    fn profile_plate(xs: &[f32], tops: &[f32], w: f32) -> Mesh {
        let mut soup = Vec::new();
        let mut quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
            soup.push([a, b, c]);
            soup.push([a, c, d]);
        };
        let p = Vec3::new;
        for i in 0..xs.len() - 1 {
            let (x0, x1, t0, t1) = (xs[i], xs[i + 1], tops[i], tops[i + 1]);
            quad(
                p(x0, 0.0, 0.0),
                p(x1, 0.0, 0.0),
                p(x1, 0.0, t1),
                p(x0, 0.0, t0),
            );
            quad(p(x0, w, 0.0), p(x0, w, t0), p(x1, w, t1), p(x1, w, 0.0));
            quad(
                p(x0, 0.0, 0.0),
                p(x0, w, 0.0),
                p(x1, w, 0.0),
                p(x1, 0.0, 0.0),
            );
            quad(p(x0, 0.0, t0), p(x1, 0.0, t1), p(x1, w, t1), p(x0, w, t0));
        }
        let (xa, ta) = (xs[0], tops[0]);
        let (xb, tb) = (xs[xs.len() - 1], tops[tops.len() - 1]);
        quad(p(xa, 0.0, 0.0), p(xa, 0.0, ta), p(xa, w, ta), p(xa, w, 0.0));
        quad(p(xb, 0.0, 0.0), p(xb, w, 0.0), p(xb, w, tb), p(xb, 0.0, tb));
        Mesh::from_soup(&soup)
    }

    fn split_of(cuts: &[f32], axis: usize, joint: Joint) -> Split {
        Split {
            cuts: cuts.iter().map(|&at| Cut { axis, at }).collect(),
            joint,
            values: JointValues::default(),
        }
    }

    #[test]
    fn joint_names_go_both_ways() {
        for j in Joint::ALL {
            assert_eq!(Joint::from_name(j.name()), Some(j));
        }
        assert_eq!(Joint::from_name("plug"), None);
    }

    #[test]
    fn fits_lets_any_side_be_the_height() {
        let area = Area::default();
        assert!(!fits(&plate(300.0, 100.0, 5.0), &area));
        assert!(fits(&plate(200.0, 100.0, 5.0), &area));
        // 230 mm only fits as the height (250).
        assert!(fits(&plate(230.0, 100.0, 5.0), &area));
        assert!(fits(&plate(5.0, 230.0, 100.0), &area));
        assert!(!fits(&plate(230.0, 230.0, 5.0), &area));
        assert!(fits(&Mesh::default(), &area));
    }

    #[test]
    fn plate_is_cut_in_two_across_x() {
        let m = plate(300.0, 100.0, 5.0);
        assert_eq!(m.open_edges(), 0);
        let area = Area::default();
        for joint in Joint::ALL {
            let s = propose(&m, &area, joint).expect("a split");
            assert_eq!(s.joint, joint);
            assert_eq!(s.pieces(), 2, "{joint:?}");
            assert_eq!(s.cuts[0].axis, 0);
            assert!((s.cuts[0].at - 150.0).abs() < 0.01, "{s:?}");
            let sizes = piece_sizes(&m, &s);
            assert_eq!(sizes.len(), 2);
            for size in &sizes {
                assert!(fits_size(*size, &area), "{size}");
            }
            let tab = if joint == Joint::Dovetail { 5.0 } else { 0.0 };
            assert!(
                (sizes[0] - Vec3::new(150.0 + tab, 100.0, 5.0)).length() < 0.01,
                "{sizes:?}"
            );
            assert!(
                (sizes[1] - Vec3::new(150.0, 100.0, 5.0)).length() < 0.01,
                "{sizes:?}"
            );
        }
    }

    #[test]
    fn a_part_that_fits_gets_no_split() {
        assert!(propose(&plate(200.0, 100.0, 5.0), &Area::default(), Joint::Flat).is_none());
    }

    #[test]
    fn long_plate_is_cut_in_three() {
        let m = plate(600.0, 80.0, 5.0);
        let s = propose(&m, &Area::default(), Joint::Dovetail).unwrap();
        assert_eq!(s.pieces(), 3, "{s:?}");
        assert!((s.cuts[0].at - 200.0).abs() < 0.01, "{s:?}");
        assert!((s.cuts[1].at - 400.0).abs() < 0.01, "{s:?}");
    }

    #[test]
    fn tall_part_is_cut_across_z() {
        let m = plate(60.0, 40.0, 400.0);
        let s = propose(&m, &Area::default(), Joint::Pins).unwrap();
        assert_eq!(s.pieces(), 2);
        assert_eq!(s.cuts[0].axis, 2);
        assert!((s.cuts[0].at - 200.0).abs() < 0.01, "{s:?}");
    }

    #[test]
    fn a_part_too_large_in_two_directions_gets_no_split() {
        assert!(propose(&plate(300.0, 300.0, 5.0), &Area::default(), Joint::Flat).is_none());
    }

    #[test]
    fn the_cut_keeps_away_from_a_thin_groove() {
        // A 1 mm thin groove across the plate from x = 140 to 160.
        let xs = [0.0, 139.5, 140.5, 159.5, 160.5, 300.0];
        let tops = [5.0, 5.0, 1.0, 1.0, 5.0, 5.0];
        let m = profile_plate(&xs, &tops, 100.0);
        assert_eq!(m.open_edges(), 0);
        assert!(m.volume() > 0.0);
        let s = propose(&m, &Area::default(), Joint::Flat).unwrap();
        assert_eq!(s.pieces(), 2);
        let at = s.cuts[0].at;
        assert!(!(138.0..=162.0).contains(&at), "{at}");
        assert!((at - 150.0).abs() <= 30.0, "{at}");
    }

    #[test]
    fn a_short_bed_makes_the_tabs_count() {
        // 2 x 103 mm fits a 105 mm bed, but 103 + 5 mm of tabs does not.
        let m = plate(206.0, 50.0, 5.0);
        let area = Area {
            w: 105.0,
            d: 105.0,
            h: 105.0,
        };
        assert_eq!(propose(&m, &area, Joint::Flat).unwrap().pieces(), 2);
        assert_eq!(propose(&m, &area, Joint::Dovetail).unwrap().pieces(), 3);
    }

    #[test]
    fn section_summary_counts_regions_holes_and_thin_walls() {
        let sq = |x: f32, y: f32, w: f32, h: f32| {
            vec![
                Vec2::new(x, y),
                Vec2::new(x + w, y),
                Vec2::new(x + w, y + h),
                Vec2::new(x, y + h),
            ]
        };
        let mut hole = sq(10.0, 10.0, 10.0, 10.0);
        hole.reverse();
        let loops = vec![
            sq(0.0, 0.0, 100.0, 50.0),
            hole,
            sq(200.0, 0.0, 300.0, 1.0),
            sq(600.0, 0.0, 3.0, 3.0),
        ];
        let s = summarize(&loops);
        assert_eq!(s.regions, 3);
        assert_eq!(s.holes, 1);
        assert_eq!(s.small, 1);
        assert_eq!(s.thin, 1);
        assert!((s.area - (5000.0 - 100.0 + 300.0 + 9.0)).abs() < 0.01);
    }

    #[test]
    fn dovetail_tabs_follow_the_book_cover_pitch() {
        let m = plate(300.0, 100.0, 5.0);
        let p = places(&m, &split_of(&[150.0], 0, Joint::Dovetail));
        assert_eq!(p.len(), 1);
        let tabs = &p[0].tabs;
        assert_eq!(tabs.len(), 3, "{tabs:?}");
        assert!((tabs[1] - 50.0).abs() < 0.3, "{tabs:?}");
        assert!((tabs[0] + tabs[2] - 100.0).abs() < 0.3, "{tabs:?}");
        assert!(tabs[0] >= 7.1 && tabs[2] <= 92.9, "{tabs:?}");
    }

    #[test]
    fn dovetail_tabs_skip_a_hole_in_the_cut() {
        // A closed cavity along x from x = 50 to 250: the section at the
        // cut is a ring.
        let mut m = plate(300.0, 100.0, 20.0);
        let cavity = transformed(
            &cube(1.0),
            Mat4::from_translation(Vec3::new(50.0, 40.0, 5.0))
                * Mat4::from_scale(Vec3::new(200.0, 20.0, 10.0)),
        );
        let base = m.vertices.len() as u32;
        m.vertices.extend(cavity.vertices.iter());
        // The cavity faces point into the cavity.
        m.triangles.extend(
            cavity
                .triangles
                .iter()
                .map(|t| [t[0] + base, t[2] + base, t[1] + base]),
        );
        assert!((m.volume() - (300.0 * 100.0 * 20.0 - 200.0 * 20.0 * 10.0)).abs() < 1.0);
        let fr = Frame::new(&m, 0);
        let s = fr.section(150.0);
        assert_eq!(regions(&s).len(), 1);
        let tabs = tab_places(&fr, 150.0, &JointValues::default());
        // No tab position may overlap y = 39..61 (the hole and its walls).
        for t in &tabs {
            assert!(*t + 7.1 <= 40.0 || *t - 7.1 >= 60.0, "{tabs:?}");
        }
        assert!(tabs.len() >= 2, "{tabs:?}");
    }

    #[test]
    fn pins_need_room_in_the_section() {
        let thin = plate(300.0, 100.0, 5.0);
        let p = places(&thin, &split_of(&[150.0], 0, Joint::Pins));
        assert!(p[0].pins.is_empty(), "{p:?}");
        let thick = plate(300.0, 100.0, 10.0);
        let p = places(&thick, &split_of(&[150.0], 0, Joint::Pins));
        let pins = &p[0].pins;
        assert_eq!(pins.len(), 2, "{pins:?}");
        let mut xs: Vec<f32> = pins.iter().map(|p| p.x).collect();
        xs.sort_by(f32::total_cmp);
        assert!(
            (xs[0] - 25.0).abs() < 1.0 && (xs[1] - 75.0).abs() < 1.0,
            "{pins:?}"
        );
        for p in pins {
            assert!((p.y.abs() - 5.0).abs() < 0.5, "{pins:?}");
        }
    }

    #[test]
    fn a_large_section_gets_more_pins() {
        let m = plate(60.0, 60.0, 400.0);
        let p = places(&m, &split_of(&[200.0], 2, Joint::Pins));
        // 3600 mm² / 400 = 9, at most 8.
        assert_eq!(p[0].pins.len(), 8, "{p:?}");
    }

    #[test]
    fn piece_scad_has_tabs_on_one_side_and_sockets_on_the_other() {
        let m = plate(300.0, 100.0, 5.0);
        let s = split_of(&[150.0], 0, Joint::Dovetail);
        let p = places(&m, &s);
        let f = vec![cut_frame(&m, &s.cuts[0])];
        let a = piece_scad("import(\"a.stl\");", 0, &s, &f, &p, 640.0);
        let b = piece_scad("import(\"a.stl\");", 1, &s, &f, &p, 640.0);
        assert!(a.contains("module tab(gap)"), "{a}");
        assert!(a.contains("union() { translate([0, -0.1]) translate([-640, -640]) square([1280, 640]); translate([17.1, 0]) tab(-0.1);"), "{a}");
        assert_eq!(a.matches("tab(-0.1);").count(), 3, "{a}");
        assert!(b.contains("difference() { translate([0, 0.1]) translate([-640, 0]) square([1280, 640]); translate([17.1, 0]) tab(0.1);"), "{b}");
        assert_eq!(b.matches("tab(0.1);").count(), 3, "{b}");
        // Local y is model x, local x is model y.
        assert!(
            a.contains("multmatrix([[0, 1, 0, 150], [1, 0, 0, 0], [0, 0, -1, 0], [0, 0, 0, 1]])"),
            "{a}"
        );
    }

    // --- Tests with OpenSCAD ------------------------------------------------

    use std::sync::atomic::AtomicU64;

    fn no_cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    /// Renders an OpenSCAD source with the default configuration.
    fn render(src: &str) -> Mesh {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "cocad-split-test-{}-{}.scad",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, src).unwrap();
        let out = crate::openscad::render(&Config::default(), &path, &[], &no_cancel());
        let _ = std::fs::remove_file(&path);
        out.unwrap().mesh
    }

    /// The volume of the intersection of `a` and `b`. A far unit cube keeps
    /// the result from being empty.
    fn overlap(a: &Mesh, b: &Mesh) -> f32 {
        let m = scad::run(
            &Config::default(),
            &[a, b],
            |i| {
                format!(
                    "union() {{ intersection() {{ {} {} }} translate([1000, 1000, 1000]) cube(1); }}\n",
                    i[0], i[1]
                )
            },
            &no_cancel(),
        )
        .unwrap();
        m.volume() - 1.0
    }

    fn total(pieces: &[Piece]) -> f32 {
        pieces.iter().map(|p| p.mesh.volume()).sum()
    }

    fn run_split(m: &Mesh, joint: Joint) -> (Split, Vec<Piece>) {
        let area = Area::default();
        let s = propose(m, &area, joint).unwrap();
        let pieces = apply(&Config::default(), m, &s, &no_cancel()).unwrap();
        for p in &pieces {
            assert!(fits(&p.mesh, &area), "{} does not fit", p.label);
            assert_eq!(p.mesh.open_edges(), 0, "{} is open", p.label);
        }
        (s, pieces)
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn plate_with_a_flat_joint_keeps_the_volume() {
        let m = render("cube([300, 100, 5]);");
        let v = m.volume();
        let (_, pieces) = run_split(&m, Joint::Flat);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].label, "1/2");
        assert_eq!(pieces[1].label, "2/2");
        let t = total(&pieces);
        assert!((t - v).abs() < 0.005 * v, "{t} {v}");
        let b0 = pieces[0].mesh.bounds().unwrap();
        let b1 = pieces[1].mesh.bounds().unwrap();
        assert!((b0.max.x - 150.0).abs() < 0.01 && (b1.min.x - 150.0).abs() < 0.01);
        eprintln!("flat: part {v:.1}, pieces {t:.1} mm³");
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn plate_with_pins_holes_and_pins_near_cancel() {
        let m = render("cube([300, 100, 10]);");
        let v = m.volume();
        let (s, pieces) = run_split(&m, Joint::Pins);
        let pins: Vec<&Piece> = pieces.iter().filter(|p| p.pin).collect();
        assert_eq!(pieces.len() - pins.len(), 2);
        assert_eq!(pins.len(), 2);
        assert!(pins.iter().all(|p| p.label == "pin"));
        let jv = s.values;
        let holes = v - total(&pieces[..2]);
        let r = jv.hole_d() / 2.0;
        let expect = 2.0 * std::f32::consts::PI * r * r * 2.0 * jv.hole_depth();
        assert!(
            (holes - expect).abs() < 0.03 * expect,
            "holes {holes} expect {expect}"
        );
        let pin_v = pins[0].mesh.volume();
        let pb = pins[0].mesh.bounds().unwrap();
        assert!((pb.size().z - jv.pin_len()).abs() < 0.01, "{pb:?}");
        assert!(pb.min.z.abs() < 0.01);
        let t = total(&pieces);
        assert!((t - v).abs() < 0.005 * v, "{t} {v}");
        // The pins fill all of the holes but the clearance and the free ends.
        assert!(holes - 2.0 * pin_v > 0.0 && holes - 2.0 * pin_v < 0.25 * holes);
        eprintln!(
            "pins: part {v:.1}, pieces {:.1}, holes {holes:.1} (expect {expect:.1}), pin {pin_v:.1} mm³",
            total(&pieces[..2])
        );
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn plate_with_a_dovetail_has_the_tabs_in_the_sockets() {
        let m = render("cube([300, 100, 5]);");
        let v = m.volume();
        let (s, pieces) = run_split(&m, Joint::Dovetail);
        assert_eq!(pieces.len(), 2);
        let jv = s.values;
        let at = s.cuts[0].at;
        let b0 = pieces[0].mesh.bounds().unwrap();
        let b1 = pieces[1].mesh.bounds().unwrap();
        // The tabs of piece 1 go into piece 2.
        assert!(
            (b0.max.x - (at + jv.tab_depth - jv.clearance / 2.0)).abs() < 0.01,
            "{b0:?}"
        );
        assert!(
            (b1.min.x - (at + jv.clearance / 2.0)).abs() < 0.01,
            "{b1:?}"
        );
        // The gap: the clearance along the cut line and around 3 tabs.
        let flank = Vec2::new((jv.tab_tip - jv.tab_root) / 2.0, jv.tab_depth).length();
        let line = 100.0 + 3.0 * (2.0 * flank + jv.tab_tip - jv.tab_root);
        let expect = jv.clearance * line * 5.0;
        let gap = v - total(&pieces);
        assert!(
            (gap - expect).abs() < 0.2 * expect,
            "gap {gap} expect {expect}"
        );
        let o = overlap(&pieces[0].mesh, &pieces[1].mesh);
        assert!(o.abs() < 0.01, "overlap {o}");
        eprintln!("dovetail: part {v:.1}, gap {gap:.2} (expect {expect:.2}), overlap {o:.4} mm³");
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn rounded_plate_with_holes_splits_with_each_joint() {
        let m = render(
            "$fn = 48;
             difference() {
                 hull() for (x = [10, 310], y = [10, 110]) translate([x, y, 0]) cylinder(r = 10, h = 12);
                 for (x = [60, 160, 260]) translate([x, 60, -1]) cylinder(d = 30, h = 14);
                 translate([-1, 20, 4]) cube([400, 8, 4]);
             }",
        );
        let v = m.volume();
        for joint in Joint::ALL {
            let (s, pieces) = run_split(&m, joint);
            let at = s.cuts[0].at;
            // The cut keeps 2 mm or more away from the hole at x = 160
            // (r = 15).
            assert!((at - 160.0).abs() >= 17.0, "{joint:?} {at}");
            let parts: Vec<&Piece> = pieces.iter().filter(|p| !p.pin).collect();
            let t = total(&pieces);
            let p = places(&m, &s);
            eprintln!(
                "{}: cut at {at}, {} pieces, {} tabs, {} pins, part {v:.0}, sum {t:.0} mm³",
                joint.name(),
                parts.len(),
                p[0].tabs.len(),
                p[0].pins.len(),
            );
            assert!((t - v).abs() < 0.01 * v, "{t} {v}");
            if joint == Joint::Dovetail {
                assert!(!p[0].tabs.is_empty());
                assert!(overlap(&parts[0].mesh, &parts[1].mesh).abs() < 0.01);
            }
            if joint == Joint::Pins {
                assert!(!p[0].pins.is_empty());
            }
        }
    }
}
