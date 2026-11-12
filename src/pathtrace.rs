//! Progressive CPU path tracer for photo renders: plastic model on a studio
//! floor, soft key light, environment fill, optional printed layer lines.

use crate::mesh::Mesh;
use glam::{Vec3, Vec4};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Studio {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Linear RGB albedo of the model.
    pub color: Vec3,
    /// 0 = mirror-like, 1 = matte.
    pub roughness: f32,
    /// Bump the sides with print layer lines of this height (0 = off).
    pub layer_lines: f32,
    pub studio: Studio,
    pub exposure: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    /// Vertical field of view in radians.
    pub fov: f32,
}

/// Coat roughness for a finish name.
pub fn finish_roughness(name: &str) -> f32 {
    match name {
        "matte" => 0.9,
        "gloss" => 0.15,
        _ => 0.45,
    }
}

/// The viewer's iso view, fitted to the mesh.
pub fn iso_camera(mesh: &Mesh, aspect: f32) -> Camera {
    fit_camera(mesh, -0.9, 0.55, aspect)
}

/// A view from this direction with the whole mesh in frame.
pub fn fit_camera(mesh: &Mesh, yaw: f32, pitch: f32, aspect: f32) -> Camera {
    let b = mesh.bounds().expect("empty mesh");
    let fov = 0.6f32;
    let r = (b.size().length() * 0.5).max(1.0);
    // Fit the bounding sphere in the narrower direction.
    let half = if aspect < 1.0 {
        (fov * 0.5).tan() * aspect
    } else {
        (fov * 0.5).tan()
    };
    let dist = r / half.atan().sin() * 1.05;
    let dir = Vec3::new(
        pitch.cos() * yaw.cos(),
        pitch.cos() * yaw.sin(),
        pitch.sin(),
    );
    Camera {
        eye: b.center() + dir * dist,
        target: b.center(),
        fov,
    }
}

/// The latest image of a running render.
#[derive(Default)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    /// sRGB, 4 bytes per pixel.
    pub rgba: Vec<u8>,
    pub samples: u32,
}

/// The albedo of triangle ranges. It replaces the color of the `Look`, for
/// example to show each part in its own color.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paint {
    /// The end of each range (exclusive), ascending.
    ends: Vec<u32>,
    /// Linear RGB albedo of each range.
    colors: Vec<Vec3>,
}

impl Paint {
    /// `ranges` holds the end triangle (exclusive) and the linear color of
    /// each range, in triangle order. The first range starts at 0.
    pub fn new(ranges: impl IntoIterator<Item = (u32, Vec3)>) -> Paint {
        let (ends, colors) = ranges.into_iter().unzip();
        Paint { ends, colors }
    }

    /// The color of triangle `tri`, or `None` after the last range.
    pub fn color(&self, tri: u32) -> Option<Vec3> {
        self.colors
            .get(self.ends.partition_point(|&e| e <= tri))
            .copied()
    }
}

/// A section cut for the photo. The planes are in model space: a part can
/// have an offset (the explode), and the cut moves with the part. A point p
/// of a part with offset o stays when `n.dot(p - o) + w >= 0` for each
/// plane `(n, w)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clip {
    planes: Vec<Vec4>,
    /// The end triangle (exclusive) of each part, ascending.
    ends: Vec<u32>,
    /// The offset of each part.
    offsets: Vec<Vec3>,
}

impl Clip {
    /// `parts` holds the end triangle (exclusive) and the offset of each
    /// part, in triangle order. Triangles after the last part have no
    /// offset.
    pub fn new(planes: Vec<Vec4>, parts: impl IntoIterator<Item = (u32, Vec3)>) -> Clip {
        let (ends, offsets) = parts.into_iter().unzip();
        Clip {
            planes,
            ends,
            offsets,
        }
    }

    fn offset(&self, tri: u32) -> Vec3 {
        (self.offsets.get(self.ends.partition_point(|&e| e <= tri)))
            .copied()
            .unwrap_or(Vec3::ZERO)
    }

    /// Point `p` on triangle `tri` is on the kept side of all planes.
    fn keeps(&self, p: Vec3, tri: u32) -> bool {
        let q = p - self.offset(tri);
        (self.planes.iter()).all(|c| c.truncate().dot(q) + c.w >= 0.0)
    }

    /// Where the ray `o + t d` goes into the kept space of the part of
    /// triangle `tri`: (t, normal of the cut face). The normal points out of
    /// the kept space, to the ray. `None` when the ray starts in the kept
    /// space.
    fn entry(&self, o: Vec3, d: Vec3, tri: u32) -> Option<(f32, Vec3)> {
        let q = o - self.offset(tri);
        let mut best: Option<(f32, Vec3)> = None;
        for c in &self.planes {
            let n = c.truncate();
            let nd = n.dot(d);
            if nd > 0.0 {
                let t = -(n.dot(q) + c.w) / nd;
                if best.is_none_or(|(bt, _)| t > bt) {
                    best = Some((t, -n.normalize_or_zero()));
                }
            }
        }
        best.filter(|(t, _)| *t > 0.0)
    }
}

