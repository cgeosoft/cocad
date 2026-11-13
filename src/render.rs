//! OpenGL renderer for the 3D view: shaded mesh, wireframe, bed grid and
//! slicer toolpaths. It runs inside an egui paint callback.
//!
//! The mesh is an `Assembly`: each part is a triangle range that is drawn
//! with its own `PartStyle` (color, offset, transparency, highlight).
//!
//! Section cut: the fragment shaders discard the removed side of each plane
//! in model space, before the part offset. The cut faces (caps) come from
//! the stencil buffer: for each plane, the clipped faces of the parts count
//! into the stencil (front faces +1, back faces -1). The plane then shows in
//! the part color where the count is not 0. In a cut, the back faces show
//! in a dark part color, not in purple.

use crate::assembly::Assembly;
use crate::slicer::{Kind, Sliced};
use eframe::glow::{self, HasContext};
use glam::{Mat4, Vec3, Vec4};
use std::ops::Range;

pub const ORANGE: [f32; 3] = [1.0, 0.576, 0.0]; // #ff9300

/// The color of back faces: inside-out or open geometry.
const PURPLE: [f32; 3] = [0.55, 0.18, 0.62];

/// The shaders take at most this number of section planes.
pub const MAX_PLANES: usize = 3;

/// A plane that keeps all points.
const NO_PLANE: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// The cut faces have the part color times this factor.
const CAP_SHADE: f32 = 0.8;

/// The color of the plane outlines.
const OUTLINE: [f32; 3] = [0.35, 0.72, 1.0];

/// How to draw one part of the assembly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartStyle {
    /// sRGB, 0 to 1.
    pub color: [f32; 3],
    /// Movement of the part, in mm.
    pub offset: Vec3,
    /// 1 is opaque. Transparent parts draw after the opaque parts. 0 hides
    /// the part.
    pub alpha: f32,
    /// Brighter, with an accent rim.
    pub highlight: bool,
}

impl Default for PartStyle {
    fn default() -> PartStyle {
        PartStyle {
            color: ORANGE,
            offset: Vec3::ZERO,
            alpha: 1.0,
            highlight: false,
        }
    }
}

/// The ranges of one part in the GPU buffers, in vertices.
struct GpuPart {
    tris: Range<i32>,
    edges: Range<i32>,
    center: Vec3,
    /// The bounds of the part, without offset.
    min: Vec3,
    max: Vec3,
}

struct Buffer {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    count: i32,
    /// Floats per vertex.
    stride: usize,
}

impl Buffer {
    /// Two vec3 attributes per vertex: position + normal or color.
    unsafe fn new(gl: &glow::Context) -> Buffer {
        unsafe { Buffer::with_attrs(gl, 2) }
    }

    /// `attrs` vec3 attributes per vertex, at locations 0, 1, ...
    unsafe fn with_attrs(gl: &glow::Context, attrs: u32) -> Buffer {
        unsafe {
            let vao = gl.create_vertex_array().unwrap();
            let vbo = gl.create_buffer().unwrap();
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = attrs as i32 * 12;
            for a in 0..attrs {
                gl.enable_vertex_attrib_array(a);
                gl.vertex_attrib_pointer_f32(a, 3, glow::FLOAT, false, stride, a as i32 * 12);
            }
            gl.bind_vertex_array(None);
            Buffer {
                vao,
                vbo,
                count: 0,
                stride: attrs as usize * 3,
            }
        }
    }

    unsafe fn upload(&mut self, gl: &glow::Context, data: &[f32]) {
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(data),
                glow::STATIC_DRAW,
            );
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
        self.count = (data.len() / self.stride) as i32;
    }

    unsafe fn draw(&self, gl: &glow::Context, mode: u32, first: i32, count: i32) {
        if count > 0 {
            unsafe {
                gl.bind_vertex_array(Some(self.vao));
                gl.draw_arrays(mode, first, count);
            }
        }
    }

    unsafe fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
        }
    }
}

const MESH_VS: &str = r#"
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_nrm;
uniform mat4 u_mvp;
uniform vec3 u_offset;
out vec3 v_nrm;
out vec3 v_pos;
out vec3 v_model;
void main() {
    v_nrm = a_nrm;
    v_model = a_pos;
    v_pos = a_pos + u_offset;
    gl_Position = u_mvp * vec4(v_pos, 1.0);
}
"#;

const MESH_FS: &str = r#"
precision highp float;
in vec3 v_nrm;
in vec3 v_pos;
in vec3 v_model;
uniform vec3 u_eye;
uniform vec3 u_color;
uniform vec3 u_back;
uniform float u_alpha;
uniform float u_highlight;
uniform vec4 u_clip[3];
out vec4 o_color;
void main() {
    for (int i = 0; i < 3; i++) {
        if (dot(u_clip[i].xyz, v_model) + u_clip[i].w < 0.0) discard;
    }
    vec3 n = normalize(v_nrm);
    vec3 base = u_color;
    if (!gl_FrontFacing) {
        // Back faces show inside-out or open geometry.
        n = -n;
        base = u_back;
    }
    vec3 v = normalize(u_eye - v_pos);
    vec3 key = normalize(vec3(0.45, 0.3, 1.0));
    vec3 fill = normalize(vec3(-0.6, -0.4, 0.2));
    float diff = 0.22 + 0.5 * max(dot(n, key), 0.0) + 0.15 * max(dot(n, fill), 0.0) + 0.25 * max(dot(n, v), 0.0);
    float spec = pow(max(dot(n, normalize(key + v)), 0.0), 48.0) * 0.3;
    float edge = 1.0 - max(dot(n, v), 0.0);
    float rim = pow(edge, 3.0) * 0.18;
    vec3 c = base * diff * (1.0 + 0.25 * u_highlight) + vec3(spec) + rim * vec3(1.0, 0.85, 0.7);
    // Highlight: an accent rim around the part.
    c += u_highlight * pow(edge, 2.0) * vec3(1.0, 0.576, 0.0) * 0.9;
    o_color = vec4(pow(c, vec3(0.95)), u_alpha);
}
"#;

