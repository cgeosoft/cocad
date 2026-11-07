//! Runs MCP tool calls inside the window. `crate::mcp` receives them.
//!
//! Calls run on the UI thread in `App::poll`. A call that needs the mesh of a
//! finished render stays in the queue until the render ends or `WAIT` passes.
//! Slow work (exports, photos) runs on its own thread, which sends the reply.

use super::*;
use crate::mcp::{Request, base64};
use serde_json::{Map, Value as Json, json};
use std::sync::mpsc::Sender;

/// Longest wait in the queue. MCP clients often give up after 60 seconds.
const WAIT: Duration = Duration::from_secs(45);
const PHOTO_WAIT: Duration = Duration::from_secs(40);
const MAX_IMAGE_SIDE: u64 = 7680;
/// Image results must stay small: clients limit the size of a tool result.
const MAX_VIEW_SIDE: u64 = 1024;
const MAX_SAMPLES: u64 = 4096;
const LOG_LINES: usize = 60;

/// Internal calls: send the state or the slice result when the work ends.
const STATE_WHEN_DONE: &str = "__state";
const SLICE_WHEN_DONE: &str = "__slice";

pub(super) struct Pending {
    req: Request,
    deadline: Instant,
}

type Reply = Sender<Result<Json, String>>;
/// `None` means that the reply comes later.
type Outcome = Result<Option<Json>, String>;

fn flag(args: &Json, name: &str, default: bool) -> bool {
    args[name].as_bool().unwrap_or(default)
}

fn absolute(args: &Json) -> Result<PathBuf, String> {
    let s = args["path"].as_str().ok_or("path is required")?;
    let p = PathBuf::from(s);
    if p.is_absolute() {
        Ok(p)
    } else {
        Err(format!("path must be absolute, got {s}"))
    }
}

/// An absolute output path that may be written.
fn output(args: &Json) -> Result<PathBuf, String> {
    let p = absolute(args)?;
    if p.exists() && !flag(args, "overwrite", false) {
        return Err(format!(
            "{} exists. Set overwrite to replace it.",
            p.display()
        ));
    }
    Ok(p)
}

fn view_angles(args: &Json) -> Result<Option<(f32, f32)>, String> {
    let Some(name) = args["view"].as_str() else {
        return Ok(None);
    };
    VIEWS
        .iter()
        .find(|(l, _, _)| l.eq_ignore_ascii_case(name))
        .map(|(_, y, p)| Some((*y, *p)))
        .ok_or_else(|| format!("Unknown view {name}. Use iso, top, front or right."))
}

fn side(args: &Json, name: &str, default: u64, max: u64) -> Result<usize, String> {
    match args[name].as_u64().unwrap_or(default) {
        v if (1..=max).contains(&v) => Ok(v as usize),
        v => Err(format!("{name} must be 1 to {max}, got {v}")),
    }
}

impl App {
    /// Starts or stops the listener to match the saved setting.
    pub(super) fn sync_server(&mut self, ctx: &egui::Context) {
        if !self.cfg.mcp {
            self.server = None;
            self.pending.clear();
        } else if self.server.is_none() {
            let ctx = ctx.clone();
            match crate::mcp::serve(move || ctx.request_repaint()) {
                Ok(s) => self.server = Some(s),
                Err(e) => {
                    self.toast = Some((format!("MCP server: {e}"), Instant::now()));
                    eprintln!("mcp: {e}");
                }
            }
        }
    }

    /// Takes new tool calls and runs those that are ready.
    pub(super) fn run_commands(&mut self) {
        if let Some(server) = &self.server {
            while let Ok(req) = server.requests.try_recv() {
                self.pending.push(Pending {
                    req,
                    deadline: Instant::now() + WAIT,
                });
            }
        }
        for p in std::mem::take(&mut self.pending) {
            let busy = self.busy_for(&p.req.tool);
            if busy && Instant::now() < p.deadline {
                self.pending.push(p);
                continue;
            }
            let Request { tool, args, reply } = p.req;
            let out = if busy && tool != STATE_WHEN_DONE {
                Err("The app is still busy. Call get_state and try again later.".into())
            } else {
                self.exec(&tool, &args, &reply)
            };
            match out {
                Ok(Some(v)) => {
                    let _ = reply.send(Ok(v));
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = reply.send(Err(e));
                }
            }
        }
    }

    fn rendering(&self) -> bool {
        self.job.is_some() || (self.auto_render && self.dirty_since.is_some())
    }