/// What the path tracer renders: a mesh and, if set, the colors of its
/// triangles and a section cut. Without a paint, the model has the color of
/// the `Look`.
pub struct Model {
    pub mesh: Arc<Mesh>,
    pub paint: Option<Arc<Paint>>,
    pub clip: Option<Arc<Clip>>,
}

impl From<Arc<Mesh>> for Model {
    fn from(mesh: Arc<Mesh>) -> Model {
        Model {
            mesh,
            paint: None,
            clip: None,
        }
    }
}

pub struct Job {
    pub frame: Arc<Mutex<Frame>>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// Starts rendering on all cores until `max_samples` per pixel.
    pub fn start(
        model: impl Into<Model>,
        cam: Camera,
        look: Look,
        width: usize,
        height: usize,
        max_samples: u32,
    ) -> Job {
        let frame = Arc::new(Mutex::new(Frame {
            width,
            height,
            ..Default::default()
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let (f, c) = (frame.clone(), cancel.clone());
        let model = model.into();
        std::thread::spawn(move || {
            let mut scene = Scene::cached(&model.mesh, look);
            scene.paint = model.paint;
            scene.clip = model.clip.filter(|c| !c.planes.is_empty());
            let mut accum = vec![Vec3::ZERO; width * height];
            for pass in 0..max_samples {
                if c.load(Ordering::Relaxed) {
                    return;
                }
                scene.pass(&cam, width, height, pass, &mut accum, &c);
                if c.load(Ordering::Relaxed) {
                    return;
                }
                let rgba = tonemap(&accum, pass + 1, look.exposure);
                let mut fr = f.lock().unwrap();
                fr.rgba = rgba;
                fr.samples = pass + 1;
            }
        });
        Job { frame, cancel }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn tonemap(accum: &[Vec3], spp: u32, exposure: f32) -> Vec<u8> {
    let k = exposure / spp as f32;
    let mut out = Vec::with_capacity(accum.len() * 4);
    for c in accum {
        // ACES filmic fit (Narkowicz), then sRGB.
        let x = *c * k;
        let m = (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14);
        for ch in m.to_array() {
            let v = ch.clamp(0.0, 1.0);
            let s = if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            };
            out.push((s * 255.0 + 0.5) as u8);
        }
        out.push(255);
    }
    out
}

/// sRGB hex color to linear RGB.
pub fn srgb(hex: u32) -> Vec3 {
    linear([(hex >> 16) as u8, (hex >> 8) as u8, hex as u8])
}

/// sRGB color to linear RGB.
pub fn linear(c: [u8; 3]) -> Vec3 {
    let f = |c: u8| {
        let v = c as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    Vec3::new(f(c[0]), f(c[1]), f(c[2]))
}

/// Writes RGBA as PNG.
pub fn save_png(path: &std::path::Path, f: &Frame) -> anyhow::Result<()> {
    write_png(std::io::BufWriter::new(std::fs::File::create(path)?), f)
}

/// RGB PNG file data, packed small for tool results.
pub fn png_bytes(f: &Frame) -> anyhow::Result<Vec<u8>> {
    let rgb: Vec<u8> = (f.rgba.iter().enumerate())
        .filter(|(i, _)| i % 4 != 3)
        .map(|(_, b)| *b)
        .collect();
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, f.width as u32, f.height as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Best);
    enc.write_header()?.write_image_data(&rgb)?;
    Ok(out)
}

fn write_png(out: impl std::io::Write, f: &Frame) -> anyhow::Result<()> {
    let mut enc = png::Encoder::new(out, f.width as u32, f.height as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&f.rgba)?;
    Ok(())
}

// ---------------------------------------------------------------- scene ----

struct Tri {
    v0: Vec3,
    e1: Vec3,
    e2: Vec3,
    /// Shading normals at the corners (smoothed below 30 degrees).
    n: [Vec3; 3],
    /// Face normal, taken from a neighbour for sliver triangles.
    g: Vec3,
    /// The index of the triangle in the mesh. The BVH changes the order.
    id: u32,
}

#[derive(Clone, Copy)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// Leaf: first triangle. Inner: left child (right = left + 1).
    start: u32,
    /// Triangles in a leaf, 0 for an inner node.
    count: u32,
}

/// The part of a scene that depends only on the mesh. It is slow to build.
struct Geometry {
    tris: Vec<Tri>,
    nodes: Vec<Node>,
    floor: f32,
    center: Vec3,
    radius: f32,
}

struct Scene {
    geo: Arc<Geometry>,
    look: Look,
    paint: Option<Arc<Paint>>,
    /// The section cut. The rays ignore the removed side of the mesh.
    clip: Option<Arc<Clip>>,
    key_dir: Vec3,
    key_cos: f32,
    key_radiance: f32,
}

const EPS: f32 = 1e-4;

impl std::ops::Deref for Scene {
    type Target = Geometry;
    fn deref(&self) -> &Geometry {
        &self.geo
    }
}

/// The geometry of the mesh that was traced last. A new render of the same
/// mesh (camera or look change) uses it again.
type Slot = Mutex<Option<(Weak<Mesh>, Arc<Geometry>)>>;
static LAST: Slot = Mutex::new(None);

impl Geometry {
    /// The lock stays held during the build. A second render of the same mesh
    /// then waits for the first build and does not start its own.
    fn cached(mesh: &Arc<Mesh>) -> Arc<Geometry> {
        Geometry::cached_in(&LAST, mesh)
    }

    fn cached_in(slot: &Slot, mesh: &Arc<Mesh>) -> Arc<Geometry> {
        let mut last = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((m, geo)) = last.as_ref()
            && std::ptr::eq(m.as_ptr(), Arc::as_ptr(mesh))
        {
            return geo.clone();
        }
        // Release the old geometry before the build, to keep peak memory low.
        *last = None;
        let geo = Arc::new(Geometry::new(mesh));
        *last = Some((Arc::downgrade(mesh), geo.clone()));
        geo
    }

    fn new(mesh: &Mesh) -> Geometry {
        let b = mesh.bounds().expect("empty mesh");
        let face_n = robust_normals(mesh);
        let mut vert_faces: Vec<Vec<u32>> = vec![Vec::new(); mesh.vertices.len()];
        for (i, t) in mesh.triangles.iter().enumerate() {
            for &v in t {
                vert_faces[v as usize].push(i as u32);
            }
        }
        let flat = flat_faces(mesh, &face_n);
        let cos30 = 30f32.to_radians().cos();
        let mut tris: Vec<Tri> = mesh
            .triangles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let [a, bb, c] = mesh.tri(i);
                let fn_ = face_n[i];
                if flat[i] {
                    return Tri {
                        v0: a,
                        e1: bb - a,
                        e2: c - a,
                        n: [fn_; 3],
                        g: fn_,
                        id: i as u32,
                    };
                }
                let n = t.map(|v| {
                    let s: Vec3 = vert_faces[v as usize]
                        .iter()
                        .map(|&f| face_n[f as usize])
                        .filter(|m| m.dot(fn_) > cos30)
                        .sum();
                    s.normalize_or(fn_)
                });
                Tri {
                    v0: a,
                    e1: bb - a,
                    e2: c - a,
                    n,
                    g: fn_,
                    id: i as u32,
                }
            })
            .collect();

        let n = tris.len();
        let mut nodes = Vec::with_capacity(n * 2 / 3 + 1);
        nodes.push(Node {
            min: Vec3::ZERO,
            max: Vec3::ZERO,
            start: 0,
            count: 0,
        });
        build(&mut tris, &mut nodes, 0, 0, n);
        Geometry {
            tris,
            nodes,
            floor: b.min.z - 1e-3,
            center: b.center(),
            radius: (b.size().length() * 0.5).max(1.0),
        }
    }
}

impl Scene {
    #[cfg(test)]
    fn new(mesh: &Mesh, look: Look) -> Scene {
        Scene::with(Arc::new(Geometry::new(mesh)), look)
    }