const LINE_VS: &str = r#"
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_col;
uniform mat4 u_mvp;
uniform vec3 u_offset;
out vec3 v_col;
out vec3 v_model;
void main() {
    v_col = a_col;
    v_model = a_pos;
    gl_Position = u_mvp * vec4(a_pos + u_offset, 1.0);
}
"#;

const LINE_FS: &str = r#"
precision highp float;
in vec3 v_col;
in vec3 v_model;
uniform float u_tint;
uniform float u_alpha;
uniform vec4 u_clip[3];
out vec4 o_color;
void main() {
    for (int i = 0; i < 3; i++) {
        if (dot(u_clip[i].xyz, v_model) + u_clip[i].w < 0.0) discard;
    }
    o_color = vec4(v_col * u_tint, u_alpha);
}
"#;

const PATH_VS: &str = r#"
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_nrm;
layout(location = 2) in vec3 a_col;
uniform mat4 u_mvp;
out vec3 v_nrm;
out vec3 v_col;
out vec3 v_pos;
void main() {
    v_nrm = a_nrm;
    v_col = a_col;
    v_pos = a_pos;
    gl_Position = u_mvp * vec4(a_pos, 1.0);
}
"#;

const PATH_FS: &str = r#"
precision highp float;
in vec3 v_nrm;
in vec3 v_col;
in vec3 v_pos;
uniform vec3 u_eye;
uniform float u_tint;
out vec4 o_color;
void main() {
    vec3 n = normalize(v_nrm);
    vec3 v = normalize(u_eye - v_pos);
    vec3 key = normalize(vec3(0.45, 0.3, 1.0));
    float diff = 0.3 + 0.45 * max(dot(n, key), 0.0) + 0.3 * max(dot(n, v), 0.0);
    float spec = pow(max(dot(n, normalize(key + v)), 0.0), 32.0) * 0.18;
    o_color = vec4((v_col * diff + vec3(spec)) * u_tint, 1.0);
}
"#;

/// One vertex of an extruded toolpath: position, packed normal, color.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PathVertex {
    pos: [f32; 3],
    nrm: [i8; 4],
    col: [u8; 4],
}

/// Toolpaths as solid strands: a top face and two rounded sides per
/// segment, `line_width` wide and one layer high.
struct Extrusions {
    vertices: Vec<PathVertex>,
    indices: Vec<u32>,
    /// Index count at the end of each layer.
    layer_ends: Vec<i32>,
}

fn extrude(s: &Sliced) -> Extrusions {
    let mut out = Extrusions {
        vertices: Vec::new(),
        indices: Vec::new(),
        layer_ends: Vec::new(),
    };
    // Slightly narrower than the line width, so neighbour lines show a seam.
    let hw = s.line_width * 0.45;
    let pack = |n: Vec3| {
        let n = n.normalize_or_zero() * 127.0;
        [n.x as i8, n.y as i8, n.z as i8, 0]
    };
    // Rounded cross-section: (across, down, normal across, normal up).
    // `across` is in half widths from the centre line, `down` in layer heights.
    const PROFILE: [(f32, f32, f32, f32); 5] = [
        (1.0, 0.85, 1.0, -0.1),
        (0.8, 0.2, 0.75, 0.65),
        (0.0, 0.0, 0.0, 1.0),
        (-0.8, 0.2, -0.75, 0.65),
        (-1.0, 0.85, -1.0, -0.1),
    ];
    for layer in &s.layers {
        let top = layer.z;
        let h = layer.height;
        for p in &layer.paths {
            let c = kind_color(p.kind).map(|x| (x * 255.0) as u8);
            let col = [c[0], c[1], c[2], 255];
            let n = p.pts.len();
            let segs = if p.closed { n } else { n.saturating_sub(1) };
            for i in 0..segs {
                let (a, b) = (p.pts[i], p.pts[(i + 1) % n]);
                let d = (b - a).normalize_or_zero();
                if d == glam::Vec2::ZERO {
                    continue;
                }
                // Extend by half a width so corners join without gaps.
                let (a, b) = (a - d * hw, b + d * hw);
                let left = d.perp();
                let base = out.vertices.len() as u32;
                for end in [a, b] {
                    for &(across, down, na, nu) in &PROFILE {
                        let q = end + left * (across * hw);
                        let nrm = pack(Vec3::new(left.x * na, left.y * na, nu));
                        out.vertices.push(PathVertex {
                            pos: [q.x, q.y, top - down * h],
                            nrm,
                            col,
                        });
                    }
                }
                for k in 0..4u32 {
                    let (a0, a1, b0, b1) = (base + k, base + k + 1, base + 5 + k, base + 5 + k + 1);
                    out.indices.extend_from_slice(&[a0, b0, b1, a0, b1, a1]);
                }
            }
        }
        out.layer_ends.push(out.indices.len() as i32);
    }
    out
}

