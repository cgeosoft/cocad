//! Exports a mesh as binary STL, 3MF or STEP.

use crate::mesh::Mesh;
use anyhow::{Result, bail};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Stl,
    ThreeMf,
    Step,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Stl, Format::ThreeMf, Format::Step];

    pub fn ext(self) -> &'static str {
        match self {
            Format::Stl => "stl",
            Format::ThreeMf => "3mf",
            Format::Step => "step",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Stl => "STL (binary)",
            Format::ThreeMf => "3MF",
            Format::Step => "STEP (AP214, faceted)",
        }
    }

    pub fn from_path(p: &Path) -> Option<Format> {
        match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "stl" => Some(Format::Stl),
            "3mf" => Some(Format::ThreeMf),
            "step" | "stp" => Some(Format::Step),
            _ => None,
        }
    }
}

/// Extra data written into 3MF and STEP headers.
#[derive(Default, Clone)]
pub struct Meta {
    pub title: String,
    /// `name = value` lines, for the 3MF metadata.
    pub params: Vec<(String, String)>,
}

pub fn write(path: &Path, mesh: &Mesh, meta: &Meta) -> Result<()> {
    if mesh.is_empty() {
        bail!("nothing to export");
    }
    let fmt = Format::from_path(path)
        .ok_or_else(|| anyhow::anyhow!("unknown export type: {}", path.display()))?;
    let bytes = match fmt {
        Format::Stl => stl_bytes(mesh),
        Format::ThreeMf => threemf_bytes(mesh, meta)?,
        Format::Step => step_string(mesh, meta).into_bytes(),
    };
    std::fs::write(path, bytes)?;
    Ok(())
}

pub fn stl_bytes(mesh: &Mesh) -> Vec<u8> {
    let mut out = Vec::with_capacity(84 + mesh.triangles.len() * 50);
    let mut header = [0u8; 80];
    let tag = b"scad-viewer binary STL";
    header[..tag.len()].copy_from_slice(tag);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(mesh.triangles.len() as u32).to_le_bytes());
    for i in 0..mesh.triangles.len() {
        let n = mesh.normal(i);
        for v in std::iter::once(n).chain(mesh.tri(i)) {
            for c in v.to_array() {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn threemf_bytes(mesh: &Mesh, meta: &Meta) -> Result<Vec<u8>> {
    let mut model = String::with_capacity(mesh.vertices.len() * 60 + mesh.triangles.len() * 50);
    model.push_str(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" \
         xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n",
    );
    let _ = writeln!(
        model,
        " <metadata name=\"Title\">{}</metadata>",
        xml_escape(&meta.title)
    );
    model.push_str(" <metadata name=\"Application\">scad-viewer</metadata>\n");
    if !meta.params.is_empty() {
        let desc: Vec<String> = meta
            .params
            .iter()
            .map(|(k, v)| format!("{k} = {v}"))
            .collect();
        let _ = writeln!(
            model,
            " <metadata name=\"Description\">{}</metadata>",
            xml_escape(&desc.join("; "))
        );
    }
    model.push_str(" <resources>\n  <object id=\"1\" type=\"model\">\n   <mesh>\n    <vertices>\n");
    for v in &mesh.vertices {
        let _ = writeln!(
            model,
            "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
            v.x, v.y, v.z
        );
    }
    model.push_str("    </vertices>\n    <triangles>\n");
    for t in &mesh.triangles {
        let _ = writeln!(
            model,
            "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>",
            t[0], t[1], t[2]
        );
    }
    model.push_str("    </triangles>\n   </mesh>\n  </object>\n </resources>\n <build>\n  <item objectid=\"1\"/>\n </build>\n</model>\n");

    let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n \
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n \
<Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n</Types>\n";
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n \
<Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" \
Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\n</Relationships>\n";

    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("[Content_Types].xml", opts)?;
        zip.write_all(content_types.as_bytes())?;
        zip.start_file("_rels/.rels", opts)?;
        zip.write_all(rels.as_bytes())?;
        zip.start_file("3D/3dmodel.model", opts)?;
        zip.write_all(model.as_bytes())?;
        zip.finish()?;
    }
    Ok(buf.into_inner())
}

/// One planar face of the STEP output: an outer loop and holes, as vertex indices.
struct Face {
    normal: DVec3,
    loops: Vec<Vec<u32>>,
}

/// Merges connected coplanar triangles into polygon faces, so a cube
/// becomes 6 faces, not 12 triangles. A patch whose boundary cannot be
/// traced cleanly falls back to its triangles.
fn planar_faces(mesh: &Mesh) -> Vec<Face> {
    let n = mesh.triangles.len();
    let normals: Vec<DVec3> = (0..n).map(|i| mesh.normal(i).as_dvec3()).collect();
    let plane_d =
        |i: usize| normals[i].dot(mesh.vertices[mesh.triangles[i][0] as usize].as_dvec3());

    // Directed edge -> triangle.
    let mut edge_tri: HashMap<(u32, u32), usize> = HashMap::with_capacity(n * 3);
    for (i, t) in mesh.triangles.iter().enumerate() {
        for k in 0..3 {
            edge_tri.insert((t[k], t[(k + 1) % 3]), i);
        }
    }

    let mut patch = vec![usize::MAX; n];
    let mut faces = Vec::new();
    for seed in 0..n {
        if patch[seed] != usize::MAX {
            continue;
        }
        let pid = seed;
        let (sn, sd) = (normals[seed], plane_d(seed));
        let mut members = vec![seed];
        patch[seed] = pid;
        let mut stack = vec![seed];
        while let Some(i) = stack.pop() {
            let t = mesh.triangles[i];
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if let Some(&j) = edge_tri.get(&(b, a))
                    && patch[j] == usize::MAX
                    && normals[j].dot(sn) > 0.999_999
                    && (plane_d(j) - sd).abs() < 1e-4
                {
                    patch[j] = pid;
                    members.push(j);
                    stack.push(j);
                }
            }
        }

        let tri_faces = |members: &[usize]| -> Vec<Face> {
            members
                .iter()
                .map(|&i| Face {
                    normal: normals[i],
                    loops: vec![mesh.triangles[i].to_vec()],
                })
                .collect()
        };
        if members.len() == 1 {
            faces.extend(tri_faces(&members));
            continue;
        }

        // Boundary edges: directed edges whose twin is not in the patch.
        let in_patch: HashSet<usize> = members.iter().copied().collect();
        let mut next: HashMap<u32, u32> = HashMap::new();
        let mut clean = true;
        for &i in &members {
            let t = mesh.triangles[i];
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let twin_inside = edge_tri.get(&(b, a)).is_some_and(|j| in_patch.contains(j));
                if !twin_inside && next.insert(a, b).is_some() {
                    clean = false; // two boundary loops touch at a vertex
                }
            }
        }
        if !clean {
            faces.extend(tri_faces(&members));
            continue;
        }
        let mut loops = Vec::new();
        let mut used = HashSet::new();
        for &start in next.keys() {
            if used.contains(&start) {
                continue;
            }
            let mut lp = vec![start];
            used.insert(start);
            let mut cur = next[&start];
            while cur != start {
                if !used.insert(cur) || lp.len() > next.len() {
                    clean = false;
                    break;
                }
                lp.push(cur);
                match next.get(&cur) {
                    Some(&c) => cur = c,
                    None => {
                        clean = false;
                        break;
                    }
                }
            }
            if !clean {
                break;
            }
            // Collinear points stay: neighbour faces share them.
            loops.push(lp);
        }
        if !clean || loops.iter().any(|l| l.len() < 3) {
            faces.extend(tri_faces(&members));
            continue;
        }
        // Outer loop first: the one with the largest area.
        loops.sort_by(|a, b| loop_area(mesh, b, sn).total_cmp(&loop_area(mesh, a, sn)));
        if loop_area(mesh, &loops[0], sn) <= 0.0 {
            faces.extend(tri_faces(&members));
            continue;
        }
        faces.push(Face { normal: sn, loops });
    }
    faces
}

