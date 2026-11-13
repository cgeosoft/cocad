//! Model thumbnails: the iso view of the model, made when the model opens
//! (after the first build) and when it closes. See `crate::thumbs`. The
//! start page shows them with the title and the description of the model.

use super::{App, Camera, VIEWS};
use crate::assembly::Assembly;
use crate::render::{PartStyle, Renderer, View};
use crate::thumbs;
use eframe::egui;
use eframe::glow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// The part of the square, from the center to the edge, that the model fills.
const THUMB_FILL: f32 = 0.88;

/// A thumbnail to draw: the model file, its mesh and the part styles.
pub struct Job {
    scad: PathBuf,
    assembly: Arc<Assembly>,
    parts: Vec<PartStyle>,
}

impl Job {
    /// The iso view of the whole model in a square.
    fn view(&self) -> Option<View> {
        let b = self.assembly.bounds(&[])?;
        let (_, yaw, pitch) = VIEWS[0];
        let mut cam = Camera {
            yaw,
            pitch,
            dist: 1.0,
            target: b.center(),
            ortho: false,
        };
        cam.fit(&b);
        // The fit holds the bounding sphere. Move closer until the corners
        // of the box fill the square with a small margin.
        for _ in 0..3 {
            let m = cam.matrix(1.0);
            let mut extent = 0.0f32;
            for i in 0..8 {
                let pick = |bit: usize, lo: f32, hi: f32| if i & bit == 0 { lo } else { hi };
                let c = glam::Vec3::new(
                    pick(1, b.min.x, b.max.x),
                    pick(2, b.min.y, b.max.y),
                    pick(4, b.min.z, b.max.z),
                );
                let p = m.project_point3(c);
                extent = extent.max(p.x.abs()).max(p.y.abs());
            }
            if !extent.is_finite() || extent <= 0.0 {
                break;
            }
            cam.dist *= extent / THUMB_FILL;
        }
        Some(View {
            mvp: cam.matrix(1.0),
            eye: cam.eye(),
            show_mesh: true,
            mesh_alpha: 1.0,
            parts: self.parts.clone(),
            wireframe: false,
            grid: false,
            layer: None,
            section: Default::default(),
        })
    }

    /// Draws the thumbnail. Returns the RGBA pixels of `thumbs::SIZE`.
    fn draw(&self, gl: &glow::Context, r: &mut Renderer) -> Option<Vec<u8>> {
        let view = self.view()?;
        let big = thumbs::SIZE * thumbs::SUPERSAMPLE;
        let px = r.snapshot(gl, &self.assembly, &view, big as i32)?;
        Some(thumbs::shrink(&px, big, thumbs::SUPERSAMPLE))
    }
}

fn save(scad: &Path, px: &[u8]) {
    if let Err(e) = thumbs::save(scad, px, thumbs::SIZE) {
        eprintln!("thumbnail of {}: {e}", scad.display());
    }
}

impl App {
    /// A thumbnail job for the open model, when it has a mesh.
    fn thumb_job(&self) -> Option<Job> {
        let scad = self.file.clone()?;
        let assembly = self.assembly.clone()?;
        if assembly.mesh.triangles.is_empty() {
            return None;
        }
        let parts = self.view3d.styles(&assembly.parts, false);
        Some(Job {
            scad,
            assembly,
            parts,
        })
    }

    /// Queues a thumbnail of the open model for the next frame.
    pub(super) fn queue_thumb(&mut self) {
        self.thumb_due = false;
        if let Some(job) = self.thumb_job() {
            self.thumb_jobs.retain(|j| j.scad != job.scad);
            self.thumb_jobs.push(job);
        }
    }