struct PathBuffer {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
}

impl PathBuffer {
    unsafe fn new(gl: &glow::Context) -> PathBuffer {
        unsafe {
            let vao = gl.create_vertex_array().unwrap();
            let vbo = gl.create_buffer().unwrap();
            let ebo = gl.create_buffer().unwrap();
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 20, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 3, glow::BYTE, true, 20, 12);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 3, glow::UNSIGNED_BYTE, true, 20, 16);
            gl.bind_vertex_array(None);
            PathBuffer { vao, vbo, ebo }
        }
    }

    unsafe fn upload(&self, gl: &glow::Context, e: &Extrusions) {
        unsafe {
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&e.vertices),
                glow::STATIC_DRAW,
            );
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.ebo));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                bytemuck::cast_slice(&e.indices),
                glow::STATIC_DRAW,
            );
            gl.bind_vertex_array(None);
        }
    }

    /// Draws indices `start..end`.
    unsafe fn draw(&self, gl: &glow::Context, start: i32, end: i32) {
        if end > start {
            unsafe {
                gl.bind_vertex_array(Some(self.vao));
                gl.draw_elements(glow::TRIANGLES, end - start, glow::UNSIGNED_INT, start * 4);
            }
        }
    }

    unsafe fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
            gl.delete_buffer(self.ebo);
        }
    }
}

/// What to draw in one frame.
pub struct View {
    pub mvp: Mat4,
    pub eye: Vec3,
    pub show_mesh: bool,
    /// Transparency of the whole mesh (the ghost in the slice view). It
    /// multiplies the alpha of each part.
    pub mesh_alpha: f32,
    /// The style of each part of the assembly, in part order. A missing
    /// entry uses `PartStyle::default()`.
    pub parts: Vec<PartStyle>,
    pub wireframe: bool,
    pub grid: bool,
    /// Slice preview: draw layers 0..=layer.
    pub layer: Option<usize>,
    pub section: Section,
}

/// The section cut of one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    /// The planes in model space, before the part offset. A point p of a
    /// part stays when `plane.truncate().dot(p) + plane.w >= 0.0` for all
    /// planes. The renderer uses the first `MAX_PLANES`.
    pub planes: Vec<Vec4>,
    /// Rectangles to show where the planes are, as 4 corners in view
    /// space (with the part offset).
    pub outlines: Vec<[Vec3; 4]>,
}

/// Where the box `min..max` is from plane `c`: `Less` when the plane
/// removes all of it, `Greater` when the plane keeps all of it, `Equal`
/// when the plane cuts it.
pub fn plane_side(c: Vec4, min: Vec3, max: Vec3) -> std::cmp::Ordering {
    let n = c.truncate();
    let up = n.cmpge(Vec3::ZERO);
    // The corners farthest along n and against n.
    let far = Vec3::select(up, max, min);
    let near = Vec3::select(up, min, max);
    if n.dot(far) + c.w < 0.0 {
        std::cmp::Ordering::Less
    } else if n.dot(near) + c.w >= 0.0 {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Equal
    }
}

/// The planes for the `u_clip` uniform. `skip` is a plane to leave out.
fn clip_uniform(planes: &[Vec4], skip: Option<usize>) -> [f32; 4 * MAX_PLANES] {
    let mut out = [0.0; 4 * MAX_PLANES];
    for k in 0..MAX_PLANES {
        let p = match planes.get(k) {
            Some(p) if skip != Some(k) => p.to_array(),
            _ => NO_PLANE,
        };
        out[4 * k..4 * k + 4].copy_from_slice(&p);
    }
    out
}

/// A rectangle on plane `c` that covers the cut of the box `min..max`, as
/// two triangles: 6 vertices of position and normal. The normal points
/// out of the kept side.
fn cap_quad(c: Vec4, min: Vec3, max: Vec3, out: &mut Vec<f32>) {
    let len = c.truncate().length().max(1e-12);
    let (n, w) = (c.truncate() / len, c.w / len);
    let center = (min + max) * 0.5;
    let half = (max - min) * 0.5;
    let p = center - n * (n.dot(center) + w);
    let (u, v) = n.any_orthonormal_pair();
    // The box seen along u and v, with a small margin.
    let margin = 1e-3 + half.max_element() * 0.01;
    let hu = u.abs().dot(half) + margin;
    let hv = v.abs().dot(half) + margin;
    let corners = [
        p - u * hu - v * hv,
        p + u * hu - v * hv,
        p + u * hu + v * hv,
        p - u * hu + v * hv,
    ];
    for k in [0, 1, 2, 0, 2, 3] {
        let q = corners[k];
        out.extend_from_slice(&[q.x, q.y, q.z, -n.x, -n.y, -n.z]);
    }
}

/// One cap: the cut face of plane `plane` through the parts `ranges`
/// (triangle vertex ranges) with one style.
struct Cap {
    style: PartStyle,
    plane: usize,
    ranges: Vec<Range<i32>>,
    /// The first vertex of the rectangle in the cap buffer.
    first: i32,
}

