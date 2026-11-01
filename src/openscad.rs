//! Runs the OpenSCAD CLI to turn a .scad file and parameters into a mesh.

use crate::mesh::{Mesh, read_stl};
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct RenderOutput {
    pub mesh: Mesh,
    pub log: String,
    pub elapsed: Duration,
}

/// The OpenSCAD binary: `$OPENSCAD`, else `openscad` on the PATH.
pub fn binary() -> String {
    std::env::var("OPENSCAD").unwrap_or_else(|_| "openscad".into())
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
pub fn render(file: &Path, defines: &[String], cancel: &Arc<AtomicBool>) -> Result<RenderOutput> {
    let start = Instant::now();
    let out = temp_stl();
    let log_path = out.with_extension("log");
    let log_file = std::fs::File::create(&log_path)?;

    let file = file.canonicalize()?;
    let mut cmd = Command::new(binary());
    cmd.arg("-o").arg(&out).arg("--export-format").arg("binstl");
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
            "cannot start '{}': {e}. Install OpenSCAD or set $OPENSCAD.",
            binary()
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
