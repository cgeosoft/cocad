// Release builds on Windows open no console window; the CLI commands attach
// to the console that started them.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod examples;
mod export;
mod mcp;
mod mesh;
mod openscad;
mod params;
mod parts;
mod pathtrace;
mod presets;
mod recent;
mod render;
mod slicer;
mod telemetry;
mod ui;

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const USAGE: &str =
    "CoCAD: view OpenSCAD models, edit Customizer parameters, export and preview slices.

Usage:
  cocad [--slice|--photo] [FILE.scad]    Open the viewer (start in slice or photo view)
  cocad mcp                              Serve MCP on stdin and stdout, for Claude Desktop and
                                               other MCP clients. Controls the open window.
  cocad params FILE.scad                 List the Customizer parameters
  cocad export FILE.scad -o OUT [-P SET] [-D name=value]... [--for prusa|orca]
                                               Render and write OUT (.stl, .3mf or .step).
                                               --for writes a multicolor 3MF for that slicer.
  cocad slice FILE.scad [-P SET] [-D name=value]... [--layer-height MM] [--infill PCT] [--walls N]
                                               Slice and print estimates
  cocad photo FILE.scad -o OUT.png [-P SET] [-D name=value]... [--size WxH] [--samples N]
                    [--color RRGGBB] [--finish matte|satin|gloss] [--studio dark|light] [--layer-lines]
                                               Photo render (path traced) from the iso view

Parameter sets:
  -P SET      Apply a saved parameter set before the -D values
  -p FILE     Read sets from FILE (default: the file chosen in the app, else FILE.json)

Environment:
  OPENSCAD       OpenSCAD binary (overrides the saved setting; default: openscad)
  OPENSCAD_ARGS  Extra OpenSCAD arguments (overrides the saved setting)

Settings are saved in $XDG_CONFIG_HOME/cocad/config (Settings window in the app).
--backend=manifold is added when the OpenSCAD binary supports it.
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    if args.first().is_some_and(|a| {
        a.starts_with('-') && a != "--slice" && a != "--photo"
            || matches!(a.as_str(), "params" | "export" | "slice" | "photo")
    }) {
        attach_console();
    }
    match args.first().map(String::as_str) {
        Some("-h" | "--help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some("-V" | "--version") => {
            println!("cocad {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("mcp") => mcp::run(),
        Some("params") => cmd_params(&args[1..]),
        Some("export") => cmd_export(&args[1..]),
        Some("slice") => cmd_slice(&args[1..]),
        Some("photo") => cmd_photo(&args[1..]),
        Some(m @ ("--slice" | "--photo")) => gui(args.get(1).map(PathBuf::from), &m[2..]),
        Some(a) if a.starts_with('-') => bail!("unknown option {a}\n\n{USAGE}"),
        file => gui(file.map(PathBuf::from), "model"),
    }
}

fn gui(file: Option<PathBuf>, mode: &str) -> Result<()> {
    let mode = mode.to_string();
    telemetry::install_panic_hook();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0])
            .with_app_id("cocad")
            .with_title("CoCAD"),
        // Window position and size, panel widths: ~/.local/share/cocad.
        persist_window: true,
        depth_buffer: 24,
        multisampling: 4,
        ..Default::default()
    };
    eframe::run_native(
        "CoCAD",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, file, &mode)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

struct PhotoArgs {
    size: (usize, usize),
    samples: u32,
    color: u32,
    finish: String,
    studio: String,
    layer_lines: bool,
}

impl Default for PhotoArgs {
    fn default() -> Self {
        PhotoArgs {
            size: (1600, 1200),
            samples: 128,
            color: 0xff9300,
            finish: "satin".into(),
            studio: "dark".into(),
            layer_lines: false,
        }
    }
}

/// Parsed command line for the batch commands.
struct Batch {
    photo: PhotoArgs,
    file: PathBuf,
    out: Option<PathBuf>,
    params: Vec<params::Param>,
    settings: slicer::Settings,
    target: Option<export::Slicer>,
}

fn parse_batch(args: &[String]) -> Result<Batch> {
    let mut file = None;
    let mut out = None;
    let mut defs = Vec::new();
    let mut set = None;
    let mut set_file = None;
    let mut settings = slicer::Settings::default();
    let mut photo = PhotoArgs::default();
    let mut target = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| {
            it.next()
                .cloned()
                .with_context(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "-o" => out = Some(PathBuf::from(val("-o")?)),
            "-D" => defs.push(val("-D")?),
            "-P" => set = Some(val("-P")?),
            "-p" => set_file = Some(PathBuf::from(val("-p")?)),
            "--for" => {
                let v = val(a)?;
                target = Some(
                    export::Slicer::from_name(&v)
                        .with_context(|| format!("--for must be prusa or orca, got {v}"))?,
                );
            }
            "--layer-height" => settings.layer_height = val(a)?.parse()?,
            "--first-layer" => settings.first_layer = val(a)?.parse()?,
            "--line-width" => settings.line_width = val(a)?.parse()?,
            "--infill" => settings.infill = val(a)?.parse::<f32>()? / 100.0,
            "--walls" => settings.walls = val(a)?.parse()?,
            "--solid-layers" => settings.solid_layers = val(a)?.parse()?,
            "--speed" => settings.speed = val(a)?.parse()?,
            "--size" => {
                let v = val(a)?;
                let (w, h) = v.split_once('x').context("--size needs WxH")?;
                photo.size = (w.parse()?, h.parse()?);
            }
            "--samples" => photo.samples = val(a)?.parse()?,
            "--color" => photo.color = u32::from_str_radix(val(a)?.trim_start_matches('#'), 16)?,
            "--finish" => photo.finish = val(a)?,
            "--studio" => photo.studio = val(a)?,
            "--layer-lines" => photo.layer_lines = true,
            s if s.starts_with('-') => bail!("unknown option {s}"),
            s => file = Some(PathBuf::from(s)),
        }
    }
    let file = file.context("missing FILE.scad")?;
    let src = std::fs::read_to_string(&file)
        .with_context(|| format!("cannot read {}", file.display()))?;
    let mut params = params::parse(&src);
    if let Some(name) = &set {
        let path = set_file.unwrap_or_else(|| presets::file_for(&file));
        for k in presets::apply(&path, name, &mut params)? {
            eprintln!("warning: set '{name}': skipped '{k}'");
        }
    }
    for d in &defs {
        params::apply_define(&mut params, d)?;
    }
    Ok(Batch {
        photo,
        file,
        out,
        params,
        settings,
        target,
    })
}

