//! Runs the OpenSCAD CLI to turn a .scad file and parameters into a mesh.
//!
//! CoCAD needs an OpenSCAD build with the Manifold backend: today a
//! development snapshot (nightly). `check` tells when the binary has no
//! Manifold, and `install_steps` tells how to get one.

use crate::mesh::{Mesh, read_stl};
use anyhow::{Result, bail};
use std::collections::{BTreeSet, HashMap};
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
    /// UI theme: "system", "dark" or "light".
    pub theme: String,
    /// Let MCP clients on this computer control the window.
    pub mcp: bool,
    /// Ask GitHub for new versions.
    pub update_check: bool,
    /// Save the values to the active parameter set when they change.
    pub autosave_sets: bool,
    /// The cards of the right panel that the user closed, by title. One
    /// choice for all models.
    pub closed_cards: BTreeSet<String>,
    /// The default print area (Settings > Printer), for a model that has
    /// no print area of its own yet.
    pub bed: crate::optimize::Area,
    /// The printer preset that the user selected for `bed`, so that the
    /// dropdown shows it when two presets have the same area. Empty: none.
    pub bed_preset: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            binary: String::new(),
            args: String::new(),
            theme: "system".into(),
            mcp: true,
            update_check: true,
            autosave_sets: false,
            closed_cards: BTreeSet::new(),
            bed: crate::optimize::Area::default(),
            bed_preset: String::new(),
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
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(Config::default, |text| Config::parse(&text))
    }

    /// The settings in the text of a config file. Unknown keys and keys of
    /// older versions do nothing, for example `manifold` (before 0.7).
    pub fn parse(text: &str) -> Config {
        let mut c = Config::default();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                let v = v.trim().to_string();
                match k.trim() {
                    "openscad" => c.binary = v,
                    "openscad_args" => c.args = v,
                    "theme" => c.theme = v,
                    "mcp" => c.mcp = v != "false",
                    "update_check" => c.update_check = v != "false",
                    "autosave_sets" => c.autosave_sets = v == "true",
                    "bed_preset" => c.bed_preset = v,
                    "bed_w" | "bed_d" | "bed_h" => {
                        let ok =
                            |x: &f32| (crate::printer::MIN_MM..=crate::printer::MAX_MM).contains(x);
                        if let Some(mm) = v.parse::<f32>().ok().filter(ok) {
                            match k.trim() {
                                "bed_w" => c.bed.w = mm,
                                "bed_d" => c.bed.d = mm,
                                _ => c.bed.h = mm,
                            }
                        }
                    }
                    "closed_cards" => {
                        c.closed_cards = (v.split(','))
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .collect();
                    }
                    _ => {}
                }
            }
        }
        c
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path().ok_or_else(|| anyhow::anyhow!("no config folder"))?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, self.text())?;
        Ok(())
    }

    /// Consent is not here: see `telemetry::set_consent`. A save drops the
    /// consent keys of older versions.
    pub(crate) fn text(&self) -> String {
        let closed: Vec<&str> = self.closed_cards.iter().map(String::as_str).collect();
        format!(
            "openscad = {}\nopenscad_args = {}\ntheme = {}\nmcp = {}\nupdate_check = {}\n\
             autosave_sets = {}\nclosed_cards = {}\nbed_w = {}\nbed_d = {}\nbed_h = {}\n\
             bed_preset = {}\n",
            self.binary,
            self.args,
            self.theme,
            self.mcp,
            self.update_check,
            self.autosave_sets,
            closed.join(","),
            self.bed.w,
            self.bed.d,
            self.bed.h,
            self.bed_preset.trim()
        )
    }

    /// The binary to run: `$OPENSCAD`, else the saved value, else the
    /// first OpenSCAD found with Manifold (nightly first), else the first
    /// OpenSCAD found, else `openscad`.
    pub fn binary(&self) -> String {
        match std::env::var("OPENSCAD") {
            Ok(b) if !b.is_empty() => b,
            _ if !self.binary.trim().is_empty() => self.binary.trim().to_string(),
            _ => detected(),
        }
    }

    /// All extra arguments, with `--backend=manifold` added. A `--backend`
    /// argument of the user stays and replaces it (see `cgal_arg`).
    pub fn args(&self) -> Vec<String> {
        let extra = std::env::var("OPENSCAD_ARGS").unwrap_or_else(|_| self.args.clone());
        with_backend(&extra)
    }
}