    fn cached(mesh: &Arc<Mesh>, look: Look) -> Scene {
        Scene::with(Geometry::cached(mesh), look)
    }

    fn with(geo: Arc<Geometry>, look: Look) -> Scene {
        let key_dir = Vec3::new(0.45, -0.55, 0.85).normalize();
        let key_cos = 8f32.to_radians().cos();
        let solid_angle = 2.0 * std::f32::consts::PI * (1.0 - key_cos);
        let key_irradiance = match look.studio {
            Studio::Dark => 2.6,
            Studio::Light => 1.6,
        };
        Scene {
            geo,
            look,
            paint: None,
            clip: None,
            key_dir,
            key_cos,
            key_radiance: key_irradiance / solid_angle,
        }
    }

    /// Environment radiance. `key` adds the key light when the ray sees it.
    fn env(&self, d: Vec3, key: bool) -> Vec3 {
        let up = d.z.max(0.0);
        let (horizon, zenith, top) = match self.look.studio {
            Studio::Dark => (Vec3::splat(0.012), Vec3::new(0.03, 0.032, 0.038), 0.35),
            Studio::Light => (Vec3::splat(0.75), Vec3::new(0.95, 0.96, 0.98), 0.6),
        };
        // Sky gradient plus a large soft box overhead.
        let mut c = horizon.lerp(zenith, up.sqrt()) + Vec3::splat(top * up.powi(6));
        if key && d.dot(self.key_dir) > self.key_cos {
            c += Vec3::splat(self.key_radiance);
        }
        c
    }

    fn floor_albedo(&self) -> Vec3 {
        match self.look.studio {
            Studio::Dark => Vec3::splat(0.05),
            Studio::Light => Vec3::splat(0.7),
        }
    }

