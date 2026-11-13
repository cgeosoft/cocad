//! GPU path tracer for the Render tab and the MCP photos. A fragment shader
//! (`trace.glsl`) traces the same scene, material and light as the CPU
//! tracer, on the same BVH (`Geometry`). The triangles and the BVH nodes go
//! to the GPU in integer textures.
//!
//! The app calls `Tracer::step` once per frame on the GL thread. A step
//! draws some tiles of a pass, about `BUDGET_NS` of GPU time, so the window
//! stays smooth and no draw call is long. A watchdog thread for each job
//! moves the job to the CPU when the GPU makes no progress or fails.

use super::{Camera, Frame, Geometry, Look, Model, Node, Scene, Tri, run_cpu, studio};
use crate::render::MAX_PLANES;
use eframe::glow::{self, HasContext};
use glam::{Vec2, Vec3, Vec4};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

/// Texels in a row of the triangle and node textures (`TEX_W` in the shader).
const TEX_W: usize = 4096;
/// RGBA32UI texels per triangle and per BVH node.
const TRI_TEXELS: usize = 4;
const NODE_TEXELS: usize = 2;
/// Side of a tile in pixels. One draw call renders one tile.
const TILE: usize = 128;
/// GPU time for the tiles of one frame, in nanoseconds.
const BUDGET_NS: f64 = 12e6;
/// A GPU job with no new tiles for this long goes on with the CPU.
const STALL: Duration = Duration::from_secs(3);
/// Time between two images of a running render.
const READ_EVERY: Duration = Duration::from_millis(250);
/// The largest accumulation buffer (RGBA32F).
const MAX_ACCUM_BYTES: usize = 1 << 30;

// The states of a job.
const RUN: u8 = 0;
const DONE: u8 = 1;
const FAILED: u8 = 2;
const CPU: u8 = 3;

const VS: &str = "
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

/// Exposure, ACES filmic fit and sRGB, as `super::tonemap`. Writes the
/// image top row first, so that `read_pixels` gives the rows in image order.
const TONE_FS: &str = "
uniform sampler2D u_accum;
uniform float u_scale;
uniform int u_h;
out vec4 o_color;

float srgb(float v) {
    return v <= 0.0031308 ? v * 12.92 : 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

void main() {
    ivec2 px = ivec2(gl_FragCoord.xy);
    vec3 x = texelFetch(u_accum, ivec2(px.x, u_h - 1 - px.y), 0).rgb * u_scale;
    vec3 m = clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0);
    o_color = vec4(srgb(m.r), srgb(m.g), srgb(m.b), 1.0);
}
";

// ------------------------------------------------------------- packing ----

/// The texture data of a `Geometry`: RGBA32UI texels in rows of `TEX_W`.
pub(super) struct Packed {
    tris: Vec<u32>,
    tri_rows: usize,
    nodes: Vec<u32>,
    node_rows: usize,
}

impl Packed {
    fn new(geo: &Geometry) -> Packed {
        let tri_rows = (geo.tris.len() * TRI_TEXELS).div_ceil(TEX_W).max(1);
        let mut tris = vec![0u32; tri_rows * TEX_W * 4];
        let (texels, _) = tris.as_chunks_mut::<{ TRI_TEXELS * 4 }>();
        for (t, out) in geo.tris.iter().zip(texels) {
            *out = pack_tri(t);
        }
        let node_rows = (geo.nodes.len() * NODE_TEXELS).div_ceil(TEX_W).max(1);
        let mut nodes = vec![0u32; node_rows * TEX_W * 4];
        let (texels, _) = nodes.as_chunks_mut::<{ NODE_TEXELS * 4 }>();
        for (n, out) in geo.nodes.iter().zip(texels) {
            *out = pack_node(n);
        }
        Packed {
            tris,
            tri_rows,
            nodes,
            node_rows,
        }
    }
}

/// `(v0, id)`, `(e1, g.x)`, `(e2, g.y)`, `(n0, n1, n2, g.z)`. The shading
/// normals are octahedral (`oct`).
fn pack_tri(t: &Tri) -> [u32; TRI_TEXELS * 4] {
    let b = f32::to_bits;
    [
        b(t.v0.x),
        b(t.v0.y),
        b(t.v0.z),
        t.id,
        b(t.e1.x),
        b(t.e1.y),
        b(t.e1.z),
        b(t.g.x),
        b(t.e2.x),
        b(t.e2.y),
        b(t.e2.z),
        b(t.g.y),
        oct(t.n[0]),
        oct(t.n[1]),
        oct(t.n[2]),
        b(t.g.z),
    ]
}