    /// Sends the queued thumbnails to the renderer. The first build of a
    /// model that opened adds one, when the build and the part search end.
    pub(super) fn send_thumbs(&mut self, ctx: &egui::Context) {
        if self.thumb_due
            && self.job.is_none()
            && self.parts_job.is_none()
            && self.assembly.is_some()
        {
            self.queue_thumb();
        }
        if self.thumb_jobs.is_empty() {
            return;
        }
        let Some(r) = self.renderer.clone() else {
            self.thumb_jobs.clear();
            return;
        };
        let jobs = std::mem::take(&mut self.thumb_jobs);
        let cb = egui::PaintCallback {
            rect: ctx.screen_rect(),
            callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |_info, painter| {
                let mut r = r.lock().unwrap();
                for job in &jobs {
                    if let Some(px) = job.draw(painter.gl(), &mut r) {
                        let scad = job.scad.clone();
                        std::thread::spawn(move || save(&scad, &px));
                    }
                }
            })),
        };
        ctx.layer_painter(egui::LayerId::background()).add(cb);
        ctx.request_repaint();
    }

    /// Draws and writes the queued thumbnails now. For the exit, when no
    /// frame follows.
    pub(super) fn draw_thumbs(&mut self, gl: &glow::Context) {
        let Some(r) = &self.renderer else { return };
        let mut r = r.lock().unwrap();
        for job in self.thumb_jobs.drain(..) {
            if let Some(px) = job.draw(gl, &mut r) {
                save(&job.scad, &px);
            }
        }
    }
}

/// What the start page shows for one model.
struct Entry {
    thumb_mtime: Option<SystemTime>,
    texture: Option<egui::TextureHandle>,
    def_key: Option<(PathBuf, Option<SystemTime>)>,
    title: Option<String>,
    description: Option<String>,
}

/// The thumbnails and the titles of the models on the start page. It reads
/// the files again when they change, at most once each second.
#[derive(Default)]
pub struct StartCache {
    entries: HashMap<PathBuf, Entry>,
    checked: HashMap<PathBuf, Instant>,
}

/// The text and the picture of one model on the start page.
pub struct Card<'a> {
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    pub texture: Option<&'a egui::TextureHandle>,
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl StartCache {
    /// The card of the model `scad`. Without a thumbnail file, the picture
    /// is `fallback`, a PNG.
    pub fn card(&mut self, ctx: &egui::Context, scad: &Path, fallback: Option<&[u8]>) -> Card<'_> {
        let now = Instant::now();
        let fresh = self
            .checked
            .get(scad)
            .is_some_and(|t| now.duration_since(*t) < Duration::from_secs(1));
        if !fresh {
            self.checked.insert(scad.to_path_buf(), now);
            self.refresh(ctx, scad, fallback);
        }
        match self.entries.get(scad) {
            Some(e) => Card {
                title: e.title.as_deref(),
                description: e.description.as_deref(),
                texture: e.texture.as_ref(),
            },
            None => Card {
                title: None,
                description: None,
                texture: None,
            },
        }
    }

    fn refresh(&mut self, ctx: &egui::Context, scad: &Path, fallback: Option<&[u8]>) {
        let e = self.entries.entry(scad.to_path_buf()).or_insert(Entry {
            thumb_mtime: None,
            texture: None,
            def_key: None,
            title: None,
            description: None,
        });
        let file = thumbs::file_for(scad);
        let m = file.as_deref().and_then(mtime);
        if m != e.thumb_mtime || e.texture.is_none() {
            e.thumb_mtime = m;
            e.texture = file
                .as_deref()
                .filter(|_| m.is_some())
                .and_then(thumbs::load)
                .or_else(|| thumbs::decode(fallback?))
                .map(|(w, h, px)| {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &px);
                    let name = format!("thumb:{}", scad.display());
                    ctx.load_texture(name, img, egui::TextureOptions::LINEAR)
                });
        }
        let def = crate::definition::path_for(scad);
        let source = crate::definition::source(&def).unwrap_or(def.clone());
        let key = (source.clone(), mtime(&source));
        if e.def_key.as_ref() != Some(&key) {
            let meta = crate::definition::load(&def).unwrap_or_default().meta;
            e.title = meta.title;
            e.description = meta.description;
            e.def_key = Some(key);
        }
    }
}