    /// Nearest model hit: (t, triangle, u, v). With a section cut, the hits
    /// on the removed side do not count.
    fn hit_mesh(&self, o: Vec3, d: Vec3, t_max: f32) -> Option<(f32, usize, f32, f32)> {
        let inv = d.recip();
        let mut best: Option<(f32, usize, f32, f32)> = None;
        let mut t_best = t_max;
        // With a section cut, a back face wins a near tie with a front face.
        // Where a part touches another part, the ray then finds the back
        // face of the cut part and shows its cut face (see `cap`).
        let tie = |t: f32| match self.clip {
            Some(_) => EPS * (1.0 + t.abs()),
            None => 0.0,
        };
        let mut best_back = false;
        let mut stack = [0u32; 64];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let node = &self.nodes[stack[sp] as usize];
            if !slab(node.min, node.max, o, inv, t_best + tie(t_best)) {
                continue;
            }
            if node.count > 0 {
                for i in node.start..node.start + node.count {
                    let tri = &self.tris[i as usize];
                    let Some((t, u, v)) = intersect(tri, o, d) else {
                        continue;
                    };
                    let take = match &self.clip {
                        None => t < t_best,
                        Some(c) => {
                            let back = tri.g.dot(d) > 0.0;
                            let near = (t - t_best).abs() <= tie(t_best);
                            (t < t_best || (near && back && !best_back))
                                && !(near && best_back && !back)
                                && c.keeps(o + d * t, tri.id)
                        }
                    };
                    if take {
                        t_best = t;
                        best_back = tri.g.dot(d) > 0.0;
                        best = Some((t, i as usize, u, v));
                    }
                }
            } else if sp + 2 <= stack.len() {
                stack[sp] = node.start;
                stack[sp + 1] = node.start + 1;
                sp += 2;
            }
        }
        best
    }

    /// The cut face in front of the hit `(t, i)` of `hit_mesh`: (t, normal).
    /// When the ray leaves the solid at the hit (a back face) and went into
    /// the kept space after its start, the ray went into the solid through a
    /// cut face. A shadow ray needs no cut face: the hit behind it occludes
    /// too.
    fn cap(&self, o: Vec3, d: Vec3, t: f32, i: usize) -> Option<(f32, Vec3)> {
        let clip = self.clip.as_ref()?;
        let tri = &self.tris[i];
        if tri.g.dot(d) <= 0.0 {
            return None;
        }
        clip.entry(o, d, tri.id).filter(|(tc, _)| *tc < t)
    }

    fn occluded(&self, o: Vec3, d: Vec3) -> bool {
        self.hit_mesh(o, d, f32::MAX).is_some()
    }