/// `(min, start)`, `(max, count)`.
fn pack_node(n: &Node) -> [u32; NODE_TEXELS * 4] {
    let b = f32::to_bits;
    [
        b(n.min.x),
        b(n.min.y),
        b(n.min.z),
        n.start,
        b(n.max.x),
        b(n.max.y),
        b(n.max.z),
        n.count,
    ]
}

fn sign(v: f32) -> f32 {
    if v >= 0.0 { 1.0 } else { -1.0 }
}

/// A unit vector in two signed 16 bit values (octahedral map). The shader
/// decodes it with `unoct`.
fn oct(n: Vec3) -> u32 {
    let s = n.x.abs() + n.y.abs() + n.z.abs();
    if s.is_nan() || s <= 0.0 {
        return 0;
    }
    let n = n / s;
    let e = if n.z < 0.0 {
        (Vec2::ONE - Vec2::new(n.y.abs(), n.x.abs())) * Vec2::new(sign(n.x), sign(n.y))
    } else {
        Vec2::new(n.x, n.y)
    };
    let q = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16 as u16 as u32;
    q(e.x) | (q(e.y) << 16)
}

/// Range textures: one RGBA32UI texel per range, `(end, x, y, z)`.
fn pack_ranges(ends: &[u32], values: &[Vec3]) -> Vec<u32> {
    let b = f32::to_bits;
    (ends.iter().zip(values))
        .flat_map(|(e, v)| [*e, b(v.x), b(v.y), b(v.z)])
        .collect()
}

// ---------------------------------------------------------------- jobs ----

/// The state of one GPU job, shared by the `Job`, its watchdog thread and
/// the tracer on the GL thread.
pub struct Shared {
    model: Model,
    cam: Camera,
    look: Look,
    size: (usize, usize),
    samples: u32,
    frame: Arc<Mutex<Frame>>,
    cancel: Arc<AtomicBool>,
    /// `RUN`, `DONE`, `FAILED` or `CPU`. A change to `DONE` or `CPU` happens
    /// with the frame locked.
    state: AtomicU8,
    /// Counts the steps that drew tiles of the job.
    beat: AtomicU64,
    /// The scene, when the watchdog has built it.
    geo: OnceLock<Arc<Geometry>>,
}

impl Shared {
    pub(super) fn new(
        model: Model,
        cam: Camera,
        look: Look,
        size: (usize, usize),
        samples: u32,
        frame: Arc<Mutex<Frame>>,
        cancel: Arc<AtomicBool>,
    ) -> Shared {
        Shared {
            model,
            cam,
            look,
            size,
            samples,
            frame,
            cancel,
            state: AtomicU8::new(RUN),
            beat: AtomicU64::new(0),
            geo: OnceLock::new(),
        }
    }

    /// The GPU renders the job, or has rendered it.
    pub(super) fn on_gpu(&self) -> bool {
        matches!(self.state.load(Ordering::Relaxed), RUN | DONE)
    }

    fn running(&self) -> bool {
        !self.cancel.load(Ordering::Relaxed) && self.state.load(Ordering::Relaxed) == RUN
    }

    /// The GPU cannot render the job. The watchdog moves it to the CPU.
    fn fail(&self) {
        let _ = (self.state).compare_exchange(RUN, FAILED, Ordering::Relaxed, Ordering::Relaxed);
    }
}

/// New GPU jobs, from any thread, for the tracer on the GL thread.
#[derive(Clone, Default)]
pub struct Queue(Arc<Mutex<Vec<Arc<Shared>>>>);

/// Queues a job and starts its watchdog.
pub(super) fn submit(queue: &Queue, shared: Shared) -> Arc<Shared> {
    let s = Arc::new(shared);
    queue.0.lock().unwrap().push(s.clone());
    let w = s.clone();
    std::thread::spawn(move || watch(&w));
    s
}