fn loop_area(mesh: &Mesh, lp: &[u32], n: DVec3) -> f64 {
    let mut s = DVec3::ZERO;
    for k in 0..lp.len() {
        let a = mesh.vertices[lp[k] as usize].as_dvec3();
        let b = mesh.vertices[lp[(k + 1) % lp.len()] as usize].as_dvec3();
        s += a.cross(b);
    }
    0.5 * s.dot(n)
}

fn f(x: f64) -> String {
    // STEP reals need a decimal point.
    let s = format!("{x}");
    if s.contains(['.', 'e', 'E']) {
        s
    } else {
        format!("{s}.")
    }
}

fn step_text(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii() && *c != '\'' && *c != '\\')
        .collect()
}

/// STEP AP214 file with a FACETED_BREP solid (or a shell when the mesh is open).
pub fn step_string(mesh: &Mesh, meta: &Meta) -> String {
    let name = step_text(if meta.title.is_empty() {
        "model"
    } else {
        &meta.title
    });
    let faces = planar_faces(mesh);
    let mut s = String::with_capacity(faces.len() * 400 + mesh.vertices.len() * 60);
    let mut id = 100usize;
    let mut next = || {
        id += 1;
        id
    };

    let _ = write!(
        s,
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('{name}'),'2;1');\n\
FILE_NAME('{name}.step','',(''),(''),'scad-viewer','scad-viewer','');\n\
FILE_SCHEMA(('AUTOMOTIVE_DESIGN {{ 1 0 10303 214 1 1 1 1 }}'));\nENDSEC;\nDATA;\n\
#1=APPLICATION_CONTEXT('core data for automotive mechanical design processes');\n\
#2=APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,#1);\n\
#3=PRODUCT_CONTEXT('',#1,'mechanical');\n\
#4=PRODUCT('{name}','{name}','',(#3));\n\
#5=PRODUCT_DEFINITION_FORMATION('','',#4);\n\
#6=PRODUCT_DEFINITION_CONTEXT('part definition',#1,'design');\n\
#7=PRODUCT_DEFINITION('design','',#5,#6);\n\
#8=PRODUCT_DEFINITION_SHAPE('','',#7);\n\
#9=PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,(#4));\n\
#10=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\n\
#11=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));\n\
#12=(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT());\n\
#13=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-06),#10,'distance_accuracy_value','confusion accuracy');\n\
#14=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#13))\
GLOBAL_UNIT_ASSIGNED_CONTEXT((#10,#11,#12))REPRESENTATION_CONTEXT('',''));\n\
#15=CARTESIAN_POINT('',(0.,0.,0.));\n\
#16=DIRECTION('',(0.,0.,1.));\n\
#17=DIRECTION('',(1.,0.,0.));\n\
#18=AXIS2_PLACEMENT_3D('',#15,#16,#17);\n"
    );

    let mut point_id = vec![0usize; mesh.vertices.len()];
    for (i, v) in mesh.vertices.iter().enumerate() {
        let pid = next();
        point_id[i] = pid;
        let _ = writeln!(
            s,
            "#{pid}=CARTESIAN_POINT('',({},{},{}));",
            f(v.x as f64),
            f(v.y as f64),
            f(v.z as f64)
        );
    }

    let mut face_ids = Vec::with_capacity(faces.len());
    for face in &faces {
        let p0 = mesh.vertices[face.loops[0][0] as usize].as_dvec3();
        let p1 = mesh.vertices[face.loops[0][1] as usize].as_dvec3();
        let normal = face.normal;
        let refdir = {
            let r = (p1 - p0).reject_from(normal).normalize_or_zero();
            if r == DVec3::ZERO {
                normal.any_orthonormal_vector()
            } else {
                r
            }
        };
        let (dn, dr, ax, pl) = (next(), next(), next(), next());
        let _ = writeln!(
            s,
            "#{dn}=DIRECTION('',({},{},{}));",
            f(normal.x),
            f(normal.y),
            f(normal.z)
        );
        let _ = writeln!(
            s,
            "#{dr}=DIRECTION('',({},{},{}));",
            f(refdir.x),
            f(refdir.y),
            f(refdir.z)
        );
        let _ = writeln!(
            s,
            "#{ax}=AXIS2_PLACEMENT_3D('',#{},#{dn},#{dr});",
            point_id[face.loops[0][0] as usize]
        );
        let _ = writeln!(s, "#{pl}=PLANE('',#{ax});");
        let mut bounds = Vec::new();
        for (k, lp) in face.loops.iter().enumerate() {
            let (l, b) = (next(), next());
            let pts: Vec<String> = lp
                .iter()
                .map(|&v| format!("#{}", point_id[v as usize]))
                .collect();
            let _ = writeln!(s, "#{l}=POLY_LOOP('',({}));", pts.join(","));
            let kind = if k == 0 {
                "FACE_OUTER_BOUND"
            } else {
                "FACE_BOUND"
            };
            let _ = writeln!(s, "#{b}={kind}('',#{l},.T.);");
            bounds.push(format!("#{b}"));
        }
        let fid = next();
        let _ = writeln!(
            s,
            "#{fid}=FACE_SURFACE('',({}),#{pl},.T.);",
            bounds.join(",")
        );
        face_ids.push(format!("#{fid}"));
    }

    let closed = mesh.open_edges() == 0;
    let (shell, solid, rep) = (next(), next(), next());
    let face_list = face_ids.join(",");
    if closed {
        let _ = writeln!(s, "#{shell}=CLOSED_SHELL('',({face_list}));");
        let _ = writeln!(s, "#{solid}=FACETED_BREP('{name}',#{shell});");
        let _ = writeln!(
            s,
            "#{rep}=FACETED_BREP_SHAPE_REPRESENTATION('{name}',(#18,#{solid}),#14);"
        );
    } else {
        let _ = writeln!(s, "#{shell}=OPEN_SHELL('',({face_list}));");
        let _ = writeln!(
            s,
            "#{solid}=SHELL_BASED_SURFACE_MODEL('{name}',(#{shell}));"
        );
        let _ = writeln!(
            s,
            "#{rep}=MANIFOLD_SURFACE_SHAPE_REPRESENTATION('{name}',(#18,#{solid}),#14);"
        );
    }
    let sdr = next();
    let _ = writeln!(s, "#{sdr}=SHAPE_DEFINITION_REPRESENTATION(#8,#{rep});");
    s.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    s
}

pub fn suggested_name(stem: &str, fmt: Format) -> String {
    format!("{stem}.{}", fmt.ext())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::cube;

    #[test]
    fn cube_step_has_six_faces() {
        let s = step_string(
            &cube(10.0),
            &Meta {
                title: "cube".into(),
                ..Default::default()
            },
        );
        assert_eq!(s.matches("FACE_SURFACE").count(), 6);
        assert_eq!(s.matches("CARTESIAN_POINT").count(), 8 + 1);
        assert!(s.contains("CLOSED_SHELL"));
    }

    #[test]
    fn threemf_is_zip() {
        let b = threemf_bytes(&cube(1.0), &Meta::default()).unwrap();
        let z = zip::ZipArchive::new(std::io::Cursor::new(b)).unwrap();
        assert_eq!(z.len(), 3);
    }
}