    fn trace(&self, mut o: Vec3, mut d: Vec3, rng: &mut Rng) -> Vec3 {
        let mut l = Vec3::ZERO;
        let mut thr = Vec3::ONE;
        let mut specular = true;
        for bounce in 0..6 {
            let floor_t = if d.z < 0.0 && o.z > self.floor {
                (self.floor - o.z) / d.z
            } else {
                f32::MAX
            };
            let hit = self.hit_mesh(o, d, floor_t);
            let cap = hit.and_then(|(t, i, _, _)| self.cap(o, d, t, i));
            let (p, mut n, gn, albedo, rough, spec) = match (hit, cap) {
                (Some((_, i, _, _)), Some((tc, cap))) => {
                    // The ray goes into the solid through the cut face.
                    let albedo = (self.paint.as_ref())
                        .and_then(|paint| paint.color(self.tris[i].id))
                        .unwrap_or(self.look.color);
                    (o + d * tc, cap, cap, albedo, self.look.roughness, 0.05)
                }
                (Some((t, i, u, v)), None) => {
                    let tri = &self.tris[i];
                    let gn = tri.g;
                    let mut n =
                        (tri.n[0] * (1.0 - u - v) + tri.n[1] * u + tri.n[2] * v).normalize();
                    let p = o + d * t;
                    if self.look.layer_lines > 0.0 && n.z.abs() < 0.97 {
                        // Ripple the normal up and down along the side: printed layers.
                        let up = (Vec3::Z - n * n.z).normalize();
                        let phase =
                            (p.z - self.floor) / self.look.layer_lines * std::f32::consts::TAU;
                        n = (n + up * (0.35 * phase.sin())).normalize();
                    }
                    let albedo = (self.paint.as_ref())
                        .and_then(|paint| paint.color(tri.id))
                        .unwrap_or(self.look.color);
                    (p, n, gn, albedo, self.look.roughness, 0.05)
                }
                (None, _) if floor_t < f32::MAX => {
                    let p = o + d * floor_t;
                    // Far floor fades into the backdrop.
                    let dist = (p - self.center).truncate().length() / self.radius;
                    let fade = ((dist - 4.0) / 6.0).clamp(0.0, 1.0);
                    if fade > 0.0 {
                        l += thr * fade * self.env(Vec3::new(d.x, d.y, 0.02).normalize(), false);
                        thr *= 1.0 - fade;
                        if thr.max_element() < 1e-3 {
                            break;
                        }
                    }
                    (p, Vec3::Z, Vec3::Z, self.floor_albedo(), 0.6, 0.02)
                }
                (None, _) => {
                    l += thr * self.env(d, specular);
                    break;
                }
            };
            // Face the ray.
            let (gn, flip) = if gn.dot(d) > 0.0 {
                (-gn, true)
            } else {
                (gn, false)
            };
            if flip {
                n = -n;
            }
            let p = p + gn * EPS * (1.0 + p.abs().max_element() * 1e-4);

            // Direct key light.
            let ld = sample_cone(self.key_dir, self.key_cos, rng);
            let cos_l = n.dot(ld);
            if cos_l > 0.0 && gn.dot(ld) > 0.0 && !self.occluded(p, ld) {
                let omega = 2.0 * std::f32::consts::PI * (1.0 - self.key_cos);
                l += thr
                    * albedo
                    * (1.0 / std::f32::consts::PI)
                    * self.key_radiance
                    * cos_l
                    * omega
                    * (1.0 - spec);
            }

            // Next direction: glossy coat or diffuse base.
            let cos_v = (-d).dot(n).clamp(0.0, 1.0);
            let fresnel = spec + (1.0 - spec) * (1.0 - cos_v).powi(5);
            if rng.f() < fresnel {
                let r = d.reflect(n);
                let exp = 2.0 / (rough * rough * rough + 1e-3);
                d = sample_lobe(r, exp, rng);
                if d.dot(gn) <= 0.0 {
                    break;
                }
                specular = rough < 0.4;
            } else {
                d = sample_cosine(n, rng);
                if d.dot(gn) <= 0.0 {
                    d = sample_cosine(gn, rng);
                }
                thr *= albedo;
                specular = false;
            }
            o = p;

            if bounce >= 2 {
                let q = thr.max_element().min(0.95);
                if rng.f() > q {
                    break;
                }
                thr /= q;
            }
        }
        l
    }

    /// One sample for every pixel, rows spread over all cores.
    fn pass(
        &self,
        cam: &Camera,
        w: usize,
        h: usize,
        pass: u32,
        accum: &mut [Vec3],
        cancel: &AtomicBool,
    ) {
        let fwd = (cam.target - cam.eye).normalize();
        let right = fwd.cross(Vec3::Z).normalize_or(Vec3::X);
        let up = right.cross(fwd);
        let th = (cam.fov * 0.5).tan();
        let aspect = w as f32 / h as f32;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let rows_per = h.div_ceil(threads * 4).max(1);
        let chunks: Vec<(usize, &mut [Vec3])> = accum
            .chunks_mut(rows_per * w)
            .enumerate()
            .map(|(i, c)| (i * rows_per, c))
            .collect();
        let queue = Mutex::new(chunks);
        std::thread::scope(|s| {
            for _ in 0..threads {
                s.spawn(|| {
                    loop {
                        let Some((y0, rows)) = queue.lock().unwrap().pop() else {
                            break;
                        };
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        for (k, px) in rows.iter_mut().enumerate() {
                            let (x, y) = (k % w, y0 + k / w);
                            let mut rng = Rng::new((y * w + x) as u64, pass as u64);
                            let sx = ((x as f32 + rng.f()) / w as f32 * 2.0 - 1.0) * th * aspect;
                            let sy = (1.0 - (y as f32 + rng.f()) / h as f32 * 2.0) * th;
                            let d = (fwd + right * sx + up * sy).normalize();
                            let c = self.trace(cam.eye, d, &mut rng);
                            // Clamp fireflies.
                            *px += c.min(Vec3::splat(20.0));
                        }
                    }
                });
            }
        });
    }
}

/// Builds the BVH node at `slot` for triangles `start..end`. The two
/// children of an inner node sit next to each other.
fn build(tris: &mut [Tri], nodes: &mut Vec<Node>, slot: usize, start: usize, end: usize) {
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let (mut cmin, mut cmax) = (min, max);
    for t in &tris[start..end] {
        for p in [t.v0, t.v0 + t.e1, t.v0 + t.e2] {
            min = min.min(p);
            max = max.max(p);
        }
        let c = t.v0 + (t.e1 + t.e2) / 3.0;
        cmin = cmin.min(c);
        cmax = cmax.max(c);
    }
    nodes[slot] = Node {
        min,
        max,
        start: start as u32,
        count: (end - start) as u32,
    };
    if end - start <= 4 {
        return;
    }
    let ext = cmax - cmin;
    let axis = if ext.x >= ext.y && ext.x >= ext.z {
        0
    } else if ext.y >= ext.z {
        1
    } else {
        2
    };
    let mid = (start + end) / 2;
    let key = |t: &Tri| (t.v0 + (t.e1 + t.e2) / 3.0)[axis];
    tris[start..end].select_nth_unstable_by(mid - start, |a, b| key(a).total_cmp(&key(b)));
    let left = nodes.len();
    nodes.push(nodes[slot]);
    nodes.push(nodes[slot]);
    nodes[slot].start = left as u32;
    nodes[slot].count = 0;
    build(tris, nodes, left, start, mid);
    build(tris, nodes, left + 1, mid, end);
}