/// Builds the scene of the job, then watches the GPU. When the GPU fails
/// or makes no progress for `STALL`, the job goes on with the CPU.
fn watch(s: &Shared) {
    let geo = Geometry::cached(&s.model.mesh);
    geo.packed.get_or_init(|| Packed::new(&geo));
    let _ = s.geo.set(geo);
    let mut beat = s.beat.load(Ordering::Relaxed);
    let mut since = Instant::now();
    loop {
        if s.cancel.load(Ordering::Relaxed) {
            return;
        }
        match s.state.load(Ordering::Relaxed) {
            DONE | CPU => return,
            FAILED => break,
            _ => {}
        }
        let b = s.beat.load(Ordering::Relaxed);
        if b != beat {
            (beat, since) = (b, Instant::now());
        } else if since.elapsed() > STALL {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    {
        // The tracer writes the frame only in the `RUN` state, with the
        // frame locked. After this block it does not write it again.
        let _f = s.frame.lock().unwrap();
        if s.state.load(Ordering::Relaxed) == DONE {
            return;
        }
        s.state.store(CPU, Ordering::Relaxed);
    }
    run_cpu(
        &s.model, s.cam, s.look, s.size, s.samples, &s.frame, &s.cancel,
    );
}

// -------------------------------------------------------------- tracer ----

/// The uploaded triangles and BVH of a geometry.
struct SceneTex {
    geo: Weak<Geometry>,
    tris: glow::Texture,
    nodes: glow::Texture,
}

/// A job on the GPU, with its buffers.
struct Active {
    s: Arc<Shared>,
    /// The scene, with the light of the look.
    scene: Scene,
    accum: (glow::Framebuffer, glow::Texture),
    out: (glow::Framebuffer, glow::Texture),
    paint: (glow::Texture, i32),
    clip: (glow::Texture, i32),
    planes: Vec<Vec4>,
    /// Tiles in x and y.
    tiles: (usize, usize),
    /// The pass in work and its next tile.
    pass: u32,
    tile: usize,
    /// Passes in the frame.
    shown: u32,
    last_read: Instant,
}

/// The GPU path tracer. It lives on the GL thread.
pub struct Tracer {
    queue: Queue,
    trace: glow::Program,
    tone: glow::Program,
    vao: glow::VertexArray,
    max_tex: usize,
    scenes: Vec<SceneTex>,
    /// Jobs that wait for their scene.
    waiting: Vec<Arc<Shared>>,
    /// Jobs on the GPU. The first one gets the next step.
    jobs: Vec<Active>,
    /// Timer queries in work, with their tile counts.
    queries: Vec<(glow::Query, usize)>,
    free_queries: Vec<glow::Query>,
    /// GPU time per tile, in nanoseconds (running mean).
    ns_per_tile: f64,
}

/// Software OpenGL. The CPU tracer is faster than a shader on it.
fn is_software(renderer: &str) -> bool {
    let r = renderer.to_ascii_lowercase();
    [
        "llvmpipe",
        "softpipe",
        "swiftshader",
        "microsoft basic render driver",
        "gdi generic",
        "apple software renderer",
    ]
    .iter()
    .any(|s| r.contains(s))
}

impl Tracer {
    /// Makes the tracer, or tells why this context cannot run it. The
    /// `COCAD_RENDER` variable (`gpu` or `cpu`) forces one tracer.
    pub fn new(gl: &glow::Context) -> Result<Tracer, String> {
        let force = std::env::var("COCAD_RENDER").unwrap_or_default();
        if force == "cpu" {
            return Err("COCAD_RENDER=cpu".into());
        }
        let v = gl.version();
        if v.is_embedded || (v.major, v.minor) < (3, 3) {
            return Err(format!(
                "needs desktop OpenGL 3.3, the context is {}{}.{}",
                if v.is_embedded { "ES " } else { "" },
                v.major,
                v.minor
            ));
        }
        unsafe {
            let renderer = gl.get_parameter_string(glow::RENDERER);
            if force != "gpu" && is_software(&renderer) {
                return Err(format!("software renderer ({renderer})"));
            }
            let version = "#version 330 core";
            let fs = format!(
                "#define MAX_PLANES {MAX_PLANES}\n{}",
                include_str!("trace.glsl")
            );
            let trace = crate::render::program(gl, version, VS, &fs)?;
            let tone = match crate::render::program(gl, version, VS, TONE_FS) {
                Ok(p) => p,
                Err(e) => {
                    gl.delete_program(trace);
                    return Err(e);
                }
            };
            gl.use_program(Some(trace));
            for (i, name) in ["u_tris", "u_nodes", "u_paint", "u_clip"].iter().enumerate() {
                gl.uniform_1_i32(gl.get_uniform_location(trace, name).as_ref(), i as i32);
            }
            gl.use_program(Some(tone));
            gl.uniform_1_i32(gl.get_uniform_location(tone, "u_accum").as_ref(), 0);
            gl.use_program(None);
            let vao = gl.create_vertex_array()?;
            Ok(Tracer {
                queue: Queue::default(),
                trace,
                tone,
                vao,
                max_tex: gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE).max(0) as usize,
                scenes: Vec::new(),
                waiting: Vec::new(),
                jobs: Vec::new(),
                queries: Vec::new(),
                free_queries: Vec::new(),
                ns_per_tile: BUDGET_NS / 4.0,
            })
        }
    }

    /// The queue for `Job::start_with`.
    pub fn queue(&self) -> Queue {
        self.queue.clone()
    }

    /// Renders the jobs for about one frame budget. Returns true while jobs
    /// wait or run: the app must then paint again. After an error, the app
    /// must call `shutdown`.
    pub fn step(&mut self, gl: &glow::Context) -> Result<bool, String> {
        let new: Vec<_> = self.queue.0.lock().unwrap().drain(..).collect();
        self.waiting.extend(new);
        self.waiting.retain(|s| s.running());
        unsafe {
            // Errors of other code are not ours.
            for _ in 0..8 {
                if gl.get_error() == glow::NO_ERROR {
                    break;
                }
            }
            let mut i = 0;
            while i < self.waiting.len() {
                let Some(geo) = self.waiting[i].geo.get().cloned() else {
                    i += 1;
                    continue;
                };
                let s = self.waiting.remove(i);
                match self.activate(gl, s.clone(), geo) {
                    Ok(a) => self.jobs.push(a),
                    Err(e) => {
                        eprintln!("gpu render: {e}, the CPU renders the image");
                        s.fail();
                    }
                }
            }
            let (keep, gone): (Vec<_>, Vec<_>) =
                std::mem::take(&mut self.jobs).into_iter().partition(|a| a.s.running());
            self.jobs = keep;
            for a in gone {
                a.destroy(gl);
            }
            self.collect_queries(gl);
            if !self.jobs.is_empty() && self.queries.len() < 2 {
                self.serve(gl)?;
                self.jobs.rotate_left(1);
            }
            self.trim_scenes(gl);
            let err = gl.get_error();
            if err != glow::NO_ERROR {
                return Err(format!("OpenGL error 0x{err:04x}"));
            }
        }
        Ok(!self.waiting.is_empty() || !self.jobs.is_empty())
    }

    /// Stops the tracer: its jobs go on with the CPU. Deletes the GL objects.
    pub fn shutdown(&mut self, gl: &glow::Context) {
        let new: Vec<_> = self.queue.0.lock().unwrap().drain(..).collect();
        for s in self.waiting.drain(..).chain(new) {
            s.fail();
        }
        unsafe {
            for a in self.jobs.drain(..) {
                a.s.fail();
                a.destroy(gl);
            }
            for t in self.scenes.drain(..) {
                gl.delete_texture(t.tris);
                gl.delete_texture(t.nodes);
            }
            for (q, _) in self.queries.drain(..) {
                gl.delete_query(q);
            }
            for q in self.free_queries.drain(..) {
                gl.delete_query(q);
            }
            gl.delete_program(self.trace);
            gl.delete_program(self.tone);
            gl.delete_vertex_array(self.vao);
        }
    }

    /// Makes the buffers of a job. An error leaves the job to the CPU.
    unsafe fn activate(
        &mut self,
        gl: &glow::Context,
        s: Arc<Shared>,
        geo: Arc<Geometry>,
    ) -> Result<Active, String> {
        let (w, h) = s.size;
        if w == 0 || h == 0 || s.samples == 0 {
            return Err("empty image".into());
        }
        if w > self.max_tex || h > self.max_tex || w * h * 16 > MAX_ACCUM_BYTES {
            return Err(format!("{w} × {h} is too large for the GPU"));
        }
        let packed = geo.packed.get_or_init(|| Packed::new(&geo));
        if packed.tri_rows > self.max_tex || packed.node_rows > self.max_tex {
            return Err("the mesh is too large for the GPU".into());
        }
        let planes: Vec<Vec4> = s.model.clip.as_ref().map_or(Vec::new(), |c| c.planes.clone());
        if planes.len() > MAX_PLANES {
            return Err(format!("{} section planes", planes.len()));
        }
        let paint = s.model.paint.as_ref().map_or(Vec::new(), |p| pack_ranges(&p.ends, &p.colors));
        let clip = match (&s.model.clip, planes.is_empty()) {
            (Some(c), false) => pack_ranges(&c.ends, &c.offsets),
            _ => Vec::new(),
        };
        if paint.len() / 4 > self.max_tex || clip.len() / 4 > self.max_tex {
            return Err("too many parts".into());
        }
        unsafe {
            self.upload_scene(gl, &geo, packed)?;
            let accum = render_target(gl, glow::RGBA32F, glow::RGBA, glow::FLOAT, w, h)?;
            let out = match render_target(gl, glow::RGBA8, glow::RGBA, glow::UNSIGNED_BYTE, w, h) {
                Ok(o) => o,
                Err(e) => {
                    gl.delete_framebuffer(accum.0);
                    gl.delete_texture(accum.1);
                    return Err(e);
                }
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(accum.0));
            gl.clear_buffer_f32_slice(glow::COLOR, 0, &[0.0; 4]);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            let ranges = |data: &[u32]| -> Result<(glow::Texture, i32), String> {
                let n = data.len() / 4;
                let one = [0u32; 4];
                let (data, width) = if n == 0 { (&one[..], 1) } else { (data, n) };
                Ok((uint_texture(gl, data, width, 1)?, n as i32))
            };
            let paint = ranges(&paint)?;
            let clip = ranges(&clip)?;
            let mut scene = Scene::with(geo, s.look);
            scene.paint = s.model.paint.clone();
            Ok(Active {
                tiles: (w.div_ceil(TILE), h.div_ceil(TILE)),
                s,
                scene,
                accum,
                out,
                paint,
                clip,
                planes,
                pass: 0,
                tile: 0,
                shown: 0,
                last_read: Instant::now(),
            })
        }
    }

    /// Uploads the triangles and the BVH of `geo`, when no scene has them.
    unsafe fn upload_scene(
        &mut self,
        gl: &glow::Context,
        geo: &Arc<Geometry>,
        packed: &Packed,
    ) -> Result<(), String> {
        let have = self.scenes.iter().any(|t| {
            t.geo.upgrade().is_some_and(|g| Arc::ptr_eq(&g, geo))
        });
        if have {
            return Ok(());
        }
        unsafe {
            let tris = uint_texture(gl, &packed.tris, TEX_W, packed.tri_rows)?;
            let nodes = match uint_texture(gl, &packed.nodes, TEX_W, packed.node_rows) {
                Ok(n) => n,
                Err(e) => {
                    gl.delete_texture(tris);
                    return Err(e);
                }
            };
            self.scenes.push(SceneTex {
                geo: Arc::downgrade(geo),
                tris,
                nodes,
            });
        }
        Ok(())
    }

    /// Deletes the scenes that no job uses, except the newest one.
    unsafe fn trim_scenes(&mut self, gl: &glow::Context) {
        let last = self.scenes.len().saturating_sub(1);
        let mut k = 0;
        self.scenes.retain(|t| {
            let i = k;
            k += 1;
            let geo = t.geo.upgrade();
            let used = geo.as_ref().is_some_and(|g| {
                self.jobs.iter().any(|a| Arc::ptr_eq(&a.scene.geo, g))
            });
            let keep = used || (i == last && geo.is_some());
            if !keep {
                unsafe {
                    gl.delete_texture(t.tris);
                    gl.delete_texture(t.nodes);
                }
            }
            keep
        });
    }

    /// Reads the finished timer queries and updates the time per tile.
    unsafe fn collect_queries(&mut self, gl: &glow::Context) {
        let mut i = 0;
        while i < self.queries.len() {
            let (q, n) = self.queries[i];
            unsafe {
                if gl.get_query_parameter_u32(q, glow::QUERY_RESULT_AVAILABLE) == 0 {
                    i += 1;
                    continue;
                }
                let ns = gl.get_query_parameter_u32(q, glow::QUERY_RESULT) as f64;
                if n > 0 {
                    let per = (ns / n as f64).max(1e3);
                    self.ns_per_tile = 0.7 * self.ns_per_tile + 0.3 * per;
                }
            }
            self.queries.swap_remove(i);
            self.free_queries.push(q);
        }
    }

    /// Draws tiles of the first job for about one frame budget. At the end of
    /// a pass, it reads the image when one is due.
    unsafe fn serve(&mut self, gl: &glow::Context) -> Result<(), String> {
        let budget = (BUDGET_NS / self.ns_per_tile).clamp(1.0, 4096.0) as usize;
        let query = match self.free_queries.pop() {
            Some(q) => q,
            None => unsafe { gl.create_query()? },
        };
        let trace = self.trace;
        let vao = self.vao;
        let a = &mut self.jobs[0];
        let scene = self
            .scenes
            .iter()
            .find(|t| Weak::as_ptr(&t.geo) == Arc::as_ptr(&a.scene.geo))
            .ok_or("the scene of a job is missing")?;
        let (w, h) = a.s.size;
        let mut drawn = 0;
        let mut read = false;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(a.accum.0));
            gl.viewport(0, 0, w as i32, h as i32);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE);
            gl.enable(glow::SCISSOR_TEST);
            gl.use_program(Some(trace));
            gl.bind_vertex_array(Some(vao));
            for (unit, tex) in [scene.tris, scene.nodes, a.paint.0, a.clip.0].into_iter().enumerate() {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            }
            a.uniforms(gl, trace);
            gl.begin_query(glow::TIME_ELAPSED, query);
            while drawn < budget {
                let (tx, ty) = (a.tile % a.tiles.0, a.tile / a.tiles.0);
                gl.scissor((tx * TILE) as i32, (ty * TILE) as i32, TILE as i32, TILE as i32);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
                drawn += 1;
                a.tile += 1;
                if a.tile == a.tiles.0 * a.tiles.1 {
                    a.tile = 0;
                    a.pass += 1;
                    if a.pass == a.s.samples
                        || a.shown == 0
                        || a.last_read.elapsed() >= READ_EVERY
                    {
                        read = true;
                        break;
                    }
                    gl.uniform_1_u32(gl.get_uniform_location(trace, "u_pass").as_ref(), a.pass);
                }
            }
            gl.end_query(glow::TIME_ELAPSED);
            self.queries.push((query, drawn));
            a.s.beat.fetch_add(1, Ordering::Relaxed);
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            if read {
                let px = a.read(gl, self.tone);
                a.publish(px);
            }
            for unit in (0..4).rev() {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        Ok(())
    }
}