pub struct Renderer {
    mesh_prog: glow::Program,
    line_prog: glow::Program,
    path_prog: glow::Program,
    mesh: Buffer,
    edges: Buffer,
    /// The parts of the assembly in `mesh` and `edges`.
    parts: Vec<GpuPart>,
    grid: Buffer,
    paths: PathBuffer,
    /// Index count at the end of each layer in `paths`.
    layer_ends: Vec<i32>,
    /// The cap rectangles of the frame.
    caps: Buffer,
    /// The plane outlines of the frame.
    outlines: Buffer,
    /// The window has a stencil buffer, for the caps. `None` until the
    /// first frame.
    stencil: Option<bool>,
    mesh_rev: u64,
    slice_rev: u64,
    /// What the grid buffer shows now.
    grid_key: GridKey,
}

/// The content of the grid buffer.
#[derive(Clone, Copy, PartialEq)]
enum GridKey {
    None,
    /// The floor grid: the cells, the floor height (bits) and the theme.
    Floor(i32, i32, i32, i32, u32, bool),
    /// The bed of the Optimize tab: width and depth (bits) and the theme.
    Bed(u32, u32, bool),
}

unsafe fn program(
    gl: &glow::Context,
    version: &str,
    vs: &str,
    fs: &str,
) -> Result<glow::Program, String> {
    unsafe {
        let prog = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
            let sh = gl.create_shader(kind)?;
            gl.shader_source(sh, &format!("{version}\n{src}"));
            gl.compile_shader(sh);
            if !gl.get_shader_compile_status(sh) {
                return Err(gl.get_shader_info_log(sh));
            }
            gl.attach_shader(prog, sh);
            shaders.push(sh);
        }
        gl.link_program(prog);
        if !gl.get_program_link_status(prog) {
            return Err(gl.get_program_info_log(prog));
        }
        for sh in shaders {
            gl.detach_shader(prog, sh);
            gl.delete_shader(sh);
        }
        Ok(prog)
    }
}

