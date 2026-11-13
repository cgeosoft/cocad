//! Runs OpenSCAD on a mesh. CoCAD writes the mesh to a temporary STL file,
//! writes a small `.scad` file that imports it and changes it, and reads the
//! result back. The Manifold backend makes this fast.
//!
//! The helpers below write OpenSCAD values in a form that OpenSCAD reads on
//! all systems.

use crate::export::stl_bytes;
use crate::mesh::Mesh;
use crate::openscad::{self, Config};
use anyhow::Result;
use glam::{Mat4, Vec2, Vec3};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// A temp file path that no other run uses.
fn temp_path(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "cocad-opt-{}-{}-{tag}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Removes the files when it goes out of scope.
struct TempFiles(Vec<PathBuf>);

impl Drop for TempFiles {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Runs OpenSCAD with the code that `body` makes. `body` gets one OpenSCAD
/// statement for each mesh of `meshes`, in the same order, for example
/// `import("/tmp/cocad-opt-1-0-a.stl", convexity = 10);`. Returns the result
/// mesh.
pub fn run(
    cfg: &Config,
    meshes: &[&Mesh],
    body: impl FnOnce(&[String]) -> String,
    cancel: &Arc<AtomicBool>,
) -> Result<Mesh> {
    let mut temp = TempFiles(Vec::new());
    let mut imports = Vec::with_capacity(meshes.len());
    for (i, m) in meshes.iter().enumerate() {
        let path = temp_path(&format!("in{i}.stl"));
        std::fs::write(&path, stl_bytes(m))?;
        imports.push(import(&path));
        temp.0.push(path);
    }
    let text = body(&imports);
    let scad = temp_path("fix.scad");
    std::fs::write(&scad, &text)?;
    temp.0.push(scad.clone());
    let out = openscad::render(cfg, &scad, &[], cancel)?;
    Ok(out.mesh)
}

/// The OpenSCAD statement that imports the STL file at `path`.
pub fn import(path: &Path) -> String {
    format!(
        "import({}, convexity = 10);",
        quote(&path.to_string_lossy().replace('\\', "/"))
    )
}

/// An OpenSCAD string literal for `value`.
pub fn quote(value: &str) -> String {
    let mut s = String::with_capacity(value.len() + 2);
    s.push('"');
    for c in value.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '"' => s.push_str("\\\""),
            _ => s.push(c),
        }
    }
    s.push('"');
    s
}

/// A number with up to 5 decimals and no trailing zeros.
pub fn num(x: f32) -> String {
    if !x.is_finite() {
        return "0".into();
    }
    let s = format!("{:.5}", x);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// `x` with all its digits: OpenSCAD reads the same value as the f32 in
/// the imported STL. Use it for a face that must be in the plane of a face
/// of the part. With `num`, 2.8 is not the f32 2.8 (2.79999995...), and
/// OpenSCAD keeps a step of 5e-8 mm between the two faces: a band of
/// triangles with no width.
pub fn exact(x: f32) -> String {
    if !x.is_finite() || x == 0.0 {
        return "0".into();
    }
    // An f64 holds each f32 value exactly. Its text is the shortest that
    // gives the same f64 back.
    format!("{}", x as f64)
}

/// `[x, y]`
pub fn v2(p: Vec2) -> String {
    format!("[{}, {}]", num(p.x), num(p.y))
}

/// `[x, y, z]`
pub fn v3(p: Vec3) -> String {
    format!("[{}, {}, {}]", num(p.x), num(p.y), num(p.z))
}

/// `polygon(points = [...], paths = [...])` for one or more closed loops.
/// Outer loops and holes can be in any order: OpenSCAD uses the even-odd
/// rule for the paths.
pub fn polygon(loops: &[Vec<Vec2>]) -> String {
    let mut pts = String::new();
    let mut paths = String::new();
    let mut n = 0;
    for (k, lp) in loops.iter().enumerate() {
        if k > 0 {
            paths.push_str(", ");
        }
        paths.push('[');
        for (i, p) in lp.iter().enumerate() {
            if n > 0 {
                pts.push_str(", ");
            }
            pts.push_str(&v2(*p));
            if i > 0 {
                paths.push_str(", ");
            }
            let _ = write!(paths, "{n}");
            n += 1;
        }
        paths.push(']');
    }
    format!("polygon(points = [{pts}], paths = [{paths}]);")
}

/// `multmatrix([[...], [...], [...], [...]])` without the children.
pub fn multmatrix(m: Mat4) -> String {
    let r = |i: usize| {
        let row = m.row(i);
        format!(
            "[{}, {}, {}, {}]",
            num(row.x),
            num(row.y),
            num(row.z),
            num(row.w)
        )
    };
    format!("multmatrix([{}, {}, {}, {}])", r(0), r(1), r(2), r(3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.25), "0.25");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(-1.5), "-1.5");
    }

    #[test]
    fn exact_numbers_give_the_f32_back() {
        assert_eq!(exact(2.8), "2.799999952316284");
        assert_eq!(exact(0.5), "0.5");
        assert_eq!(exact(-0.0), "0");
        for x in [2.8f32, 0.1, -36.0, 153.2, 329.99, 1.0e-3] {
            let back: f64 = exact(x).parse().unwrap();
            assert_eq!(back, x as f64);
        }
    }

    #[test]
    fn polygon_numbers_the_points_over_all_loops() {
        let a = vec![Vec2::ZERO, Vec2::X, Vec2::Y];
        let b = vec![Vec2::splat(0.1), Vec2::splat(0.2), Vec2::new(0.1, 0.2)];
        let p = polygon(&[a, b]);
        assert!(p.contains("paths = [[0, 1, 2], [3, 4, 5]]"), "{p}");
    }

    #[test]
    fn multmatrix_writes_rows() {
        let m = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(
            multmatrix(m),
            "multmatrix([[1, 0, 0, 1], [0, 1, 0, 2], [0, 0, 1, 3], [0, 0, 0, 1]])"
        );
    }
}