/// The arguments in `extra`, with `--backend=manifold` added when `extra`
/// has no `--backend` argument.
fn with_backend(extra: &str) -> Vec<String> {
    let mut args: Vec<String> = extra.split_whitespace().map(String::from).collect();
    if !args.iter().any(|a| a.starts_with("--backend")) {
        args.push("--backend=manifold".into());
    }
    args
}

/// True when the extra arguments `args` select the CGAL backend, as
/// `--backend=cgal` or `--backend cgal`. The settings then show a warning.
pub fn cgal_arg(args: &str) -> bool {
    let list: Vec<&str> = args.split_whitespace().collect();
    list.iter().enumerate().any(|(i, a)| {
        a.eq_ignore_ascii_case("--backend=cgal")
            || (*a == "--backend"
                && list
                    .get(i + 1)
                    .is_some_and(|v| v.eq_ignore_ascii_case("cgal")))
    })
}

/// The binary that the search found. `forget` clears it.
static FOUND: Mutex<Option<String>> = Mutex::new(None);
/// The probe of each binary. `forget` clears it.
static PROBES: Mutex<Option<HashMap<String, Probe>>> = Mutex::new(None);

/// Looks for OpenSCAD once (see `Config::binary`).
fn detected() -> String {
    let mut found = FOUND.lock().unwrap();
    found
        .get_or_insert_with(|| first_with_manifold(&candidates(), |b| probe(b).manifold))
        .clone()
}

/// Forgets the probes and the binary that the search found. The next call
/// probes again, for example after the user installs a snapshot.
pub fn forget() {
    *FOUND.lock().unwrap() = None;
    *PROBES.lock().unwrap() = None;
}

/// The first binary of `found` that has Manifold, else the first one, else
/// `openscad`.
fn first_with_manifold(found: &[String], manifold: impl Fn(&str) -> bool) -> String {
    found
        .iter()
        .find(|b| manifold(b))
        .or(found.first())
        .map_or_else(|| "openscad".to_string(), String::clone)
}

/// The OpenSCAD binaries on this computer, in search order: on the PATH
/// (`openscad-nightly` first), then in the usual install places.
fn candidates() -> Vec<String> {
    let mut out = Vec::new();
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
        let hit = paths
            .iter()
            .find_map(|dir| (exts.iter()).find(|ext| dir.join(format!("{name}{ext}")).is_file()));
        if let Some(ext) = hit {
            out.push(format!("{name}{ext}"));
        }
    }
    {
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
        out.extend(
            (known.iter())
                .filter(|p| Path::new(p).is_file())
                .map(|p| p.to_string()),
        );
    }
    out
}

/// The download page of the development snapshots.
pub const SNAPSHOTS_URL: &str = "https://openscad.org/downloads.html#snapshots";

/// The steps to install an OpenSCAD development snapshot on this system.
/// Each step has a text and an optional line to copy.
pub fn install_steps() -> Vec<(&'static str, Option<&'static str>)> {
    if cfg!(windows) {
        vec![
            (
                "Download a development snapshot for Windows from the OpenSCAD download page.",
                None,
            ),
            (
                "Install it. CoCAD looks in C:\\Program Files\\OpenSCAD (Nightly) and \
                 C:\\Program Files\\OpenSCAD.",
                None,
            ),
            ("Click Check again.", None),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            (
                "Download a development snapshot for macOS from the OpenSCAD download page.",
                None,
            ),
            ("Install it in the Applications folder.", None),
            ("Click Check again.", None),
        ]
    } else {
        vec![
            (
                "Debian and Ubuntu: add the OpenSCAD repository on download.opensuse.org \
                 (home:/t-paul). On Debian 13, the source line is below. Other releases use \
                 another folder name.",
                Some("deb https://download.opensuse.org/repositories/home:/t-paul/Debian_13/ ./"),
            ),
            (
                "Add the signing key of the repository, as the OpenSCAD download page shows.",
                None,
            ),
            (
                "Install the package openscad-nightly.",
                Some("sudo apt update && sudo apt install openscad-nightly"),
            ),
            ("Click Check again.", None),
        ]
    }
}

/// The problem when `probe` of `binary` has no Manifold backend. `None`
/// when it has one.
pub fn problem(binary: &str, probe: &Probe) -> Option<String> {
    if probe.manifold {
        return None;
    }
    Some(if probe.version == "not found" {
        format!("OpenSCAD was not found ({binary}).")
    } else {
        format!(
            "{} ({binary}) has no Manifold backend.",
            probe.version.trim()
        )
    })
}