impl Active {
    /// Sets the uniforms of the trace program for this job.
    unsafe fn uniforms(&self, gl: &glow::Context, prog: glow::Program) {
        let s = &self.s;
        let (w, h) = s.size;
        let cam = &s.cam;
        let fwd = (cam.target - cam.eye).normalize();
        let right = fwd.cross(Vec3::Z).normalize_or(Vec3::X);
        let up = right.cross(fwd);
        let st = studio(s.look.studio);
        let sc = &self.scene;
        unsafe {
            let loc = |name: &str| gl.get_uniform_location(prog, name);
            let v3 = |name: &str, v: Vec3| gl.uniform_3_f32(loc(name).as_ref(), v.x, v.y, v.z);
            let f = |name: &str, v: f32| gl.uniform_1_f32(loc(name).as_ref(), v);
            let i = |name: &str, v: i32| gl.uniform_1_i32(loc(name).as_ref(), v);
            gl.uniform_2_i32(loc("u_size").as_ref(), w as i32, h as i32);
            gl.uniform_1_u32(loc("u_pass").as_ref(), self.pass);
            v3("u_eye", cam.eye);
            v3("u_fwd", fwd);
            v3("u_right", right);
            v3("u_up", up);
            f("u_th", (cam.fov * 0.5).tan());
            f("u_aspect", w as f32 / h as f32);
            v3("u_color", s.look.color);
            f("u_rough", s.look.roughness);
            f("u_layer", s.look.layer_lines);
            v3("u_horizon", st.horizon);
            v3("u_zenith", st.zenith);
            f("u_top", st.top);
            v3("u_floor_albedo", st.floor);
            f("u_floor", sc.floor);
            v3("u_center", sc.center);
            f("u_radius", sc.radius);
            v3("u_key_dir", sc.key_dir);
            f("u_key_cos", sc.key_cos);
            f("u_key_rad", sc.key_radiance);
            i("u_npaint", self.paint.1);
            i("u_nclip", self.clip.1);
            i("u_nplanes", self.planes.len() as i32);
            let mut planes = [0.0f32; 4 * MAX_PLANES];
            for (k, p) in self.planes.iter().enumerate() {
                planes[k * 4..k * 4 + 4].copy_from_slice(&p.to_array());
            }
            gl.uniform_4_f32_slice(loc("u_planes").as_ref(), &planes);
        }
    }

