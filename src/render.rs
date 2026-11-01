//! OpenGL renderer for the 3D view: shaded mesh, wireframe, bed grid and
//! slicer toolpaths. It runs inside an egui paint callback.

use crate::mesh::Mesh;
use crate::slicer::{Kind, Sliced};
use eframe::glow::{self, HasContext};
use glam::{Mat4, Vec3};

pub const ORANGE: [f32; 3] = [1.0, 0.576, 0.0]; // #ff9300

struct Buffer {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    count: i32,
}

impl Buffer {
    unsafe fn new(gl: &glow::Context) -> Buffer {
        unsafe {
            let vao = gl.create_vertex_array().unwrap();
            let vbo = gl.create_buffer().unwrap();
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            // Two vec3 attributes per vertex: position + normal or color.
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 24, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, 24, 12);
            gl.bind_vertex_array(None);
            Buffer { vao, vbo, count: 0 }
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
        self.count = (data.len() / 6) as i32;
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
out vec3 v_nrm;
out vec3 v_pos;
void main() {
    v_nrm = a_nrm;
    v_pos = a_pos;
    gl_Position = u_mvp * vec4(a_pos, 1.0);
}
"#;

const MESH_FS: &str = r#"
precision highp float;
in vec3 v_nrm;
in vec3 v_pos;
uniform vec3 u_eye;
uniform vec3 u_color;
uniform float u_alpha;
out vec4 o_color;
void main() {
    vec3 n = normalize(v_nrm);
    vec3 base = u_color;
    if (!gl_FrontFacing) {
        // Back faces show inside-out or open geometry.
        n = -n;
        base = vec3(0.55, 0.18, 0.62);
    }
    vec3 v = normalize(u_eye - v_pos);
    vec3 key = normalize(vec3(0.45, 0.3, 1.0));
    vec3 fill = normalize(vec3(-0.6, -0.4, 0.2));
    float diff = 0.22 + 0.5 * max(dot(n, key), 0.0) + 0.15 * max(dot(n, fill), 0.0) + 0.25 * max(dot(n, v), 0.0);
    float spec = pow(max(dot(n, normalize(key + v)), 0.0), 48.0) * 0.3;
    float rim = pow(1.0 - max(dot(n, v), 0.0), 3.0) * 0.18;
    vec3 c = base * diff + vec3(spec) + rim * vec3(1.0, 0.85, 0.7);
    o_color = vec4(pow(c, vec3(0.95)), u_alpha);
}
"#;

const LINE_VS: &str = r#"
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_col;
uniform mat4 u_mvp;
out vec3 v_col;
void main() {
    v_col = a_col;
    gl_Position = u_mvp * vec4(a_pos, 1.0);
}
"#;

const LINE_FS: &str = r#"
precision highp float;
in vec3 v_col;
uniform float u_tint;
uniform float u_alpha;
out vec4 o_color;
void main() {
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
    pub mesh_alpha: f32,
    pub wireframe: bool,
    pub grid: bool,
    /// Slice preview: draw layers 0..=layer.
    pub layer: Option<usize>,
}

pub struct Renderer {
    mesh_prog: glow::Program,
    line_prog: glow::Program,
    path_prog: glow::Program,
    mesh: Buffer,
    edges: Buffer,
    grid: Buffer,
    paths: PathBuffer,
    /// Index count at the end of each layer in `paths`.
    layer_ends: Vec<i32>,
    mesh_rev: u64,
    slice_rev: u64,
    grid_key: (i32, i32, i32, i32, bool),
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
                grid: Buffer::new(gl),
                paths: PathBuffer::new(gl),
                layer_ends: Vec::new(),
                mesh_rev: u64::MAX,
                slice_rev: u64::MAX,
                grid_key: (0, 0, 0, -1, true),
            })
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.mesh_prog);
            gl.delete_program(self.line_prog);
            gl.delete_program(self.path_prog);
            self.paths.destroy(gl);
            for b in [&self.mesh, &self.edges, &self.grid] {
                b.destroy(gl);
            }
        }
    }

    pub fn set_mesh(&mut self, gl: &glow::Context, mesh: Option<&Mesh>, rev: u64) {
        if rev == self.mesh_rev {
            return;
        }
        self.mesh_rev = rev;
        let mut tri = Vec::new();
        let mut edge = Vec::new();
        if let Some(m) = mesh {
            tri.reserve(m.triangles.len() * 18);
            for i in 0..m.triangles.len() {
                let n = m.normal(i);
                for v in m.tri(i) {
                    tri.extend_from_slice(&[v.x, v.y, v.z, n.x, n.y, n.z]);
                }
            }
            let c = [0.08, 0.05, 0.02];
            for (a, b) in m.edges() {
                for v in [m.vertices[a as usize], m.vertices[b as usize]] {
                    edge.extend_from_slice(&[v.x, v.y, v.z, c[0], c[1], c[2]]);
                }
            }
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
        let key = (
            snap(min.x - pad, false),
            snap(min.y - pad, false),
            snap(max.x + pad, true),
            snap(max.y + pad, true),
            dark,
        );
        if key == self.grid_key {
            return;
        }
        self.grid_key = key;
        let (x0, y0, x1, y1, _) = key;
        let z = min.z.min(0.0) - 0.01;
        let mut d = Vec::new();
        let mut line = |a: [f32; 2], b: [f32; 2], c: [f32; 3]| {
            d.extend_from_slice(&[
                a[0], a[1], z, c[0], c[1], c[2], b[0], b[1], z, c[0], c[1], c[2],
            ]);
        };
        let (minor, major) = if dark {
            ([0.22, 0.23, 0.26], [0.34, 0.35, 0.39])
        } else {
            ([0.70, 0.72, 0.76], [0.55, 0.58, 0.63])
        };
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
        line([x0 as f32, 0.0], [x1 as f32, 0.0], [0.85, 0.3, 0.3]);
        line([0.0, y0 as f32], [0.0, y1 as f32], [0.35, 0.8, 0.4]);
        unsafe { self.grid.upload(gl, &d) };
    }

    pub fn paint(&self, gl: &glow::Context, view: &View) {
        unsafe {
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
            gl.uniform_1_f32(u(self.line_prog, "u_alpha").as_ref(), 1.0);
            gl.uniform_1_f32(u(self.line_prog, "u_tint").as_ref(), 1.0);
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
                gl.use_program(Some(self.mesh_prog));
                gl.uniform_matrix_4_f32_slice(u(self.mesh_prog, "u_mvp").as_ref(), false, &mvp);
                gl.uniform_3_f32(
                    u(self.mesh_prog, "u_eye").as_ref(),
                    view.eye.x,
                    view.eye.y,
                    view.eye.z,
                );
                gl.uniform_3_f32(
                    u(self.mesh_prog, "u_color").as_ref(),
                    ORANGE[0],
                    ORANGE[1],
                    ORANGE[2],
                );
                gl.uniform_1_f32(u(self.mesh_prog, "u_alpha").as_ref(), view.mesh_alpha);
                let ghost = view.mesh_alpha < 1.0;
                if ghost {
                    gl.depth_mask(false);
                }
                gl.enable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(1.0, 1.0);
                self.mesh.draw(gl, glow::TRIANGLES, 0, self.mesh.count);
                gl.disable(glow::POLYGON_OFFSET_FILL);
                gl.depth_mask(true);

                if view.wireframe && !ghost {
                    gl.use_program(Some(self.line_prog));
                    gl.uniform_1_f32(u(self.line_prog, "u_tint").as_ref(), 1.0);
                    gl.uniform_1_f32(u(self.line_prog, "u_alpha").as_ref(), 0.6);
                    self.edges.draw(gl, glow::LINES, 0, self.edges.count);
                }
            }

            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.disable(glow::DEPTH_TEST);
        }
    }
}

pub fn kind_color(k: Kind) -> [f32; 3] {
    match k {
        Kind::OuterWall => ORANGE,
        Kind::InnerWall => [1.0, 0.82, 0.4],
        Kind::Infill => [0.88, 0.35, 0.28],
        Kind::Solid => [0.75, 0.47, 0.87],
    }
}