/// Face normals. Sliver triangles (almost no area for their length) get
/// the normal of their best-shaped neighbour: their own is numerical noise.
fn robust_normals(mesh: &Mesh) -> Vec<Vec3> {
    let n = mesh.triangles.len();
    let quality = |i: usize| {
        let [a, b, c] = mesh.tri(i);
        let longest = (b - a)
            .length_squared()
            .max((c - b).length_squared())
            .max((a - c).length_squared());
        (b - a).cross(c - a).length() / longest.max(1e-20)
    };
    let q: Vec<f32> = (0..n).map(quality).collect();
    let mut normals: Vec<Vec3> = (0..n).map(|i| mesh.normal(i)).collect();
    let mut edge_face = std::collections::HashMap::with_capacity(n * 3);
    for (i, t) in mesh.triangles.iter().enumerate() {
        for k in 0..3 {
            edge_face.insert((t[k], t[(k + 1) % 3]), i);
        }
    }
    const SLIVER: f32 = 1e-3;
    // A few rounds, so chains of slivers take the normal from a good face.
    let mut good: Vec<bool> = q.iter().map(|&x| x >= SLIVER).collect();
    for _ in 0..8 {
        let mut changed = false;
        for i in 0..n {
            if good[i] {
                continue;
            }
            let t = mesh.triangles[i];
            // The neighbour that agrees most with the sliver's own (noisy) normal.
            let own = normals[i];
            let best = (0..3)
                .filter_map(|k| edge_face.get(&(t[(k + 1) % 3], t[k])).copied())
                .filter(|&j| good[j])
                .max_by(|&a, &b| own.dot(normals[a]).total_cmp(&own.dot(normals[b])));
            if let Some(j) = best.filter(|&j| own == Vec3::ZERO || own.dot(normals[j]) > 0.9) {
                normals[i] = normals[j];
                good[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    normals
}

/// Faces of large planar areas. They keep their face normal; only the narrow
/// facets of curved surfaces ($fn steps) get smooth normals.
fn flat_faces(mesh: &Mesh, face_n: &[Vec3]) -> Vec<bool> {
    let n = mesh.triangles.len();
    let mut edge_face = std::collections::HashMap::with_capacity(n * 3);
    for (i, t) in mesh.triangles.iter().enumerate() {
        for k in 0..3 {
            edge_face.insert((t[k], t[(k + 1) % 3]), i);
        }
    }
    let area = |i: usize| {
        let [a, b, c] = mesh.tri(i);
        (b - a).cross(c - a).length() * 0.5
    };
    let total: f32 = (0..n).map(area).sum();
    let mut group = vec![usize::MAX; n];
    let mut flat = vec![false; n];
    for seed in 0..n {
        if group[seed] != usize::MAX {
            continue;
        }
        group[seed] = seed;
        let mut members = vec![seed];
        let mut k = 0;
        while k < members.len() {
            let i = members[k];
            k += 1;
            let t = mesh.triangles[i];
            for e in 0..3 {
                if let Some(&j) = edge_face.get(&(t[(e + 1) % 3], t[e]))
                    && group[j] == usize::MAX
                    && face_n[j].dot(face_n[seed]) > 0.99999
                {
                    group[j] = seed;
                    members.push(j);
                }
            }
        }
        let a: f32 = members.iter().map(|&i| area(i)).sum();
        if members.len() > 2 || a > total * 0.02 {
            for i in members {
                flat[i] = true;
            }
        }
    }
    flat
}

fn slab(min: Vec3, max: Vec3, o: Vec3, inv: Vec3, t_max: f32) -> bool {
    let t0 = (min - o) * inv;
    let t1 = (max - o) * inv;
    let tmin = t0.min(t1).max_element().max(0.0);
    let tmax = t0.max(t1).min_element().min(t_max);
    tmin <= tmax
}

fn intersect(t: &Tri, o: Vec3, d: Vec3) -> Option<(f32, f32, f32)> {
    let p = d.cross(t.e2);
    let det = t.e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - t.v0;
    // A small tolerance closes the hairline gaps between long thin
    // neighbour triangles, where rays would slip through.
    const TOL: f32 = 1e-5;
    let u = s.dot(p) * inv;
    if !(-TOL..=1.0 + TOL).contains(&u) {
        return None;
    }
    let q = s.cross(t.e1);
    let v = d.dot(q) * inv;
    if v < -TOL || u + v > 1.0 + TOL {
        return None;
    }
    // Distance from the face plane: exact also for slivers, where the
    // barycentric distance drifts and puts the hit under the surface.
    let gd = t.g.dot(d);
    let tt = if gd.abs() > 1e-4 {
        t.g.dot(t.v0 - o) / gd
    } else {
        t.e2.dot(q) * inv
    };
    (tt > EPS).then_some((tt, u, v))
}

// ------------------------------------------------------------- sampling ----

struct Rng(u64);

impl Rng {
    fn new(pixel: u64, pass: u64) -> Rng {
        let mut r = Rng(pixel.wrapping_mul(0x9E3779B97F4A7C15)
            ^ pass.wrapping_mul(0xD1B54A32D192ED03)
            ^ 0x2545F4914F6CDD1D);
        r.next();
        r
    }
    fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn basis(n: Vec3) -> (Vec3, Vec3) {
    let a = n.any_orthonormal_vector();
    (a, n.cross(a))
}

fn sample_cosine(n: Vec3, rng: &mut Rng) -> Vec3 {
    let (u1, u2) = (rng.f(), rng.f());
    let r = u1.sqrt();
    let phi = std::f32::consts::TAU * u2;
    let (t, b) = basis(n);
    (t * (r * phi.cos()) + b * (r * phi.sin()) + n * (1.0 - u1).max(0.0).sqrt()).normalize()
}

/// Phong-style lobe around `dir`.
fn sample_lobe(dir: Vec3, exp: f32, rng: &mut Rng) -> Vec3 {
    let (u1, u2) = (rng.f(), rng.f());
    let cos_t = u1.powf(1.0 / (exp + 1.0));
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let phi = std::f32::consts::TAU * u2;
    let (t, b) = basis(dir);
    (t * (sin_t * phi.cos()) + b * (sin_t * phi.sin()) + dir * cos_t).normalize()
}

fn sample_cone(dir: Vec3, cos_max: f32, rng: &mut Rng) -> Vec3 {
    let (u1, u2) = (rng.f(), rng.f());
    let cos_t = 1.0 - u1 * (1.0 - cos_max);
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let phi = std::f32::consts::TAU * u2;
    let (t, b) = basis(dir);
    t * (sin_t * phi.cos()) + b * (sin_t * phi.sin()) + dir * cos_t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::cube;

    #[test]
    fn bvh_hits_like_brute_force() {
        let m = cube(10.0);
        let look = Look {
            color: Vec3::ONE,
            roughness: 0.5,
            layer_lines: 0.0,
            studio: Studio::Dark,
            exposure: 1.0,
        };
        let s = Scene::new(&m, look);
        let mut rng = Rng::new(1, 2);
        for _ in 0..500 {
            let o = Vec3::new(
                rng.f() * 40.0 - 15.0,
                rng.f() * 40.0 - 15.0,
                rng.f() * 40.0 - 15.0,
            );
            let d = (Vec3::splat(5.0) + Vec3::new(rng.f(), rng.f(), rng.f()) * 8.0 - 4.0 - o)
                .normalize();
            let fast = s.hit_mesh(o, d, f32::MAX).map(|h| h.0);
            let brute = s
                .tris
                .iter()
                .filter_map(|t| intersect(t, o, d).map(|h| h.0))
                .min_by(f32::total_cmp);
            match (fast, brute) {
                (Some(a), Some(b)) => assert!((a - b).abs() < 1e-3),
                (None, None) => {}
                other => panic!("mismatch {other:?}"),
            }
        }
    }

    #[test]
    fn renders_a_frame() {
        let m = Arc::new(cube(10.0));
        let look = Look {
            color: srgb(0xff9300),
            roughness: 0.4,
            layer_lines: 0.2,
            studio: Studio::Dark,
            exposure: 1.0,
        };
        let cam = Camera {
            eye: Vec3::new(30.0, -30.0, 25.0),
            target: Vec3::splat(5.0),
            fov: 0.6,
        };
        let job = Job::start(m, cam, look, 64, 48, 4);
        for _ in 0..200 {
            if job.frame.lock().unwrap().samples == 4 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let f = job.frame.lock().unwrap();
        assert_eq!(f.samples, 4);
        assert_eq!(f.rgba.len(), 64 * 48 * 4);
        // The centre pixel sees the orange cube: red > blue.
        let c = (24 * 64 + 32) * 4;
        assert!(f.rgba[c] > f.rgba[c + 2], "{:?}", &f.rgba[c..c + 4]);
    }

    #[test]
    fn paint_replaces_the_look_color() {
        let m = Arc::new(cube(10.0));
        let look = Look {
            color: srgb(0xff9300),
            roughness: 0.9,
            layer_lines: 0.0,
            studio: Studio::Dark,
            exposure: 1.0,
        };
        let cam = Camera {
            eye: Vec3::new(30.0, -30.0, 25.0),
            target: Vec3::splat(5.0),
            fov: 0.6,
        };
        let paint = Paint::new([(12, linear([0x20, 0x40, 0xff]))]);
        let model = Model {
            mesh: m,
            paint: Some(Arc::new(paint)),
            clip: None,
        };
        let job = Job::start(model, cam, look, 64, 48, 4);
        for _ in 0..200 {
            if job.frame.lock().unwrap().samples == 4 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let f = job.frame.lock().unwrap();
        assert_eq!(f.samples, 4);
        // The centre pixel sees the blue cube: blue > red.
        let c = (24 * 64 + 32) * 4;
        assert!(f.rgba[c + 2] > f.rgba[c], "{:?}", &f.rgba[c..c + 4]);
    }

    #[test]
    fn clip_removes_one_side_and_shows_a_cut_face() {
        // A 10 mm cube, cut at x = 5, the +X side stays.
        let m = cube(10.0);
        let look = Look {
            color: Vec3::ONE,
            roughness: 0.5,
            layer_lines: 0.0,
            studio: Studio::Dark,
            exposure: 1.0,
        };
        let mut s = Scene::new(&m, look);
        let plane = Vec4::new(1.0, 0.0, 0.0, -5.0);
        s.clip = Some(Arc::new(Clip::new(vec![plane], [])));
        // From -X: the face at x = 0 is cut away. The ray goes into the
        // solid at x = 5 and leaves it at x = 10.
        let (o, d) = (Vec3::new(-20.0, 5.0, 5.0), Vec3::X);
        let (t, i, _, _) = s.hit_mesh(o, d, f32::MAX).unwrap();
        assert!((t - 30.0).abs() < 1e-3);
        let (tc, n) = s.cap(o, d, t, i).unwrap();
        assert!((tc - 25.0).abs() < 1e-3);
        assert_eq!(n, -Vec3::X);
        // From +X: the face at x = 10 stays, no cut face.
        let (o, d) = (Vec3::new(40.0, 5.0, 5.0), -Vec3::X);
        let (t, i, _, _) = s.hit_mesh(o, d, f32::MAX).unwrap();
        assert!((t - 30.0).abs() < 1e-3);
        assert!(s.cap(o, d, t, i).is_none());
        // Past the cube: no hit.
        assert!(
            s.hit_mesh(Vec3::new(-20.0, 20.0, 5.0), d, f32::MAX)
                .is_none()
        );
        // A part offset moves the cut with the part: with an offset of
        // +10 mm in X, the cut is at x = 15.
        let moved = Clip::new(vec![plane], [(12, Vec3::new(10.0, 0.0, 0.0))]);
        assert!(!moved.keeps(Vec3::new(14.0, 5.0, 5.0), 0));
        assert!(moved.keeps(Vec3::new(16.0, 5.0, 5.0), 0));
        // Triangles after the last part have no offset.
        assert!(moved.keeps(Vec3::new(6.0, 5.0, 5.0), 12));
    }

    #[test]
    fn touching_parts_keep_their_cut_face() {
        // Cube A on top of cube B: the bottom of A and the top of B are at
        // z = 0. The cut at z = 5 keeps the side below.
        let a = cube(10.0);
        let mut m = a.clone();
        let n = m.vertices.len() as u32;
        m.vertices
            .extend(a.vertices.iter().map(|v| *v - Vec3::new(0.0, 0.0, 10.0)));
        m.triangles
            .extend(a.triangles.iter().map(|t| t.map(|i| i + n)));
        let look = Look {
            color: Vec3::ONE,
            roughness: 0.5,
            layer_lines: 0.0,
            studio: Studio::Dark,
            exposure: 1.0,
        };
        let mut s = Scene::new(&m, look);
        let plane = Vec4::new(0.0, 0.0, -1.0, 5.0);
        s.clip = Some(Arc::new(Clip::new(vec![plane], [])));
        // From above: into A through the cut face at z = 5.
        let (o, d) = (Vec3::new(5.0, 5.0, 20.0), -Vec3::Z);
        let (t, i, _, _) = s.hit_mesh(o, d, f32::MAX).unwrap();
        assert!((t - 20.0).abs() < 1e-3);
        let (tc, n) = s.cap(o, d, t, i).unwrap();
        assert!((tc - 15.0).abs() < 1e-3);
        assert_eq!(n, Vec3::Z);
    }

    #[test]
    fn geometry_is_reused_for_the_same_mesh() {
        let a = Arc::new(cube(10.0));
        let b = Arc::new(cube(10.0));
        let slot: Slot = Mutex::new(None);
        let g1 = Geometry::cached_in(&slot, &a);
        assert!(Arc::ptr_eq(&g1, &Geometry::cached_in(&slot, &a)));
        let g3 = Geometry::cached_in(&slot, &b);
        assert!(!Arc::ptr_eq(&g1, &g3));
        assert!(!Arc::ptr_eq(&g1, &Geometry::cached_in(&slot, &a)));
    }
}