impl Renderer {
    pub fn new(gl: &glow::Context) -> Result<Renderer, String> {
        unsafe {
            let version = if gl.version().is_embedded {
                "#version 300 es"
            } else {
                "#version 330 core"
            };
            Ok(Renderer {
                mesh_prog: program(gl, version, MESH_VS, MESH_FS)?,
                line_prog: program(gl, version, LINE_VS, LINE_FS)?,
                path_prog: program(gl, version, PATH_VS, PATH_FS)?,
                mesh: Buffer::new(gl),
                edges: Buffer::new(gl),
                parts: Vec::new(),
                grid: Buffer::new(gl),
                paths: PathBuffer::new(gl),
                layer_ends: Vec::new(),
                caps: Buffer::new(gl),
                outlines: Buffer::new(gl),
                stencil: None,
                mesh_rev: u64::MAX,
                slice_rev: u64::MAX,
                grid_key: GridKey::None,
            })
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.mesh_prog);
            gl.delete_program(self.line_prog);
            gl.delete_program(self.path_prog);
            self.paths.destroy(gl);
            for b in [
                &self.mesh,
                &self.edges,
                &self.grid,
                &self.caps,
                &self.outlines,
            ] {
                b.destroy(gl);
            }
        }
    }

    /// Uploads the display mesh and the wireframe of `assembly` when `rev`
    /// changes. The colors and offsets come with each `View`.
    pub fn set_mesh(&mut self, gl: &glow::Context, assembly: Option<&Assembly>, rev: u64) {
        if rev == self.mesh_rev {
            return;
        }
        self.mesh_rev = rev;
        let mut tri = Vec::new();
        let mut edge = Vec::new();
        self.parts.clear();
        if let Some(a) = assembly {
            let m = &a.mesh;
            tri.reserve(m.triangles.len() * 18);
            for i in 0..m.triangles.len() {
                let n = m.normal(i);
                for v in m.tri(i) {
                    tri.extend_from_slice(&[v.x, v.y, v.z, n.x, n.y, n.z]);
                }
            }
            let c = [0.08, 0.05, 0.02];
            edge.reserve(a.edges.len() * 12);
            for e in &a.edges {
                for v in e.map(|i| m.vertices[i as usize]) {
                    edge.extend_from_slice(&[v.x, v.y, v.z, c[0], c[1], c[2]]);
                }
            }
            let vertices =
                |r: &Range<usize>, per: usize| (r.start * per) as i32..(r.end * per) as i32;
            self.parts = (a.parts.iter())
                .map(|p| GpuPart {
                    tris: vertices(&p.tris, 3),
                    edges: vertices(&p.edges, 2),
                    center: p.center,
                    min: p.bounds.min,
                    max: p.bounds.max,
                })
                .collect();
        }
        unsafe {
            self.mesh.upload(gl, &tri);
            self.edges.upload(gl, &edge);
        }
    }

    pub fn set_slice(&mut self, gl: &glow::Context, sliced: Option<&Sliced>, rev: u64) {
        if rev == self.slice_rev {
            return;
        }
        self.slice_rev = rev;
        let e = sliced.map(extrude).unwrap_or(Extrusions {
            vertices: Vec::new(),
            indices: Vec::new(),
            layer_ends: Vec::new(),
        });
        unsafe { self.paths.upload(gl, &e) };
        self.layer_ends = e.layer_ends;
    }

    /// Bed grid around the model: 10 mm cells, brighter every 50 mm, X/Y axes.
    pub fn set_grid(&mut self, gl: &glow::Context, min: Vec3, max: Vec3, dark: bool) {
        let snap = |v: f32, up: bool| -> i32 {
            let s = if up {
                (v / 10.0).ceil()
            } else {
                (v / 10.0).floor()
            };
            s as i32 * 10
        };
        let pad = 20.0;
        // The floor is below the lowest point, for example a part that the
        // explode moves down.
        let z = min.z.min(0.0) - 0.01;
        let (x0, y0, x1, y1) = (
            snap(min.x - pad, false),
            snap(min.y - pad, false),
            snap(max.x + pad, true),
            snap(max.y + pad, true),
        );
        let key = GridKey::Floor(x0, y0, x1, y1, z.to_bits(), dark);
        if key == self.grid_key {
            return;
        }
        self.grid_key = key;
        let mut d = Vec::new();
        let mut line = |a: [f32; 2], b: [f32; 2], c: [f32; 3]| {
            d.extend_from_slice(&[
                a[0], a[1], z, c[0], c[1], c[2], b[0], b[1], z, c[0], c[1], c[2],
            ]);
        };
        let (minor, major) = grid_colors(dark);
        for x in (x0..=x1).step_by(10) {
            if x != 0 {
                line(
                    [x as f32, y0 as f32],
                    [x as f32, y1 as f32],
                    if x % 50 == 0 { major } else { minor },
                );
            }
        }
        for y in (y0..=y1).step_by(10) {
            if y != 0 {
                line(
                    [x0 as f32, y as f32],
                    [x1 as f32, y as f32],
                    if y % 50 == 0 { major } else { minor },
                );
            }
        }
        line([x0 as f32, 0.0], [x1 as f32, 0.0], AXIS_X);
        line([0.0, y0 as f32], [0.0, y1 as f32], AXIS_Y);
        unsafe { self.grid.upload(gl, &d) };
    }

    /// The bed of the Optimize tab in the grid buffer: a grid from (0, 0) to
    /// (`w`, `d`) at z = 0, 10 mm cells, brighter every 50 mm, and the
    /// outline. The X and Y axes are the front and left edges.
    pub fn set_bed(&mut self, gl: &glow::Context, w: f32, d: f32, dark: bool) {
        let key = GridKey::Bed(w.to_bits(), d.to_bits(), dark);
        if key == self.grid_key {
            return;
        }
        self.grid_key = key;
        let data = bed_lines(w, d, dark);
        unsafe { self.grid.upload(gl, &data) };
    }

    pub fn paint(&mut self, gl: &glow::Context, view: &View) {
        unsafe {
            if self.stencil.is_none() {
                let bits = stencil_bits(gl);
                if bits == 0 {
                    eprintln!("renderer: no stencil buffer, the section shows no cut faces");
                }
                self.stencil = Some(bits > 0);
            }
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            let mvp = view.mvp.to_cols_array();

            gl.use_program(Some(self.line_prog));
            let u = |p, n: &str| gl.get_uniform_location(p, n);
            gl.uniform_matrix_4_f32_slice(u(self.line_prog, "u_mvp").as_ref(), false, &mvp);
            gl.uniform_3_f32(u(self.line_prog, "u_offset").as_ref(), 0.0, 0.0, 0.0);
            gl.uniform_1_f32(u(self.line_prog, "u_alpha").as_ref(), 1.0);
            gl.uniform_1_f32(u(self.line_prog, "u_tint").as_ref(), 1.0);
            let no_clip = clip_uniform(&[], None);
            gl.uniform_4_f32_slice(u(self.line_prog, "u_clip").as_ref(), &no_clip);
            if view.grid {
                self.grid.draw(gl, glow::LINES, 0, self.grid.count);
            }

            if let Some(layer) = view.layer.filter(|_| !self.layer_ends.is_empty()) {
                let layer = layer.min(self.layer_ends.len() - 1);
                let end = self.layer_ends[layer];
                let start = if layer == 0 {
                    0
                } else {
                    self.layer_ends[layer - 1]
                };
                gl.use_program(Some(self.path_prog));
                gl.uniform_matrix_4_f32_slice(u(self.path_prog, "u_mvp").as_ref(), false, &mvp);
                gl.uniform_3_f32(
                    u(self.path_prog, "u_eye").as_ref(),
                    view.eye.x,
                    view.eye.y,
                    view.eye.z,
                );
                gl.uniform_1_f32(u(self.path_prog, "u_tint").as_ref(), 0.8);
                self.paths.draw(gl, 0, start);
                gl.uniform_1_f32(u(self.path_prog, "u_tint").as_ref(), 1.1);
                self.paths.draw(gl, start, end);
            }

            if view.show_mesh && self.mesh.count > 0 {
                self.paint_parts(gl, view);
            }
            if !view.section.outlines.is_empty() {
                self.paint_outlines(gl, view);
            }

            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.disable(glow::DEPTH_TEST);
        }
    }
}

