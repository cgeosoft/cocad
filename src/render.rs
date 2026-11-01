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
    mesh: Buffer,
    edges: Buffer,
    grid: Buffer,
    paths: Buffer,
    /// Vertex count at the end of each layer in `paths`.
    layer_ends: Vec<i32>,
    mesh_rev: u64,
    slice_rev: u64,
    grid_key: (i32, i32, i32, i32),
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
                mesh: Buffer::new(gl),
                edges: Buffer::new(gl),
                grid: Buffer::new(gl),
                paths: Buffer::new(gl),
                layer_ends: Vec::new(),
                mesh_rev: u64::MAX,
                slice_rev: u64::MAX,
                grid_key: (0, 0, 0, -1),
            })
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.mesh_prog);
            gl.delete_program(self.line_prog);
            for b in [&self.mesh, &self.edges, &self.grid, &self.paths] {
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
        self.layer_ends.clear();
        let mut data = Vec::new();
        if let Some(s) = sliced {
            for layer in &s.layers {
                let z = layer.z;
                for p in &layer.paths {
                    let c = kind_color(p.kind);
                    let n = p.pts.len();
                    let segs = if p.closed { n } else { n.saturating_sub(1) };
                    for i in 0..segs {
                        for q in [p.pts[i], p.pts[(i + 1) % n]] {
                            data.extend_from_slice(&[q.x, q.y, z, c[0], c[1], c[2]]);
                        }
                    }
                }
                self.layer_ends.push((data.len() / 6) as i32);
            }
        }
        unsafe { self.paths.upload(gl, &data) };
    }

    /// Bed grid around the model: 10 mm cells, brighter every 50 mm, X/Y axes.
    pub fn set_grid(&mut self, gl: &glow::Context, min: Vec3, max: Vec3) {
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
        );
        if key == self.grid_key {
            return;
        }
        self.grid_key = key;
        let (x0, y0, x1, y1) = key;
        let z = min.z.min(0.0) - 0.01;
        let mut d = Vec::new();
        let mut line = |a: [f32; 2], b: [f32; 2], c: [f32; 3]| {
            d.extend_from_slice(&[
                a[0], a[1], z, c[0], c[1], c[2], b[0], b[1], z, c[0], c[1], c[2],
            ]);
        };
        let minor = [0.22, 0.23, 0.26];
        let major = [0.34, 0.35, 0.39];
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
                gl.uniform_1_f32(u(self.line_prog, "u_tint").as_ref(), 0.55);
                self.paths.draw(gl, glow::LINES, 0, start);
                gl.uniform_1_f32(u(self.line_prog, "u_tint").as_ref(), 1.15);
                self.paths.draw(gl, glow::LINES, start, end - start);
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