    fn busy_for(&self, tool: &str) -> bool {
        match tool {
            SLICE_WHEN_DONE => self.slice_job.is_some(),
            STATE_WHEN_DONE | "export_model" | "save_photo" | "view_model" | "slice" => {
                self.rendering()
            }
            _ => false,
        }
    }

    fn later(&mut self, tool: &str, reply: &Reply) {
        self.pending.push(Pending {
            req: Request {
                tool: tool.to_string(),
                args: Json::Null,
                reply: reply.clone(),
            },
            deadline: Instant::now() + WAIT,
        });
    }

    fn state_after(&mut self, args: &Json, reply: &Reply) -> Outcome {
        if flag(args, "wait", true) {
            self.later(STATE_WHEN_DONE, reply);
            Ok(None)
        } else {
            Ok(Some(self.state()))
        }
    }

    fn note(&mut self, msg: String) {
        self.toast = Some((format!("MCP: {msg}"), Instant::now()));
    }

    fn exec(&mut self, tool: &str, args: &Json, reply: &Reply) -> Outcome {
        match tool {
            "get_state" | STATE_WHEN_DONE => Ok(Some(self.state())),
            "get_params" => {
                self.need_model()?;
                let list: Vec<Json> = self.params.iter().map(params::describe).collect();
                Ok(Some(json!({ "params": list })))
            }
            "get_source" => {
                let file = self.need_model()?;
                let source = std::fs::read_to_string(&file)
                    .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
                Ok(Some(json!({ "file": file, "source": source })))
            }
            "open_model" => {
                let path = absolute(args)?;
                if !path.is_file() {
                    return Err(format!("{} is not a file", path.display()));
                }
                self.open(path.clone());
                self.note(format!("opened {}", self.stem()));
                self.state_after(args, reply)
            }
            "set_params" => self.set_params(args, reply),
            "reset_params" => self.reset_params(args, reply),
            "render" => {
                self.need_model()?;
                if self.file.as_deref().and_then(mtime) != self.mtime {
                    self.reload();
                } else {
                    self.start_render();
                }
                self.state_after(args, reply)
            }
            "load_parameter_set" => self.load_set(args, reply),
            "save_parameter_set" => self.save_set(args),
            "export_model" => self.export_to(args, reply),
            "view_model" => self.view_image(args, reply),
            "save_photo" => self.save_photo_to(args, reply),
            "slice" => self.slice_now(args, reply),
            SLICE_WHEN_DONE => self.slice_result().map(Some),
            "set_view" => self.view_change(args),
            _ => Err(format!("Unknown tool {tool}.")),
        }
    }

    fn need_model(&self) -> Result<PathBuf, String> {
        self.file
            .clone()
            .ok_or_else(|| "No model is open. Call open_model first.".to_string())
    }

    /// The mesh for exports and images: it must match the current values.
    fn current_mesh(&self) -> Result<Arc<Mesh>, String> {
        self.need_model()?;
        if let Some(e) = &self.error {
            return Err(format!(
                "The last render failed, so the mesh is out of date: {e}"
            ));
        }
        if self.dirty_since.is_some() {
            return Err("The values changed after the last render. Call render first.".into());
        }
        self.mesh
            .clone()
            .ok_or_else(|| "The model has no mesh yet. Call render first.".to_string())
    }

    fn state(&self) -> Json {
        let status = if self.file.is_none() {
            "no model"
        } else if self.job.is_some() {
            "rendering"
        } else if self.error.is_some() {
            "failed"
        } else if self.dirty_since.is_some() {
            "needs render"
        } else if self.mesh.is_some() {
            "ready"
        } else {
            "idle"
        };
        let mesh = match (&self.mesh, &self.info) {
            (Some(m), Some(i)) => {
                let s = i.bounds.size();
                json!({
                    "size_mm": [s.x, s.y, s.z],
                    "min_mm": i.bounds.min.to_array(),
                    "max_mm": i.bounds.max.to_array(),
                    "volume_mm3": i.volume,
                    "triangles": m.triangles.len(),
                    "open_edges": i.open_edges,
                })
            }
            _ => Json::Null,
        };
        let changed: Map<String, Json> = self
            .params
            .iter()
            .filter(|p| p.changed())
            .map(|p| (p.name.clone(), params::to_json(&p.value)))
            .collect();
        let mode = match self.mode {
            Mode::Model => "model",
            Mode::Slice => "slice",
            Mode::Photo => "photo",
        };
        let lines: Vec<&str> = self.log.lines().collect();
        let log = lines[lines.len().saturating_sub(LOG_LINES)..].join("\n");
        json!({
            "file": self.file,
            "status": status,
            "error": self.error,
            "render_seconds": self.last_time.map(|t| t.as_secs_f32()),
            "auto_render": self.auto_render,
            "mesh": mesh,
            "parameter_count": self.params.len(),
            "changed_params": changed,
            "parameter_sets": {
                "file": self.sets_file,
                "names": self.sets,
                "active": self.active_set,
                "edited": self.set_edited,
            },
            "mode": mode,
            "wireframe": self.wireframe,
            "grid": self.grid,
            "log": log,
        })
    }

