// Release builds on Windows open no console window; the CLI commands attach
// to the console that started them.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod assembly;
mod csg_split;
mod examples;
mod export;
mod mcp;
mod mesh;
mod net;
mod openscad;
mod optimize;
mod params;
mod parts;
mod pathtrace;
mod presets;
mod printer;
mod recent;
mod render;
mod slicer;
mod telemetry;
mod ui;
mod update;

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const USAGE: &str =
    "CoCAD: view OpenSCAD models, edit Customizer parameters, export and preview slices.

Usage:
  cocad [--optimize|--render] [FILE.scad]
                                               Open the viewer (start in the Optimize or
                                               Render tab; --slice and --photo also work)
  cocad mcp                              Serve MCP on stdin and stdout, for Claude Desktop and
                                               other MCP clients. Controls the open window.
  cocad params FILE.scad                 List the Customizer parameters
  cocad export FILE.scad -o OUT [-P SET] [-D name=value]... [PRINT OPTIONS] [--for prusa|orca]
                                               Build and write OUT (.stl, .3mf or .step).
                                               STL and 3MF get the print plate, STEP gets
                                               the model as built.
                                               --for writes a multicolor 3MF for that slicer.
  cocad slice FILE.scad [-P SET] [-D name=value]... [PRINT OPTIONS]
                    [--layer-height MM] [--first-layer MM] [--infill PCT] [--walls N]
                    [--line-width MM] [--solid-layers N] [--speed MM/S]
                                               Slice the plates and print estimates
  cocad photo FILE.scad -o OUT.png [-P SET] [-D name=value]... [--size WxH] [--samples N]
                    [--color RRGGBB] [--finish matte|satin|gloss] [--studio dark|light] [--layer-lines]
                                               Photo render (path traced) from the iso view

Parameter sets:
  -P SET      Apply a saved parameter set before the -D values
  -p FILE     Read sets from FILE (default: the file chosen in the app, else FILE.json)

Print options (export and slice):
  CoCAD starts from the Optimize values that the app stored for the model, else
  from the defaults (all parts, Auto orientation, all fixes off). The options change them.
  --plate N|all        The plate to export or slice (default: export plate 1, slice
                       all plates). all: export one file per plate, OUT-plateN.ext
  --part NAME|all      Only this print part, or all print parts
  --bed WxDxH          Print area in mm (default: the stored value, else Settings > Printer)
  --orient model|auto  Orientation of all parts: the model rotation, or the best face
  --chamfer[=MM]       Bed chamfer (0.4 mm by default; 0 turns it off)
  --teardrops          Teardrop holes
  --bridges[=sacrificial|stepped]
                       Bridge fixes, with the fix for a hole above a bridge
  --brim               Brim on small bed islands
  --no-fixes           All stored fixes off. The fix options above still turn a fix on.
  --as-built           The model as built, without the print preparation (as before 0.7)

Environment:
  OPENSCAD       OpenSCAD binary (overrides the saved setting; default: openscad)
  OPENSCAD_ARGS  Extra OpenSCAD arguments (overrides the saved setting)

Settings are saved in $XDG_CONFIG_HOME/cocad/config (Settings window in the app).
CoCAD needs an OpenSCAD development snapshot with the Manifold backend, and adds
--backend=manifold. Get one at https://openscad.org/downloads.html#snapshots
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    if args.first().is_some_and(|a| {
        a.starts_with('-') && start_tab(a).is_none()
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
        // `params` does not run OpenSCAD, so it needs no check.
        Some("params") => cmd_params(&args[1..]),
        Some(cmd @ ("mcp" | "export" | "slice" | "photo")) => {
            // These stop when OpenSCAD has no Manifold. The window shows a
            // notice instead.
            openscad::check(&openscad::Config::load())?;
            match cmd {
                "mcp" => mcp::run(),
                "export" => cmd_export(&args[1..]),
                "slice" => cmd_slice(&args[1..]),
                _ => cmd_photo(&args[1..]),
            }
        }
        Some(a) if start_tab(a).is_some() => gui(
            args.get(1).map(PathBuf::from),
            start_tab(a).unwrap_or("model"),
        ),
        Some(a) if a.starts_with('-') => bail!("unknown option {a}\n\n{USAGE}"),
        file => gui(file.map(PathBuf::from), "model"),
    }
}