    /// Tone maps the accumulated passes and reads the image (sRGB RGBA,
    /// top row first).
    unsafe fn read(&self, gl: &glow::Context, tone: glow::Program) -> Vec<u8> {
        let (w, h) = self.s.size;
        let mut px = vec![0u8; w * h * 4];
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.out.0));
            gl.viewport(0, 0, w as i32, h as i32);
            gl.disable(glow::DITHER);
            gl.use_program(Some(tone));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.accum.1));
            let scale = self.s.look.exposure / self.pass as f32;
            gl.uniform_1_f32(gl.get_uniform_location(tone, "u_scale").as_ref(), scale);
            gl.uniform_1_i32(gl.get_uniform_location(tone, "u_h").as_ref(), h as i32);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.enable(glow::DITHER);
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
            gl.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut px)),
            );
        }
        px
    }

    /// Puts the image in the frame, unless the CPU has the job now.
    fn publish(&mut self, rgba: Vec<u8>) {
        let mut f = self.s.frame.lock().unwrap();
        if self.s.state.load(Ordering::Relaxed) != RUN {
            return;
        }
        if self.pass > f.samples {
            f.rgba = rgba;
            f.samples = self.pass;
        }
        if self.pass >= self.s.samples {
            self.s.state.store(DONE, Ordering::Relaxed);
        }
        self.shown = self.pass;
        self.last_read = Instant::now();
    }

    unsafe fn destroy(&self, gl: &glow::Context) {
        unsafe {
            for (fb, tex) in [self.accum, self.out] {
                gl.delete_framebuffer(fb);
                gl.delete_texture(tex);
            }
            gl.delete_texture(self.paint.0);
            gl.delete_texture(self.clip.0);
        }
    }
}