    fn set_params(&mut self, args: &Json, reply: &Reply) -> Outcome {
        self.need_model()?;
        let values = args["values"]
            .as_object()
            .ok_or("values must be an object")?;
        if values.is_empty() {
            return Err("values is empty".into());
        }
        // Check all values first: a rejected call changes nothing.
        let mut fresh = Vec::with_capacity(values.len());
        let mut errors = Vec::new();
        for (name, v) in values {
            match self.params.iter().position(|p| p.name == *name) {
                Some(i) => match params::from_json(&self.params[i], v) {
                    Ok(value) => fresh.push((i, value)),
                    Err(e) => errors.push(format!("{name}: {e}")),
                },
                None => errors.push(format!("{name}: not a Customizer parameter")),
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("\n"));
        }
        for (i, value) in fresh {
            self.params[i].value = value;
        }
        self.set_edited = self.active_set.is_some();
        let names: Vec<&str> = values.keys().map(String::as_str).collect();
        self.note(format!("changed {}", names.join(", ")));
        if flag(args, "render", true) {
            self.start_render();
            self.state_after(args, reply)
        } else {
            self.dirty_since = Some(Instant::now());
            Ok(Some(self.state()))
        }
    }

    fn reset_params(&mut self, args: &Json, reply: &Reply) -> Outcome {
        self.need_model()?;
        match args["names"].as_array() {
            Some(names) => {
                let mut wanted = Vec::new();
                for n in names {
                    let n = n.as_str().ok_or("names must be strings")?;
                    if !self.params.iter().any(|p| p.name == n) {
                        return Err(format!("{n}: not a Customizer parameter"));
                    }
                    wanted.push(n);
                }
                for p in &mut self.params {
                    if wanted.contains(&p.name.as_str()) {
                        p.value = p.default.clone();
                    }
                }
            }
            None => {
                for p in &mut self.params {
                    p.value = p.default.clone();
                }
            }
        }
        self.set_edited = self.active_set.is_some();
        self.note("reset parameters".into());
        self.start_render();
        self.state_after(args, reply)
    }

    fn load_set(&mut self, args: &Json, reply: &Reply) -> Outcome {
        self.need_model()?;
        let name = args["name"].as_str().map(String::from);
        let mut skipped = Vec::new();
        match &name {
            None => {
                for p in &mut self.params {
                    p.value = p.default.clone();
                }
            }
            Some(n) => {
                self.reload_sets();
                if !self.sets.contains(n) {
                    return Err(format!("No parameter set named {n}."));
                }
                skipped = presets::apply(&self.sets_file, n, &mut self.params)
                    .map_err(|e| e.to_string())?;
            }
        }
        self.active_set = name;
        self.set_edited = false;
        self.confirm_delete = false;
        if !skipped.is_empty() {
            self.note(format!("skipped {}", skipped.join(", ")));
        }
        self.start_render();
        self.state_after(args, reply)
    }

    fn save_set(&mut self, args: &Json) -> Outcome {
        self.need_model()?;
        let name = args["name"].as_str().unwrap_or("").trim().to_string();
        if name.is_empty() {
            return Err("name is required".into());
        }
        if self.sets.contains(&name) && !flag(args, "overwrite", false) {
            return Err(format!("Set {name} exists. Set overwrite to replace it."));
        }
        presets::save(&self.sets_file, &name, &self.params).map_err(|e| e.to_string())?;
        self.sets = presets::names(&self.sets_file);
        self.active_set = Some(name.clone());
        self.set_edited = false;
        self.note(format!("saved set {name}"));
        Ok(Some(json!({ "saved": name, "file": self.sets_file })))
    }