/// The tab of a start flag. `--slice` and `--photo` are the names before 0.7.
fn start_tab(flag: &str) -> Option<&'static str> {
    match flag {
        "--optimize" | "--slice" => Some("optimize"),
        "--render" | "--photo" => Some("render"),
        _ => None,
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
        // The cut faces of the section (render.rs).
        stencil_buffer: 8,
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
    /// The print options of export and slice.
    print: optimize::headless::Options,
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
    let mut print = optimize::headless::Options::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| {
            it.next()
                .cloned()
                .with_context(|| format!("{name} needs a value"))
        };
        if print.parse_flag(a, || val(a))? {
            continue;
        }
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
        print,
    })
}

fn build(b: &Batch) -> Result<mesh::Mesh> {
    let cancel = Arc::new(AtomicBool::new(false));
    let out = openscad::render(
        &openscad::Config::load(),
        &b.file,
        &params::defines(&b.params),
        &cancel,
    )?;
    built_line(&out.mesh, out.elapsed);
    Ok(out.mesh)
}

/// The first line on stderr after the build. `scripts/test-models.sh`
/// reads the triangle count from it.
fn built_line(mesh: &mesh::Mesh, elapsed: std::time::Duration) {
    let s = optimize::bounds_or_zero(mesh).size();
    eprintln!(
        "built {} triangles, {:.2} x {:.2} x {:.2} mm in {:.2} s",
        mesh.triangles.len(),
        s.x,
        s.y,
        s.z,
        elapsed.as_secs_f32()
    );
}

/// Builds the model and finds its printable parts (see
/// `optimize::headless::inputs`).
fn build_parts(b: &Batch) -> Result<optimize::headless::Built> {
    let cancel = Arc::new(AtomicBool::new(false));
    let built = optimize::headless::inputs(&openscad::Config::load(), &b.file, &b.params, &cancel)?;
    built_line(&built.mesh, built.elapsed);
    Ok(built)
}

/// Builds the model, runs the print preparation with the print options and
/// prints the report on stderr.
fn prepare(b: &Batch) -> Result<(optimize::headless::Built, optimize::headless::Prepared)> {
    let built = build_parts(b)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let layers = optimize::Layers {
        first: b.settings.first_layer,
        height: b.settings.layer_height,
    };
    let p = optimize::headless::optimize(
        &openscad::Config::load(),
        &b.file,
        &built,
        &b.print,
        layers,
        &cancel,
    )?;
    eprint!("{}", optimize::headless::report(&built, &p));
    Ok((built, p))
}

/// The plates to write or slice: `--plate`, else `default`. Fails when
/// the parts are on no plate, or the plate does not exist.
fn plates(b: &Batch, o: &optimize::pipeline::Outcome, all_by_default: bool) -> Result<Vec<usize>> {
    use optimize::headless::Plates;
    if o.plates == 0 {
        bail!("Nothing to print: no print part is on a plate. See the report above.");
    }
    Ok(match b.print.plate {
        Some(Plates::One(n)) if n >= o.plates => bail!(
            "--plate {}: the parts are on {} plate{}",
            n + 1,
            o.plates,
            if o.plates == 1 { "" } else { "s" }
        ),
        Some(Plates::One(n)) => vec![n],
        Some(Plates::All) => (0..o.plates).collect(),
        None if all_by_default => (0..o.plates).collect(),
        None => vec![0],
    })
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
    let fmt = export::Format::from_path(&out).context("OUT must end in .stl, .3mf or .step")?;
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
    let opts = &b.print;
    if let Some(target) = b.target {
        if opts.changes_print() || opts.part.is_some() || opts.as_built {
            eprintln!(
                "warning: --for writes all parts of the model as built. The print options do nothing."
            );
        }
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
        eprintln!("wrote {}", out.display());
        return Ok(());
    }
    if fmt == export::Format::Step || opts.as_built {
        // The model as built, or one part of it in its model position.
        if opts.changes_print() {
            let why = if opts.as_built {
                "--as-built writes the model as built"
            } else {
                "STEP gets the model as built"
            };
            eprintln!("warning: {why}. The print options do nothing.");
        }
        let mesh = match opts.only() {
            None => build(&b)?,
            Some(name) => model_part(&b, name)?,
        };
        export::write(&out, &mesh, &meta)?;
        eprintln!("wrote {}", out.display());
        return Ok(());
    }
    let (_, p) = prepare(&b)?;
    let o = &p.outcome;
    let list = plates(&b, o, false)?;
    let all = opts.plate == Some(optimize::headless::Plates::All);
    for &plate in &list {
        let path = if all {
            let name = out.file_name().unwrap_or_default().to_string_lossy();
            out.with_file_name(export::plate_name(&name, plate))
        } else {
            out.clone()
        };
        match fmt {
            export::Format::ThreeMf => {
                let bytes = optimize::headless::plate_3mf(o, plate, &meta)?;
                std::fs::write(&path, bytes)
                    .with_context(|| format!("cannot write {}", path.display()))?;
            }
            _ => export::write(&path, &o.plate_mesh(plate), &meta)?,
        }
        eprintln!(
            "wrote {} (plate {} of {})",
            path.display(),
            plate + 1,
            o.plates
        );
    }
    if o.plates > 1 && !all && opts.plate.is_none() {
        eprintln!(
            "note: the parts are on {} plates. Use --plate N or --plate all for the others.",
            o.plates
        );
    }
    Ok(())
}

