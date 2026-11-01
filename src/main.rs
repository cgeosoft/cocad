mod app;
mod export;
mod mesh;
mod openscad;
mod params;
mod render;
mod slicer;
mod ui;

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const USAGE: &str =
    "SCAD Viewer: view OpenSCAD models, edit Customizer parameters, export and preview slices.

Usage:
  scad-viewer [--slice] [FILE.scad]            Open the viewer (--slice: start in slice view)
  scad-viewer params FILE.scad                 List the Customizer parameters
  scad-viewer export FILE.scad -o OUT [-D name=value]...
                                               Render and write OUT (.stl, .3mf or .step)
  scad-viewer slice FILE.scad [-D name=value]... [--layer-height MM] [--infill PCT] [--walls N]
                                               Slice and print estimates

Environment:
  OPENSCAD       OpenSCAD binary (overrides the saved setting; default: openscad)
  OPENSCAD_ARGS  Extra OpenSCAD arguments (overrides the saved setting)

Settings are saved in $XDG_CONFIG_HOME/scad-viewer/config (Settings window in the app).
--backend=manifold is added when the OpenSCAD binary supports it.
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-h" | "--help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some("-V" | "--version") => {
            println!("scad-viewer {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("params") => cmd_params(&args[1..]),
        Some("export") => cmd_export(&args[1..]),
        Some("slice") => cmd_slice(&args[1..]),
        Some("--slice") => gui(args.get(1).map(PathBuf::from), true),
        Some(a) if a.starts_with('-') => bail!("unknown option {a}\n\n{USAGE}"),
        file => gui(file.map(PathBuf::from), false),
    }
}

fn gui(file: Option<PathBuf>, slice: bool) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0])
            .with_app_id("scad-viewer")
            .with_title("SCAD Viewer"),
        depth_buffer: 24,
        multisampling: 4,
        ..Default::default()
    };
    eframe::run_native(
        "SCAD Viewer",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, file, slice)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Parsed command line for the batch commands.
struct Batch {
    file: PathBuf,
    out: Option<PathBuf>,
    params: Vec<params::Param>,
    settings: slicer::Settings,
}

fn parse_batch(args: &[String]) -> Result<Batch> {
    let mut file = None;
    let mut out = None;
    let mut defs = Vec::new();
    let mut settings = slicer::Settings::default();
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
            "--layer-height" => settings.layer_height = val(a)?.parse()?,
            "--first-layer" => settings.first_layer = val(a)?.parse()?,
            "--line-width" => settings.line_width = val(a)?.parse()?,
            "--infill" => settings.infill = val(a)?.parse::<f32>()? / 100.0,
            "--walls" => settings.walls = val(a)?.parse()?,
            "--solid-layers" => settings.solid_layers = val(a)?.parse()?,
            "--speed" => settings.speed = val(a)?.parse()?,
            s if s.starts_with('-') => bail!("unknown option {s}"),
            s => file = Some(PathBuf::from(s)),
        }
    }
    let file = file.context("missing FILE.scad")?;
    let src = std::fs::read_to_string(&file)
        .with_context(|| format!("cannot read {}", file.display()))?;
    let mut params = params::parse(&src);
    for d in &defs {
        params::apply_define(&mut params, d)?;
    }
    Ok(Batch {
        file,
        out,
        params,
        settings,
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
    let mesh = render(&b)?;
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
    export::write(&out, &mesh, &meta)?;
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