/// An RGBA32UI texture with `data` (`width * height` texels).
unsafe fn uint_texture(
    gl: &glow::Context,
    data: &[u32],
    width: usize,
    height: usize,
) -> Result<glow::Texture, String> {
    unsafe {
        let tex = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        nearest(gl);
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
        gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA32UI as i32,
            width as i32,
            height as i32,
            0,
            glow::RGBA_INTEGER,
            glow::UNSIGNED_INT,
            glow::PixelUnpackData::Slice(Some(bytemuck::cast_slice(data))),
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Ok(tex)
    }
}

/// A texture in a framebuffer, to render to.
unsafe fn render_target(
    gl: &glow::Context,
    internal: u32,
    format: u32,
    ty: u32,
    w: usize,
    h: usize,
) -> Result<(glow::Framebuffer, glow::Texture), String> {
    unsafe {
        let tex = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        nearest(gl);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            internal as i32,
            w as i32,
            h as i32,
            0,
            format,
            ty,
            glow::PixelUnpackData::Slice(None),
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        let fb = gl.create_framebuffer()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(tex),
            0,
        );
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        if status != glow::FRAMEBUFFER_COMPLETE {
            gl.delete_framebuffer(fb);
            gl.delete_texture(tex);
            return Err(format!("framebuffer not complete (0x{status:04x})"));
        }
        Ok((fb, tex))
    }
}