/// The part `name` of the model, in its model position. For STEP and
/// `--as-built` with `--part`.
fn model_part(b: &Batch, name: &str) -> Result<mesh::Mesh> {
    let built = build_parts(b)?;
    if let Some(p) = built.parts.iter().find(|p| p.name == name) {
        return Ok(p.mesh.as_ref().clone());
    }
    let names: Vec<&str> = built.parts.iter().map(|p| p.name.as_str()).collect();
    bail!(
        "The model has no part with the name '{name}'.\nThe parts: {}",
        names.join(", ")
    )
}

/// Prints the slice estimate: the layers, the path, the filament and the
/// print time.
fn print_stats(st: &slicer::Stats, plates: usize) {
    let all = if plates > 1 {
        format!("  (all {plates} plates)")
    } else {
        String::new()
    };
    println!("layers      {}{all}", st.layers);
    println!("path        {:.1} m", st.path_mm / 1000.0);
    println!("filament    {:.2} m, {:.1} g", st.filament_m, st.grams);
    println!("print time  {:.0} min (rough)", st.seconds / 60.0);
}

fn cmd_slice(args: &[String]) -> Result<()> {
    let b = parse_batch(args)?;
    if b.print.as_built {
        if b.print.changes_print() || b.print.part.is_some() {
            eprintln!(
                "warning: --as-built slices the model as built. The print options do nothing."
            );
        }
        let mesh = build(&b)?;
        let t = std::time::Instant::now();
        let r = slicer::slice(&mesh, &b.settings);
        print_stats(&r.stats, 1);
        println!("sliced in   {} ms", t.elapsed().as_millis());
        return Ok(());
    }
    let (_, p) = prepare(&b)?;
    let o = &p.outcome;
    let list = plates(&b, o, true)?;
    let t = std::time::Instant::now();
    let mut total = slicer::Stats::default();
    for &plate in &list {
        let st = slicer::slice(&o.plate_mesh(plate), &b.settings).stats;
        if list.len() > 1 {
            println!(
                "{:<12}{} layers, {:.2} m, {:.1} g, {:.0} min",
                format!("plate {}", plate + 1),
                st.layers,
                st.filament_m,
                st.grams,
                st.seconds / 60.0
            );
        }
        total.layers += st.layers;
        total.path_mm += st.path_mm;
        total.filament_m += st.filament_m;
        total.grams += st.grams;
        total.seconds += st.seconds;
    }
    print_stats(&total, list.len());
    println!("sliced in   {} ms", t.elapsed().as_millis());
    Ok(())
}

fn cmd_photo(args: &[String]) -> Result<()> {
    let b = parse_batch(args)?;
    let out = b.out.clone().context("missing -o OUT.png")?;
    let mesh = std::sync::Arc::new(build(&b)?);
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