fn render(b: &Batch) -> Result<mesh::Mesh> {
    let cancel = Arc::new(AtomicBool::new(false));
    let out = openscad::render(
        &openscad::Config::load(),
        &b.file,
        &params::defines(&b.params),
        &cancel,
    )?;
    let bb = out.mesh.bounds().unwrap();
    let s = bb.size();
    eprintln!(
        "rendered {} triangles, {:.2} x {:.2} x {:.2} mm in {:.2} s",
        out.mesh.triangles.len(),
        s.x,
        s.y,
        s.z,
        out.elapsed.as_secs_f32()
    );
    Ok(out.mesh)
}

fn cmd_params(args: &[String]) -> Result<()> {
    let b = parse_batch(args)?;
    let mut group = String::new();
    for p in &b.params {
        if p.group != group {
            group = p.group.clone();
            println!("[{group}]");
        }
        let widget = match &p.widget {
            params::Widget::Plain => String::new(),
            params::Widget::Slider { min, max, step } => match step {
                Some(s) => format!("  [{min}:{s}:{max}]"),
                None => format!("  [{min}:{max}]"),
            },
            params::Widget::Dropdown(o) => {
                format!(
                    "  [{}]",
                    o.iter()
                        .map(|(v, l)| format!("{}:{l}", v.to_scad()))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            params::Widget::TextMax(n) => format!("  [max {n}]"),
        };
        let desc = if p.description.is_empty() {
            String::new()
        } else {
            format!("  // {}", p.description)
        };
        println!("  {} = {}{widget}{desc}", p.name, p.value.to_scad());
    }
    Ok(())
}

fn cmd_export(args: &[String]) -> Result<()> {
    let b = parse_batch(args)?;
    let out = b.out.clone().context("missing -o OUT")?;
    export::Format::from_path(&out).context("OUT must end in .stl, .3mf or .step")?;
    let title = b
        .file
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let meta = export::Meta {
        title,
        params: b
            .params
            .iter()
            .map(|p| (p.name.clone(), p.value.to_scad()))
            .collect(),
    };
    if let Some(target) = b.target {
        let cancel = Arc::new(AtomicBool::new(false));
        let parts = parts::render(&openscad::Config::load(), &b.file, &b.params, &cancel)?;
        export::write_parts(&out, &parts, &meta, target)?;
        for (i, p) in parts.iter().enumerate() {
            eprintln!(
                "filament {}: {} ({} triangles)",
                i + 1,
                p.name,
                p.mesh.triangles.len()
            );
        }
    } else {
        export::write(&out, &render(&b)?, &meta)?;
    }
    eprintln!("wrote {}", out.display());
    Ok(())
}

fn cmd_slice(args: &[String]) -> Result<()> {
    let b = parse_batch(args)?;
    let mesh = render(&b)?;
    let t = std::time::Instant::now();
    let r = slicer::slice(&mesh, &b.settings);
    let st = &r.stats;
    println!("layers      {}", st.layers);
    println!("path        {:.1} m", st.path_mm / 1000.0);
    println!("filament    {:.2} m, {:.1} g", st.filament_m, st.grams);
    println!("print time  {:.0} min (rough)", st.seconds / 60.0);
    println!("sliced in   {} ms", t.elapsed().as_millis());
    Ok(())
}

fn cmd_photo(args: &[String]) -> Result<()> {
    let b = parse_batch(args)?;
    let out = b.out.clone().context("missing -o OUT.png")?;
    let mesh = std::sync::Arc::new(render(&b)?);
    let p = &b.photo;
    let look = pathtrace::Look {
        color: pathtrace::srgb(p.color),
        roughness: pathtrace::finish_roughness(&p.finish),
        layer_lines: if p.layer_lines {
            b.settings.layer_height
        } else {
            0.0
        },
        studio: if p.studio == "light" {
            pathtrace::Studio::Light
        } else {
            pathtrace::Studio::Dark
        },
        exposure: 1.0,
    };
    let cam = pathtrace::iso_camera(&mesh, p.size.0 as f32 / p.size.1 as f32);
    let t = std::time::Instant::now();
    let job = pathtrace::Job::start(mesh, cam, look, p.size.0, p.size.1, p.samples);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let f = job.frame.lock().unwrap();
        if f.samples >= p.samples {
            pathtrace::save_png(&out, &f)?;
            break;
        }
    }
    eprintln!(
        "{} samples in {:.1} s, wrote {}",
        p.samples,
        t.elapsed().as_secs_f32(),
        out.display()
    );
    Ok(())
}

#[cfg(windows)]
fn attach_console() {
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    // SAFETY: plain Win32 call without pointers; failure only means no console.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