    fn export_to(&mut self, args: &Json, reply: &Reply) -> Outcome {
        let mesh = self.current_mesh()?;
        let path = output(args)?;
        if Format::from_path(&path).is_none() {
            return Err("The path must end in .stl, .3mf or .step.".into());
        }
        let meta = Meta {
            title: self.stem(),
            params: self
                .params
                .iter()
                .map(|p| (p.name.clone(), p.value.to_scad()))
                .collect(),
        };
        let target = match args["slicer"].as_str() {
            Some(name) => Some(
                export::Slicer::from_name(name)
                    .ok_or_else(|| format!("slicer must be prusa or orca, got {name}"))?,
            ),
            None => None,
        };
        let job = match target {
            Some(_) => {
                let file = self.file.clone().ok_or("No model is open.")?;
                Some((file, self.cfg.clone(), self.params.clone()))
            }
            None => None,
        };
        self.note(format!("exporting {}", path.display()));
        let reply = reply.clone();
        std::thread::spawn(move || {
            let written = match (job, target) {
                (Some((file, cfg, params)), Some(slicer)) => {
                    let cancel = Arc::new(AtomicBool::new(false));
                    crate::parts::render(&cfg, &file, &params, &cancel)
                        .and_then(|parts| export::write_parts(&path, &parts, &meta, slicer))
                }
                _ => export::write(&path, &mesh, &meta),
            };
            let r = written
                .map(|_| {
                    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    json!({ "path": path, "bytes": bytes })
                })
                .map_err(|e| e.to_string());
            let _ = reply.send(r);
        });
        Ok(None)
    }

    /// Starts a photo job that frames the whole model.
    fn photo_job_for(
        &self,
        mesh: Arc<Mesh>,
        view: Option<(f32, f32)>,
        size: (usize, usize),
        samples: u32,
    ) -> pathtrace::Job {
        let (yaw, pitch) = view.unwrap_or((self.cam.yaw, self.cam.pitch));
        let aspect = size.0 as f32 / size.1 as f32;
        let cam = pathtrace::fit_camera(&mesh, yaw, pitch, aspect);
        pathtrace::Job::start(mesh, cam, self.photo_look(), size.0, size.1, samples)
    }

    fn view_image(&mut self, args: &Json, reply: &Reply) -> Outcome {
        let mesh = self.current_mesh()?;
        let size = (
            side(args, "width", 640, MAX_VIEW_SIDE)?,
            side(args, "height", 480, MAX_VIEW_SIDE)?,
        );
        let samples = side(args, "samples", 24, MAX_SAMPLES)? as u32;
        let job = self.photo_job_for(mesh, view_angles(args)?, size, samples);
        let reply = reply.clone();
        std::thread::spawn(move || {
            let r = finish_photo(&job, samples).and_then(|n| {
                let f = job.frame.lock().unwrap();
                let png = pathtrace::png_bytes(&f).map_err(|e| e.to_string())?;
                Ok(json!({
                    "image": base64(&png),
                    "info": { "width": f.width, "height": f.height, "samples": n },
                }))
            });
            let _ = reply.send(r);
        });
        Ok(None)
    }

    fn save_photo_to(&mut self, args: &Json, reply: &Reply) -> Outcome {
        let mesh = self.current_mesh()?;
        let path = output(args)?;
        let view = view_angles(args)?;
        let width = side(args, "width", self.photo.width as u64, MAX_IMAGE_SIDE)?;
        let height = side(args, "height", self.photo.height as u64, MAX_IMAGE_SIDE)?;
        let samples = side(args, "samples", self.photo.samples as u64, MAX_SAMPLES)? as u32;
        let color = match args["color"].as_str() {
            Some(c) => {
                let c = c.trim_start_matches('#');
                let n = u32::from_str_radix(c, 16)
                    .ok()
                    .filter(|_| c.len() == 6)
                    .ok_or_else(|| format!("color must be RRGGBB, got {c}"))?;
                Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
            }
            None => None,
        };
        let finish = match args["finish"].as_str() {
            None => None,
            Some("matte") => Some("matte"),
            Some("satin") => Some("satin"),
            Some("gloss") => Some("gloss"),
            Some(f) => return Err(format!("Unknown finish {f}. Use matte, satin or gloss.")),
        };
        let studio = match args["studio"].as_str() {
            None => None,
            Some("dark") => Some(pathtrace::Studio::Dark),
            Some("light") => Some(pathtrace::Studio::Light),
            Some(s) => return Err(format!("Unknown studio {s}. Use dark or light.")),
        };
        // Same values as the Photo panel, so that the panel shows the result.
        let p = &mut self.photo;
        (p.width, p.height, p.samples) = (width, height, samples);
        p.color = color.unwrap_or(p.color);
        p.finish = finish.unwrap_or(p.finish);
        p.studio = studio.unwrap_or(p.studio);
        p.layer_lines = flag(args, "layer_lines", p.layer_lines);

        self.note(format!("rendering {}", path.display()));
        let job = self.photo_job_for(mesh, view, (width, height), samples);
        let reply = reply.clone();
        std::thread::spawn(move || {
            let r = finish_photo(&job, samples).and_then(|n| {
                let f = job.frame.lock().unwrap();
                pathtrace::save_png(&path, &f).map_err(|e| e.to_string())?;
                Ok(json!({ "path": path, "width": f.width, "height": f.height, "samples": n }))
            });
            let _ = reply.send(r);
        });
        Ok(None)
    }