/// An error when the OpenSCAD binary of `cfg` has no Manifold backend. The
/// command line and `cocad mcp` stop with it.
pub fn check(cfg: &Config) -> Result<()> {
    let binary = cfg.binary();
    let Some(problem) = problem(&binary, &probe(&binary)) else {
        return Ok(());
    };
    let mut text = format!(
        "{problem}\nCoCAD needs an OpenSCAD development snapshot with the Manifold backend.\n"
    );
    for (i, (step, line)) in install_steps().iter().enumerate() {
        let step = step.replace("Click Check again.", "Run the command again.");
        text.push_str(&format!("{}. {step}\n", i + 1));
        if let Some(line) = line {
            text.push_str(&format!("   {line}\n"));
        }
    }
    text.push_str(&format!(
        "Download page: {SNAPSHOTS_URL}\nTo use another binary, set $OPENSCAD or the binary in File > Settings > OpenSCAD."
    ));
    bail!(text)
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
    let mut cache = PROBES.lock().unwrap();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_text_round_trip() {
        let mut c = Config {
            binary: "/opt/openscad".into(),
            args: "--enable=lazy-union".into(),
            theme: "dark".into(),
            mcp: false,
            autosave_sets: true,
            ..Config::default()
        };
        c.closed_cards = ["Mesh", "Print settings"].map(String::from).into();
        c.bed = crate::optimize::Area {
            w: 250.0,
            d: 210.0,
            h: 220.5,
        };
        c.bed_preset = "Creality Ender-3 V3".into();
        let text = c.text();
        assert!(
            text.contains("bed_w = 250\nbed_d = 210\nbed_h = 220.5\n"),
            "{text}"
        );
        assert!(
            text.contains("closed_cards = Mesh,Print settings\n"),
            "{text}"
        );
        assert_eq!(Config::parse(&text), c);
        assert_eq!(Config::parse(&Config::default().text()), Config::default());
    }

    #[test]
    fn config_reads_old_and_unknown_keys() {
        let c = Config::parse(
            "theme = light\nclosed_cards = , Build ,\nmanifold = false\nother = 1\nno equals sign\n",
        );
        assert_eq!(c.theme, "light");
        assert_eq!(c.closed_cards, BTreeSet::from(["Build".to_string()]));
        assert!(!Config::default().text().contains("manifold"));
        // Print area values out of the limits keep the default.
        let c = Config::parse("bed_w = 300\nbed_d = -5\nbed_h = x\n");
        let default = crate::optimize::Area::default();
        assert_eq!((c.bed.w, c.bed.d, c.bed.h), (300.0, default.d, default.h));
    }

    #[test]
    fn manifold_is_always_added_unless_the_user_selects_a_backend() {
        assert_eq!(with_backend(""), ["--backend=manifold"]);
        assert_eq!(
            with_backend("--enable=lazy-union"),
            ["--enable=lazy-union", "--backend=manifold"]
        );
        assert_eq!(with_backend("--backend=cgal"), ["--backend=cgal"]);
        assert!(cgal_arg("--backend=cgal"));
        assert!(cgal_arg("-q --backend CGAL"));
        assert!(!cgal_arg("--backend=manifold"));
        assert!(!cgal_arg("--backend"));
        assert!(!cgal_arg(""));
    }

    #[test]
    fn search_takes_the_first_binary_with_manifold() {
        let found = ["openscad-nightly".to_string(), "openscad".to_string()];
        assert_eq!(first_with_manifold(&found, |b| b == "openscad"), "openscad");
        assert_eq!(first_with_manifold(&found, |_| true), "openscad-nightly");
        assert_eq!(first_with_manifold(&found, |_| false), "openscad-nightly");
        assert_eq!(first_with_manifold(&[], |_| true), "openscad");
    }

    #[test]
    fn problem_names_the_version_or_not_found() {
        let old = Probe {
            version: "OpenSCAD version 2021.01".into(),
            manifold: false,
        };
        let text = problem("/usr/bin/openscad", &old).unwrap();
        assert!(
            text.contains("2021.01") && text.contains("/usr/bin/openscad"),
            "{text}"
        );
        let none = Probe {
            version: "not found".into(),
            manifold: false,
        };
        assert!(problem("openscad", &none).unwrap().contains("not found"));
        let nightly = Probe {
            version: "OpenSCAD version 2026.11.11.nightly".into(),
            manifold: true,
        };
        assert_eq!(problem("openscad-nightly", &nightly), None);
        assert!(!install_steps().is_empty());
    }
}