/// No filter and no mipmaps: `texelFetch` reads exact texels.
unsafe fn nearest(gl: &glow::Context) {
    unsafe {
        for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::NEAREST as i32);
        }
        for p in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::CLAMP_TO_EDGE as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::cube;

    /// The decode of `oct`, as `unoct` in the shader.
    fn unoct(p: u32) -> Vec3 {
        let x = (p as u16 as i16) as f32 / 32767.0;
        let y = ((p >> 16) as u16 as i16) as f32 / 32767.0;
        let mut n = Vec3::new(x, y, 1.0 - x.abs() - y.abs());
        if n.z < 0.0 {
            let (a, b) = (n.x, n.y);
            n.x = (1.0 - b.abs()) * sign(a);
            n.y = (1.0 - a.abs()) * sign(b);
        }
        n.normalize()
    }

    #[test]
    fn octahedral_normals_round_trip() {
        let mut dirs = vec![
            Vec3::X,
            -Vec3::X,
            Vec3::Y,
            -Vec3::Y,
            Vec3::Z,
            -Vec3::Z,
            Vec3::new(1.0, -2.0, -3.0).normalize(),
        ];
        // A spiral over the sphere.
        for k in 0..500 {
            let z = 1.0 - 2.0 * (k as f32 + 0.5) / 500.0;
            let r = (1.0 - z * z).sqrt();
            let a = k as f32 * 2.399_963;
            dirs.push(Vec3::new(r * a.cos(), r * a.sin(), z));
        }
        for d in dirs {
            let back = unoct(oct(d));
            assert!((back - d).length() < 1e-3, "{d} -> {back}");
        }
        assert_eq!(oct(Vec3::ZERO), 0);
    }

    #[test]
    fn packed_texels_decode_to_the_geometry() {
        let geo = Geometry::new(&cube(10.0));
        let p = Packed::new(&geo);
        assert_eq!(p.tris.len(), p.tri_rows * TEX_W * 4);
        assert_eq!(p.nodes.len(), p.node_rows * TEX_W * 4);
        let f = f32::from_bits;
        for (i, t) in geo.tris.iter().enumerate() {
            let x = &p.tris[i * 16..i * 16 + 16];
            assert_eq!(Vec3::new(f(x[0]), f(x[1]), f(x[2])), t.v0);
            assert_eq!(x[3], t.id);
            assert_eq!(Vec3::new(f(x[4]), f(x[5]), f(x[6])), t.e1);
            assert_eq!(Vec3::new(f(x[8]), f(x[9]), f(x[10])), t.e2);
            assert_eq!(Vec3::new(f(x[7]), f(x[11]), f(x[15])), t.g);
            for k in 0..3 {
                assert!((unoct(x[12 + k]) - t.n[k]).length() < 1e-3);
            }
        }
        for (i, n) in geo.nodes.iter().enumerate() {
            let x = &p.nodes[i * 8..i * 8 + 8];
            assert_eq!(Vec3::new(f(x[0]), f(x[1]), f(x[2])), n.min);
            assert_eq!(x[3], n.start);
            assert_eq!(Vec3::new(f(x[4]), f(x[5]), f(x[6])), n.max);
            assert_eq!(x[7], n.count);
        }
    }

    #[test]
    fn software_renderers_are_known() {
        assert!(is_software("llvmpipe (LLVM 19.1.7, 256 bits)"));
        assert!(is_software("Microsoft Basic Render Driver"));
        assert!(!is_software("NVIDIA GeForce RTX 3060/PCIe/SSE2"));
        assert!(!is_software("AMD Radeon Graphics (radeonsi, gfx1103)"));
    }

    /// A GPU job that no tracer serves goes on with the CPU after `STALL`.
    #[test]
    fn a_stalled_gpu_job_renders_on_the_cpu() {
        let look = Look {
            color: super::super::srgb(0xff9300),
            roughness: 0.4,
            layer_lines: 0.0,
            studio: super::super::Studio::Dark,
            exposure: 1.0,
        };
        let cam = Camera {
            eye: Vec3::new(30.0, -30.0, 25.0),
            target: Vec3::splat(5.0),
            fov: 0.6,
        };
        let queue = Queue::default();
        let job = super::super::Job::start_with(
            Some(&queue),
            Arc::new(cube(10.0)),
            cam,
            look,
            32,
            24,
            2,
        );
        assert_eq!(job.backend(), super::super::Backend::Gpu);
        let until = Instant::now() + STALL + Duration::from_secs(10);
        while job.frame.lock().unwrap().samples < 2 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(job.frame.lock().unwrap().samples, 2);
        assert_eq!(job.backend(), super::super::Backend::Cpu);
    }
}