impl Renderer {
    /// Draws the parts: first the opaque parts, their cut faces and their
    /// wireframe, then the transparent parts from back to front, without
    /// depth writes. Neighbour parts with the same style share one draw
    /// call.
    unsafe fn paint_parts(&mut self, gl: &glow::Context, view: &View) {
        unsafe {
            let u = |p, n: &str| gl.get_uniform_location(p, n);
            let mp = self.mesh_prog;
            let (u_offset, u_color) = (u(mp, "u_offset"), u(mp, "u_color"));
            let (u_alpha, u_highlight) = (u(mp, "u_alpha"), u(mp, "u_highlight"));
            let (u_back, u_clip) = (u(mp, "u_back"), u(mp, "u_clip"));
            let planes = &view.section.planes[..view.section.planes.len().min(MAX_PLANES)];
            let all_planes = clip_uniform(planes, None);
            let caps = !planes.is_empty() && self.stencil == Some(true);
            gl.use_program(Some(mp));
            gl.uniform_matrix_4_f32_slice(
                u(mp, "u_mvp").as_ref(),
                false,
                &view.mvp.to_cols_array(),
            );
            gl.uniform_3_f32(u(mp, "u_eye").as_ref(), view.eye.x, view.eye.y, view.eye.z);
            gl.uniform_4_f32_slice(u_clip.as_ref(), &all_planes);
            let style = |i: usize| {
                let mut s = view.parts.get(i).copied().unwrap_or_default();
                s.alpha *= view.mesh_alpha;
                s
            };
            let set = |s: &PartStyle| {
                gl.uniform_3_f32(u_offset.as_ref(), s.offset.x, s.offset.y, s.offset.z);
                gl.uniform_3_f32(u_color.as_ref(), s.color[0], s.color[1], s.color[2]);
                gl.uniform_1_f32(u_alpha.as_ref(), s.alpha);
                gl.uniform_1_f32(u_highlight.as_ref(), if s.highlight { 1.0 } else { 0.0 });
                // In a cut, the inside of a part shows in a dark part color,
                // not as open geometry. With cut faces, it has the color of
                // the cut face: the edge pixels of the cut face then agree.
                // Through a transparent part, the inside shows in a dark
                // part color too, for example a reference part.
                let back = if planes.is_empty() && s.alpha >= 1.0 {
                    PURPLE
                } else if caps && s.alpha >= 1.0 {
                    s.color.map(|c| c * CAP_SHADE)
                } else {
                    s.color.map(|c| c * 0.55)
                };
                gl.uniform_3_f32(u_back.as_ref(), back[0], back[1], back[2]);
            };
            // Runs of neighbour parts with the same style: (style, first part, end part).
            let mut runs: Vec<(PartStyle, usize, usize)> = Vec::new();
            for i in 0..self.parts.len() {
                let s = style(i);
                match runs.last_mut() {
                    Some((last, _, end)) if *last == s => *end = i + 1,
                    _ => runs.push((s, i, i + 1)),
                }
            }
            let tris = |a: usize, b: usize| {
                let r = self.parts[a].tris.start..self.parts[b - 1].tris.end;
                (r.start, r.end - r.start)
            };

            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(1.0, 1.0);
            let opaque: Vec<_> = runs.iter().filter(|r| r.0.alpha >= 1.0).collect();
            for (s, a, b) in &opaque {
                set(s);
                let (first, count) = tris(*a, *b);
                self.mesh.draw(gl, glow::TRIANGLES, first, count);
            }
            if caps {
                let (list, quads) = cap_list(&self.parts, &opaque, planes);
                self.caps.upload(gl, &quads);
                for cap in &list {
                    // Count the faces behind or in front of the plane.
                    gl.clear_stencil(0);
                    gl.clear(glow::STENCIL_BUFFER_BIT);
                    gl.enable(glow::STENCIL_TEST);
                    gl.stencil_mask(0xff);
                    gl.stencil_func(glow::ALWAYS, 0, 0xff);
                    gl.stencil_op_separate(glow::FRONT, glow::KEEP, glow::KEEP, glow::INCR_WRAP);
                    gl.stencil_op_separate(glow::BACK, glow::KEEP, glow::KEEP, glow::DECR_WRAP);
                    gl.color_mask(false, false, false, false);
                    gl.depth_mask(false);
                    gl.disable(glow::DEPTH_TEST);
                    let only = clip_uniform(&planes[cap.plane..=cap.plane], None);
                    gl.uniform_4_f32_slice(u_clip.as_ref(), &only);
                    set(&cap.style);
                    for r in &cap.ranges {
                        self.mesh
                            .draw(gl, glow::TRIANGLES, r.start, r.end - r.start);
                    }
                    // The plane where the count is not 0.
                    gl.color_mask(true, true, true, true);
                    gl.depth_mask(true);
                    gl.enable(glow::DEPTH_TEST);
                    gl.stencil_func(glow::NOTEQUAL, 0, 0xff);
                    gl.stencil_op(glow::KEEP, glow::KEEP, glow::KEEP);
                    let others = clip_uniform(planes, Some(cap.plane));
                    gl.uniform_4_f32_slice(u_clip.as_ref(), &others);
                    let dark = cap.style.color.map(|c| c * CAP_SHADE);
                    gl.uniform_3_f32(u_color.as_ref(), dark[0], dark[1], dark[2]);
                    gl.uniform_3_f32(u_back.as_ref(), dark[0], dark[1], dark[2]);
                    self.caps.draw(gl, glow::TRIANGLES, cap.first, 6);
                }
                gl.disable(glow::STENCIL_TEST);
                gl.uniform_4_f32_slice(u_clip.as_ref(), &all_planes);
            }
            gl.disable(glow::POLYGON_OFFSET_FILL);

            if view.wireframe && !opaque.is_empty() {
                let lp = self.line_prog;
                let u_line_offset = u(lp, "u_offset");
                gl.use_program(Some(lp));
                gl.uniform_1_f32(u(lp, "u_tint").as_ref(), 1.0);
                gl.uniform_1_f32(u(lp, "u_alpha").as_ref(), 0.6);
                gl.uniform_4_f32_slice(u(lp, "u_clip").as_ref(), &all_planes);
                for (s, a, b) in &opaque {
                    let o = s.offset;
                    gl.uniform_3_f32(u_line_offset.as_ref(), o.x, o.y, o.z);
                    let r = self.parts[*a].edges.start..self.parts[*b - 1].edges.end;
                    self.edges.draw(gl, glow::LINES, r.start, r.end - r.start);
                }
                gl.uniform_4_f32_slice(u(lp, "u_clip").as_ref(), &clip_uniform(&[], None));
                gl.use_program(Some(mp));
            }

            let mut clear: Vec<(f32, usize)> = (0..self.parts.len())
                .filter(|&i| style(i).alpha > 0.0 && style(i).alpha < 1.0)
                .map(|i| {
                    let at = self.parts[i].center + style(i).offset;
                    (at.distance_squared(view.eye), i)
                })
                .collect();
            if !clear.is_empty() {
                clear.sort_by(|a, b| b.0.total_cmp(&a.0));
                gl.depth_mask(false);
                gl.enable(glow::POLYGON_OFFSET_FILL);
                for (_, i) in clear {
                    set(&style(i));
                    let (first, count) = tris(i, i + 1);
                    self.mesh.draw(gl, glow::TRIANGLES, first, count);
                }
                gl.disable(glow::POLYGON_OFFSET_FILL);
                gl.depth_mask(true);
            }
        }
    }
}

