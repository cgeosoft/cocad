//! Runs the OpenSCAD CLI to turn a .scad file and parameters into a mesh.

use crate::mesh::{Mesh, read_stl};
use anyhow::{Result, bail};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct RenderOutput {
    pub mesh: Mesh,
    pub log: String,
    pub elapsed: Duration,
}

/// How to run OpenSCAD. Saved in `$XDG_CONFIG_HOME/scad-viewer/config`.
/// `$OPENSCAD` and `$OPENSCAD_ARGS` override the saved values.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Binary path or name. Empty means `openscad` on the PATH.
    pub binary: String,
    /// Extra arguments, split at whitespace.
    pub args: String,
    /// Add `--backend=manifold` when the binary supports it.
    pub manifold: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            binary: String::new(),
            args: String::new(),
            manifold: true,
        }
    }
}

/// `$XDG_CONFIG_HOME/scad-viewer`, else `~/.config/scad-viewer`.
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("scad-viewer"))
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("scad-viewer").join("config"))
}

impl Config {
    pub fn load() -> Config {
        let mut c = Config::default();
        if let Some(text) = config_path().and_then(|p| std::fs::read_to_string(p).ok()) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    let v = v.trim().to_string();
                    match k.trim() {
                        "openscad" => c.binary = v,
                        "openscad_args" => c.args = v,
                        "manifold" => c.manifold = v != "false",
                        _ => {}
                    }
                }
            }
        }
        c
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path().ok_or_else(|| anyhow::anyhow!("no config folder"))?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(
            &path,
            format!(
                "openscad = {}\nopenscad_args = {}\nmanifold = {}\n",
                self.binary, self.args, self.manifold
            ),
        )?;
        Ok(())
    }

    /// The binary to run: `$OPENSCAD`, else the saved value, else
    /// `openscad-nightly` when it is on the PATH, else `openscad`.
    pub fn binary(&self) -> String {
        match std::env::var("OPENSCAD") {
            Ok(b) if !b.is_empty() => b,
            _ if !self.binary.trim().is_empty() => self.binary.trim().to_string(),
            _ if on_path("openscad-nightly") => "openscad-nightly".into(),
            _ => "openscad".into(),
        }
    }

    /// All extra arguments, with `--backend=manifold` added when it applies.
    pub fn args(&self) -> Vec<String> {
        let extra = std::env::var("OPENSCAD_ARGS").unwrap_or_else(|_| self.args.clone());
        let mut args: Vec<String> = extra.split_whitespace().map(String::from).collect();
        if self.manifold
            && !args.iter().any(|a| a.starts_with("--backend"))
            && probe(&self.binary()).manifold
        {
            args.push("--backend=manifold".into());
        }
        args
    }
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(name).is_file()))
}

#[derive(Clone, Debug, Default)]
pub struct Probe {
    pub version: String,
    pub manifold: bool,
}

/// Version and features of an OpenSCAD binary. Cached per binary.
pub fn probe(binary: &str) -> Probe {
    static CACHE: Mutex<Option<HashMap<String, Probe>>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    let map = cache.get_or_insert_with(HashMap::new);
    if let Some(p) = map.get(binary) {
        return p.clone();
    }
    let run = |arg: &str| {
        Command::new(binary)
            .arg(arg)
            .stdin(Stdio::null())
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout).into_owned()
                    + &String::from_utf8_lossy(&o.stderr)
            })
            .unwrap_or_default()
    };
    let version = run("--version")
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let help = run("--help");
    let p = Probe {
        version: if version.is_empty() {
            "not found".into()
        } else {
            version
        },
        manifold: help.contains("--backend") && help.to_lowercase().contains("manifold"),
    };
    map.insert(binary.to_string(), p.clone());
    p
}

fn temp_stl() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "scad-viewer-{}-{}.stl",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Renders `file` with `-D` overrides. Returns early with an error when
/// `cancel` becomes true; the OpenSCAD process is killed then.
pub fn render(
    cfg: &Config,
    file: &Path,
    defines: &[String],
    cancel: &Arc<AtomicBool>,
) -> Result<RenderOutput> {
    let start = Instant::now();
    let out = temp_stl();
    let log_path = out.with_extension("log");
    let log_file = std::fs::File::create(&log_path)?;

    let file = file.canonicalize()?;
    let binary = cfg.binary();
    let mut cmd = Command::new(&binary);
    cmd.args(cfg.args())
        .arg("-o")
        .arg(&out)
        .arg("--export-format")
        .arg("binstl");
    for d in defines {
        cmd.arg("-D").arg(d);
    }
    cmd.arg(&file)
        .stdin(Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file);
    // Relative include<> and import() paths resolve from the file's folder.
    if let Some(dir) = file.parent() {
        cmd.current_dir(dir);
    }

    let mut child = cmd.spawn().map_err(|e| {
        anyhow::anyhow!(
            "cannot start '{binary}': {e}. Install OpenSCAD or set the binary in Settings."
        )
    })?;

    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&out);
            let _ = std::fs::remove_file(&log_path);
            bail!("cancelled");
        }
        if let Some(s) = child.try_wait()? {
            break s;
        }
        std::thread::sleep(Duration::from_millis(15));
    };

    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let _ = std::fs::remove_file(&log_path);
    let data = std::fs::read(&out);
    let _ = std::fs::remove_file(&out);

    if !status.success() {
        bail!("OpenSCAD failed ({status})\n{log}");
    }
    let data = match data {
        Ok(d) => d,
        Err(_) => bail!("OpenSCAD wrote no geometry. Is the top level object empty or 2D?\n{log}"),
    };
    let mesh = read_stl(&data)?;
    if mesh.is_empty() {
        bail!("The model has no 3D geometry.\n{log}");
    }
    Ok(RenderOutput {
        mesh,
        log,
        elapsed: start.elapsed(),
    })
}