    fn slice_now(&mut self, args: &Json, reply: &Reply) -> Outcome {
        self.current_mesh()?;
        let mut s = self.settings;
        let num = |name: &str, min: f64, max: f64| -> Result<Option<f64>, String> {
            match args[name].as_f64() {
                Some(v) if (min..=max).contains(&v) => Ok(Some(v)),
                Some(v) => Err(format!("{name} must be {min} to {max}, got {v}")),
                None => Ok(None),
            }
        };
        s.layer_height = num("layer_height", 0.02, 2.0)?.map_or(s.layer_height, |v| v as f32);
        s.first_layer = num("first_layer", 0.02, 2.0)?.map_or(s.first_layer, |v| v as f32);
        s.line_width = num("line_width", 0.05, 3.0)?.map_or(s.line_width, |v| v as f32);
        s.walls = num("walls", 0.0, 50.0)?.map_or(s.walls, |v| v as u32);
        s.infill = num("infill", 0.0, 100.0)?.map_or(s.infill, |v| v as f32 / 100.0);
        s.solid_layers = num("solid_layers", 0.0, 100.0)?.map_or(s.solid_layers, |v| v as u32);
        s.speed = num("speed", 1.0, 1000.0)?.map_or(s.speed, |v| v as f32);
        self.settings = s;
        self.start_slice();
        self.later(SLICE_WHEN_DONE, reply);
        Ok(None)
    }

    fn slice_result(&self) -> Result<Json, String> {
        let r = self
            .sliced
            .as_ref()
            .filter(|_| self.slice_for_rev == self.mesh_rev)
            .ok_or("The slice result is out of date. Call slice again.")?;
        let st = &r.stats;
        Ok(json!({
            "layers": st.layers,
            "path_m": st.path_mm / 1000.0,
            "filament_m": st.filament_m,
            "grams": st.grams,
            "print_minutes_rough": st.seconds / 60.0,
            "settings": {
                "layer_height": self.settings.layer_height,
                "first_layer": self.settings.first_layer,
                "line_width": self.settings.line_width,
                "walls": self.settings.walls,
                "infill": self.settings.infill * 100.0,
                "solid_layers": self.settings.solid_layers,
                "speed": self.settings.speed,
            },
        }))
    }

    fn view_change(&mut self, args: &Json) -> Outcome {
        let mode = match args["mode"].as_str() {
            None => None,
            Some("model") => Some(Mode::Model),
            Some("slice") => Some(Mode::Slice),
            Some("photo") => Some(Mode::Photo),
            Some(m) => return Err(format!("Unknown mode {m}. Use model, slice or photo.")),
        };
        let view = view_angles(args)?;
        if let Some(m) = mode {
            self.set_mode(m);
        }
        if let Some((yaw, pitch)) = view {
            self.set_view(yaw, pitch);
        }
        if flag(args, "fit", false) {
            self.fit();
        }
        self.wireframe = flag(args, "wireframe", self.wireframe);
        self.grid = flag(args, "grid", self.grid);
        if let Some(l) = args["layer"].as_u64() {
            let top = self.sliced.as_ref().map_or(0, |s| s.layers.len());
            self.layer = (l as usize).clamp(1, top.max(1)) - 1;
        }
        Ok(Some(self.state()))
    }
}

/// Waits until the job has `samples`, or until `PHOTO_WAIT` passes.
/// Returns the samples in the frame.
fn finish_photo(job: &pathtrace::Job, samples: u32) -> Result<u32, String> {
    let until = Instant::now() + PHOTO_WAIT;
    loop {
        let n = job.frame.lock().unwrap().samples;
        if n >= samples || (n > 0 && Instant::now() >= until) {
            return Ok(n);
        }
        if Instant::now() >= until {
            return Err("The render made no sample in time. Use a smaller image.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