/// The caps of the opaque runs, and their rectangles (see `cap_quad`). A
/// cap holds the parts of the run that the plane cuts and that the other
/// planes do not remove completely.
fn cap_list(
    parts: &[GpuPart],
    opaque: &[&(PartStyle, usize, usize)],
    planes: &[Vec4],
) -> (Vec<Cap>, Vec<f32>) {
    use std::cmp::Ordering;
    let mut list = Vec::new();
    let mut quads = Vec::new();
    for (s, a, b) in opaque {
        for (j, &c) in planes.iter().enumerate() {
            let mut ranges: Vec<Range<i32>> = Vec::new();
            let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for p in &parts[*a..*b] {
                let cut = plane_side(c, p.min, p.max) == Ordering::Equal;
                let removed = (planes.iter().enumerate())
                    .any(|(k, o)| k != j && plane_side(*o, p.min, p.max) == Ordering::Less);
                if !cut || removed || p.tris.is_empty() {
                    continue;
                }
                match ranges.last_mut() {
                    Some(r) if r.end == p.tris.start => r.end = p.tris.end,
                    _ => ranges.push(p.tris.clone()),
                }
                min = min.min(p.min);
                max = max.max(p.max);
            }
            if ranges.is_empty() {
                continue;
            }
            let first = (quads.len() / 6) as i32;
            cap_quad(c, min, max, &mut quads);
            list.push(Cap {
                style: *s,
                plane: j,
                ranges,
                first,
            });
        }
    }
    (list, quads)
}

impl Renderer {
    /// Draws the plane outlines over the model.
    unsafe fn paint_outlines(&mut self, gl: &glow::Context, view: &View) {
        let mut d = Vec::new();
        for r in &view.section.outlines {
            for k in 0..4 {
                for q in [r[k], r[(k + 1) % 4]] {
                    d.extend_from_slice(&[q.x, q.y, q.z, OUTLINE[0], OUTLINE[1], OUTLINE[2]]);
                }
            }
        }
        unsafe {
            self.outlines.upload(gl, &d);
            let lp = self.line_prog;
            let u = |n: &str| gl.get_uniform_location(lp, n);
            gl.use_program(Some(lp));
            gl.uniform_matrix_4_f32_slice(u("u_mvp").as_ref(), false, &view.mvp.to_cols_array());
            gl.uniform_3_f32(u("u_offset").as_ref(), 0.0, 0.0, 0.0);
            gl.uniform_1_f32(u("u_tint").as_ref(), 1.0);
            gl.uniform_4_f32_slice(u("u_clip").as_ref(), &clip_uniform(&[], None));
            // Faint behind the model, brighter in front of it.
            gl.depth_mask(false);
            gl.disable(glow::DEPTH_TEST);
            gl.uniform_1_f32(u("u_alpha").as_ref(), 0.35);
            self.outlines.draw(gl, glow::LINES, 0, self.outlines.count);
            gl.enable(glow::DEPTH_TEST);
            gl.uniform_1_f32(u("u_alpha").as_ref(), 0.6);
            self.outlines.draw(gl, glow::LINES, 0, self.outlines.count);
            gl.depth_mask(true);
        }
    }
}

/// The bits of the stencil buffer of the window. 0 when the query fails.
fn stencil_bits(gl: &glow::Context) -> i32 {
    unsafe {
        // Older errors are not ours.
        for _ in 0..8 {
            if gl.get_error() == glow::NO_ERROR {
                break;
            }
        }
        let bits = gl.get_framebuffer_attachment_parameter_i32(
            glow::FRAMEBUFFER,
            glow::STENCIL,
            glow::FRAMEBUFFER_ATTACHMENT_STENCIL_SIZE,
        );
        if gl.get_error() == glow::NO_ERROR {
            bits
        } else {
            0
        }
    }
}

/// The colors of the X and Y axis lines.
const AXIS_X: [f32; 3] = [0.85, 0.3, 0.3];
const AXIS_Y: [f32; 3] = [0.35, 0.8, 0.4];

