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

/// How to run OpenSCAD. Saved in `$XDG_CONFIG_HOME/cocad/config`.
/// `$OPENSCAD` and `$OPENSCAD_ARGS` override the saved values.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Binary path or name. Empty means `openscad` on the PATH.
    pub binary: String,
    /// Extra arguments, split at whitespace.
    pub args: String,
    /// Add `--backend=manifold` when the binary supports it.
    pub manifold: bool,
    /// UI theme: "system", "dark" or "light".
    pub theme: String,
    /// Let MCP clients on this computer control the window.
    pub mcp: bool,
    /// The user accepted usage analytics.
    pub analytics: bool,
    /// The user accepted error reports.
    pub error_reports: bool,
    /// The user answered the privacy question.
    pub privacy_asked: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            binary: String::new(),
            args: String::new(),
            manifold: true,
            theme: "system".into(),
            mcp: true,
            analytics: false,
            error_reports: false,
            privacy_asked: false,
        }
    }
}

/// Per-user settings folder: `%APPDATA%\cocad` on Windows,
/// `~/Library/Application Support/cocad` on macOS, else
/// `$XDG_CONFIG_HOME/cocad` or `~/.config/cocad`.
pub fn config_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?
    };
    Some(base.join("cocad"))
}

fn config_path() -> Option<PathBuf> {
    Some(config_dir()?.join("config"))
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
                        "theme" => c.theme = v,
                        "mcp" => c.mcp = v != "false",
                        "analytics" => c.analytics = v == "true",
                        "error_reports" => c.error_reports = v == "true",
                        "privacy_asked" => c.privacy_asked = v == "true",
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
                "openscad = {}\nopenscad_args = {}\nmanifold = {}\ntheme = {}\nmcp = {}\n\
                 analytics = {}\nerror_reports = {}\nprivacy_asked = {}\n",
                self.binary,
                self.args,
                self.manifold,
                self.theme,
                self.mcp,
                self.analytics,
                self.error_reports,
                self.privacy_asked
            ),
        )?;
        Ok(())
    }

    /// The binary to run: `$OPENSCAD`, else the saved value, else the
    /// first OpenSCAD found (nightly first), else `openscad`.
    pub fn binary(&self) -> String {
        match std::env::var("OPENSCAD") {
            Ok(b) if !b.is_empty() => b,
            _ if !self.binary.trim().is_empty() => self.binary.trim().to_string(),
            _ => detected().to_string(),
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

/// Looks for OpenSCAD once: on the PATH, then in the usual install places.
fn detected() -> &'static str {
    static FOUND: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FOUND.get_or_init(|| {
        // On Windows openscad.com is the console build: it reports errors.
        let exts: &[&str] = if cfg!(windows) {
            &[".com", ".exe"]
        } else {
            &[""]
        };
        let paths: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        for name in ["openscad-nightly", "openscad"] {
            for dir in &paths {
                for ext in exts {
                    if dir.join(format!("{name}{ext}")).is_file() {
                        return format!("{name}{ext}");
                    }
                }
            }
        }
        let known: &[&str] = if cfg!(windows) {
            &[
                r"C:\Program Files\OpenSCAD (Nightly)\openscad.com",
                r"C:\Program Files\OpenSCAD\openscad.com",
                r"C:\Program Files\OpenSCAD (Nightly)\openscad.exe",
                r"C:\Program Files\OpenSCAD\openscad.exe",
            ]
        } else if cfg!(target_os = "macos") {
            &[
                "/Applications/OpenSCAD (Nightly).app/Contents/MacOS/OpenSCAD",
                "/Applications/OpenSCAD.app/Contents/MacOS/OpenSCAD",
                "/opt/homebrew/bin/openscad",
                "/usr/local/bin/openscad",
            ]
        } else {
            &[]
        };
        known
            .iter()
            .find(|p| Path::new(p).is_file())
            .map_or_else(|| "openscad".to_string(), |p| p.to_string())
    })
}

/// A command that does not open a console window on Windows.
fn command(binary: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(binary);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Absolute path without the Windows `\\?\` prefix, which OpenSCAD cannot read.
pub fn absolute(p: &Path) -> std::io::Result<PathBuf> {
    let full = p.canonicalize()?;
    let text = full.to_string_lossy();
    Ok(match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => full,
    })
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
        command(binary)
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
        "cocad-{}-{}.stl",
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

    let file = absolute(file)?;
    let binary = cfg.binary();
    let mut cmd = command(&binary);
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
        // Lead with OpenSCAD's own reason when there is one.
        let reason = log
            .lines()
            .find(|l| l.starts_with("ERROR") || l.contains("top level object"))
            .map(|l| l.trim().to_string())
            .unwrap_or_else(|| format!("OpenSCAD failed ({status})"));
        bail!("{reason}\n{log}");
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