/// The minor and the major grid line colors.
fn grid_colors(dark: bool) -> ([f32; 3], [f32; 3]) {
    if dark {
        ([0.22, 0.23, 0.26], [0.34, 0.35, 0.39])
    } else {
        ([0.60, 0.63, 0.67], [0.47, 0.50, 0.55])
    }
}

/// The lines of a bed of `w` x `d` mm (see `Renderer::set_bed`): position
/// and color of each end, 6 floats each.
fn bed_lines(w: f32, d: f32, dark: bool) -> Vec<f32> {
    let z = -0.01;
    let (minor, major) = grid_colors(dark);
    let edge = if dark {
        [0.55, 0.57, 0.62]
    } else {
        [0.32, 0.35, 0.40]
    };
    let mut out = Vec::new();
    let mut line = |a: [f32; 2], b: [f32; 2], c: [f32; 3]| {
        out.extend_from_slice(&[
            a[0], a[1], z, c[0], c[1], c[2], b[0], b[1], z, c[0], c[1], c[2],
        ]);
    };
    let (w, d) = (w.max(0.0), d.max(0.0));
    let mut x = 10;
    while (x as f32) < w - 0.5 {
        let c = if x % 50 == 0 { major } else { minor };
        line([x as f32, 0.0], [x as f32, d], c);
        x += 10;
    }
    let mut y = 10;
    while (y as f32) < d - 0.5 {
        let c = if y % 50 == 0 { major } else { minor };
        line([0.0, y as f32], [w, y as f32], c);
        y += 10;
    }
    line([w, 0.0], [w, d], edge);
    line([0.0, d], [w, d], edge);
    line([0.0, 0.0], [w, 0.0], AXIS_X);
    line([0.0, 0.0], [0.0, d], AXIS_Y);
    out
}

pub fn kind_color(k: Kind) -> [f32; 3] {
    match k {
        Kind::OuterWall => ORANGE,
        Kind::InnerWall => [1.0, 0.82, 0.4],
        Kind::Infill => [0.88, 0.35, 0.28],
        Kind::Solid => [0.75, 0.47, 0.87],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn bed_lines_stay_on_the_bed() {
        let v = bed_lines(255.0, 210.0, true);
        assert_eq!(v.len() % 12, 0);
        // 25 inner lines across x, 20 across y, and the 4 edges.
        assert_eq!(v.len() / 12, 25 + 20 + 4);
        for p in v.chunks(6) {
            assert!((0.0..=255.0).contains(&p[0]) && (0.0..=210.0).contains(&p[1]));
        }
    }

    #[test]
    fn plane_side_of_a_box() {
        let (min, max) = (Vec3::ZERO, Vec3::splat(10.0));
        // x >= 5 cuts the box, x >= 20 removes it, x >= -1 keeps it.
        assert_eq!(
            plane_side(Vec4::new(1.0, 0.0, 0.0, -5.0), min, max),
            Ordering::Equal
        );
        assert_eq!(
            plane_side(Vec4::new(1.0, 0.0, 0.0, -20.0), min, max),
            Ordering::Less
        );
        assert_eq!(
            plane_side(Vec4::new(1.0, 0.0, 0.0, 1.0), min, max),
            Ordering::Greater
        );
        // x <= -1 removes the box, z <= 3 cuts it.
        assert_eq!(
            plane_side(Vec4::new(-1.0, 0.0, 0.0, -1.0), min, max),
            Ordering::Less
        );
        assert_eq!(
            plane_side(Vec4::new(0.0, 0.0, -1.0, 3.0), min, max),
            Ordering::Equal
        );
    }

    #[test]
    fn clip_uniform_fills_unused_planes() {
        let a = Vec4::new(1.0, 0.0, 0.0, -5.0);
        let b = Vec4::new(0.0, -1.0, 0.0, 2.0);
        let u = clip_uniform(&[a, b], None);
        assert_eq!(&u[0..4], &a.to_array());
        assert_eq!(&u[4..8], &b.to_array());
        assert_eq!(&u[8..12], &NO_PLANE);
        // The cap of plane 0 is clipped by the other planes only.
        let u = clip_uniform(&[a, b], Some(0));
        assert_eq!(&u[0..4], &NO_PLANE);
        assert_eq!(&u[4..8], &b.to_array());
    }

    #[test]
    fn cap_quad_covers_the_cut() {
        let (min, max) = (Vec3::new(0.0, -2.0, 1.0), Vec3::new(10.0, 4.0, 9.0));
        let mut q = Vec::new();
        cap_quad(Vec4::new(-1.0, 0.0, 0.0, 6.0), min, max, &mut q);
        assert_eq!(q.len(), 36);
        let pts: Vec<Vec3> = q.chunks(6).map(|v| Vec3::new(v[0], v[1], v[2])).collect();
        // All corners are on the plane x = 6, the normal points to +X (out
        // of the kept side x <= 6).
        assert!(pts.iter().all(|p| (p.x - 6.0).abs() < 1e-4));
        assert_eq!(Vec3::new(q[3], q[4], q[5]), Vec3::X);
        let lo = pts.iter().fold(Vec3::MAX, |a, p| a.min(*p));
        let hi = pts.iter().fold(Vec3::MIN, |a, p| a.max(*p));
        assert!(lo.y <= -2.0 && lo.z <= 1.0 && hi.y >= 4.0 && hi.z >= 9.0);
    }
}
