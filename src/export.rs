//! Exports a mesh as binary STL, 3MF or STEP.

use crate::mesh::Mesh;
use crate::parts::Part;
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
    let tag = b"cocad binary STL";
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

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n \
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n \
<Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n</Types>\n";
const CONTENT_TYPES_PARTS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n \
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n \
<Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n \
<Default Extension=\"config\" ContentType=\"text/xml\"/>\n</Types>\n";
const RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n \
<Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" \
Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\n</Relationships>\n";
/// The plate position of a model in an OrcaSlicer file, in mm.
const ORCA_CENTER: f32 = 100.0;

/// The slicer that a multicolor 3MF is for. Each one reads its own part data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slicer {
    Prusa,
    Orca,
}

impl Slicer {
    pub const ALL: [Slicer; 2] = [Slicer::Prusa, Slicer::Orca];

    pub fn label(self) -> &'static str {
        match self {
            Slicer::Prusa => "PrusaSlicer 2.9",
            Slicer::Orca => "OrcaSlicer",
        }
    }

    pub fn from_name(name: &str) -> Option<Slicer> {
        match name.to_ascii_lowercase().as_str() {
            "prusa" | "prusaslicer" => Some(Slicer::Prusa),
            "orca" | "orcaslicer" => Some(Slicer::Orca),
            _ => None,
        }
    }
}

/// The start of a 3MF model file, up to the `<resources>` element.
fn model_head(meta: &Meta, namespaces: &str, metadata: &str) -> String {
    let mut model = String::new();
    let _ = writeln!(
        model,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" \
         xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\"{namespaces}>"
    );
    model.push_str(metadata);
    let _ = writeln!(
        model,
        " <metadata name=\"Title\">{}</metadata>",
        xml_escape(&meta.title)
    );
    model.push_str(" <metadata name=\"Application\">cocad</metadata>\n");
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
    model
}

/// Zips the parts of a 3MF package. The files are `(name, content)`.
fn package(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in files {
            zip.start_file(*name, opts)?;
            zip.write_all(content)?;
        }
        zip.finish()?;
    }
    Ok(buf.into_inner())
}

pub fn threemf_bytes(mesh: &Mesh, meta: &Meta) -> Result<Vec<u8>> {
    let mut model = model_head(meta, "", "");
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

    package(&[
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", RELS.as_bytes()),
        ("3D/3dmodel.model", model.as_bytes()),
    ])
}

/// A 3MF file with one object for each mesh in `objects` (name, mesh) and
/// one build item for each entry of `items` (object index, matrix). The
/// slicer then shows each copy as a separate object at its place.
// The Optimize tab uses it in the next step.
#[allow(dead_code)]
pub fn plate_3mf_bytes(
    objects: &[(String, &Mesh)],
    items: &[(usize, glam::Mat4)],
    meta: &Meta,
) -> Result<Vec<u8>> {
    if items.is_empty() {
        bail!("nothing to export");
    }
    let mut model = model_head(meta, "", "");
    model.push_str(" <resources>\n");
    for (i, (name, mesh)) in objects.iter().enumerate() {
        let _ = writeln!(
            model,
            "  <object id=\"{}\" name=\"{}\" type=\"model\">\n   <mesh>\n    <vertices>",
            i + 1,
            xml_escape(name)
        );
        mesh_rows(&mut model, mesh, true, 0);
        model.push_str("    </vertices>\n    <triangles>\n");
        mesh_rows(&mut model, mesh, false, 0);
        model.push_str("    </triangles>\n   </mesh>\n  </object>\n");
    }
    model.push_str(" </resources>\n <build>\n");
    for (obj, m) in items {
        // 3MF: x' = m00 x + m10 y + m20 z + m30, so the columns of the
        // glam matrix are the rows of the 3MF matrix.
        let c = [m.x_axis, m.y_axis, m.z_axis, m.w_axis];
        let t: Vec<String> = c
            .iter()
            .flat_map(|v| [v.x, v.y, v.z])
            .map(|x| x.to_string())
            .collect();
        let _ = writeln!(
            model,
            "  <item objectid=\"{}\" transform=\"{}\"/>",
            obj + 1,
            t.join(" ")
        );
    }
    model.push_str(" </build>\n</model>\n");
    package(&[
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", RELS.as_bytes()),
        ("3D/3dmodel.model", model.as_bytes()),
    ])
}

/// `name` with `-plate<n>` before the extension, for plate `plate` (0 is
/// plate 1).
#[allow(dead_code)]
pub fn plate_name(name: &str, plate: usize) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}-plate{}.{ext}", plate + 1),
        None => format!("{name}-plate{}", plate + 1),
    }
}

/// Writes one mesh as `<vertices>` and `<triangles>`. Indices start at `first`.
fn mesh_rows(model: &mut String, mesh: &Mesh, vertices: bool, first: u32) {
    if vertices {
        for v in &mesh.vertices {
            let _ = writeln!(
                model,
                "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
                v.x, v.y, v.z
            );
        }
    } else {
        for t in &mesh.triangles {
            let _ = writeln!(
                model,
                "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>",
                t[0] + first,
                t[1] + first,
                t[2] + first
            );
        }
    }
}

/// PrusaSlicer 2.9: one mesh with all parts. `Slic3r_PE_model.config` gives
/// each part a triangle range and an extruder.
fn prusa_bytes(parts: &[Part], meta: &Meta) -> Result<Vec<u8>> {
    let mut model = model_head(meta, "", "");
    model.push_str(" <resources>\n  <object id=\"1\" type=\"model\">\n   <mesh>\n    <vertices>\n");
    for part in parts {
        mesh_rows(&mut model, &part.mesh, true, 0);
    }
    model.push_str("    </vertices>\n    <triangles>\n");
    let mut config = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n <object id=\"1\" instances_count=\"1\">\n",
    );
    let _ = writeln!(
        config,
        "  <metadata type=\"object\" key=\"name\" value=\"{}\"/>",
        xml_escape(&meta.title)
    );
    let (mut first_vertex, mut first_triangle) = (0u32, 0usize);
    for (i, part) in parts.iter().enumerate() {
        mesh_rows(&mut model, &part.mesh, false, first_vertex);
        let count = part.mesh.triangles.len();
        let _ = writeln!(
            config,
            "  <volume firstid=\"{}\" lastid=\"{}\">\n   \
             <metadata type=\"volume\" key=\"name\" value=\"{}\"/>\n   \
             <metadata type=\"volume\" key=\"volume_type\" value=\"ModelPart\"/>\n   \
             <metadata type=\"volume\" key=\"extruder\" value=\"{}\"/>\n  </volume>",
            first_triangle,
            first_triangle + count - 1,
            xml_escape(&part.name),
            i + 1
        );
        first_vertex += part.mesh.vertices.len() as u32;
        first_triangle += count;
    }
    model.push_str("    </triangles>\n   </mesh>\n  </object>\n </resources>\n <build>\n  <item objectid=\"1\"/>\n </build>\n</model>\n");
    config.push_str(" </object>\n</config>\n");
    package(&[
        ("[Content_Types].xml", CONTENT_TYPES_PARTS.as_bytes()),
        ("_rels/.rels", RELS.as_bytes()),
        ("3D/3dmodel.model", model.as_bytes()),
        ("Metadata/Slic3r_PE_model.config", config.as_bytes()),
    ])
}

/// OrcaSlicer: one mesh object for each part, joined by a component object.
/// `model_settings.config` gives each part an extruder.
///
/// `project_settings.config` holds only the filament colors and the flush
/// table, so the user's printer and process stay as they are. OrcaSlicer 2.4.2
/// crashes when the file has colors but no flush table.
fn orca_bytes(parts: &[Part], meta: &Meta) -> Result<Vec<u8>> {
    const NS: &str = " xmlns:BambuStudio=\"http://schemas.bambulab.com/package/2021\" \
                      xmlns:p=\"http://schemas.microsoft.com/3dmanufacturing/production/2015/06\" \
                      requiredextensions=\"p\"";
    const VERSION: &str = " <metadata name=\"BambuStudio:3mfVersion\">1</metadata>\n";
    const PATH: &str = "/3D/Objects/parts.model";

    let root_id = parts.len() + 1;
    let mut objects = model_head(meta, NS, VERSION);
    objects.push_str(" <resources>\n");
    let mut components = String::new();
    let mut part_settings = String::new();
    for (i, part) in parts.iter().enumerate() {
        let id = i + 1;
        let _ = writeln!(
            objects,
            "  <object id=\"{id}\" type=\"model\">\n   <mesh>\n    <vertices>"
        );
        mesh_rows(&mut objects, &part.mesh, true, 0);
        objects.push_str("    </vertices>\n    <triangles>\n");
        mesh_rows(&mut objects, &part.mesh, false, 0);
        objects.push_str("    </triangles>\n   </mesh>\n  </object>\n");
        let _ = writeln!(
            components,
            "    <component p:path=\"{PATH}\" objectid=\"{id}\" transform=\"1 0 0 0 1 0 0 0 1 0 0 0\"/>"
        );
        let _ = writeln!(
            part_settings,
            "    <part id=\"{id}\" subtype=\"normal_part\">\n     \
             <metadata key=\"name\" value=\"{}\"/>\n     \
             <metadata key=\"extruder\" value=\"{id}\"/>\n    </part>",
            xml_escape(&part.name)
        );
    }
    objects.push_str(" </resources>\n <build/>\n</model>\n");

    // Center the model near (100, 100) on the plate.
    let (min, max) = parts
        .iter()
        .filter_map(|p| p.mesh.bounds())
        .map(|b| (b.min, b.max))
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
        .ok_or_else(|| anyhow::anyhow!("nothing to export"))?;
    let (x, y, z) = (
        ORCA_CENTER - (min.x + max.x) / 2.0,
        ORCA_CENTER - (min.y + max.y) / 2.0,
        -min.z,
    );
    let mut root = model_head(meta, NS, VERSION);
    let _ = write!(
        root,
        " <resources>\n  <object id=\"{root_id}\" type=\"model\">\n   <components>\n{components}   \
         </components>\n  </object>\n </resources>\n <build>\n  \
         <item objectid=\"{root_id}\" transform=\"1 0 0 0 1 0 0 0 1 {x} {y} {z}\" printable=\"1\"/>\n \
         </build>\n</model>\n"
    );
    let rels = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n \
<Relationship Target=\"{PATH}\" Id=\"rel-1\" \
Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/>\n</Relationships>\n"
    );
    let settings = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n  <object id=\"{root_id}\">\n    \
         <metadata key=\"name\" value=\"{title}\"/>\n    \
         <metadata key=\"extruder\" value=\"1\"/>\n{part_settings}  </object>\n  <plate>\n    \
         <metadata key=\"plater_id\" value=\"1\"/>\n    <model_instance>\n      \
         <metadata key=\"object_id\" value=\"{root_id}\"/>\n      \
         <metadata key=\"instance_id\" value=\"0\"/>\n      \
         <metadata key=\"identify_id\" value=\"{root_id}\"/>\n    </model_instance>\n  </plate>\n</config>\n",
        title = xml_escape(&meta.title)
    );
    let n = parts.len();
    let flush: Vec<&str> = (0..n * n)
        .map(|i| if i % (n + 1) == 0 { "0" } else { "280" })
        .collect();
    let colors: Vec<String> = parts
        .iter()
        .map(|p| format!("#{:02X}{:02X}{:02X}", p.color[0], p.color[1], p.color[2]))
        .collect();
    let project = serde_json::json!({
        "filament_colour": colors,
        "flush_volumes_matrix": flush,
    })
    .to_string();
    package(&[
        ("[Content_Types].xml", CONTENT_TYPES_PARTS.as_bytes()),
        ("_rels/.rels", RELS.as_bytes()),
        ("3D/3dmodel.model", root.as_bytes()),
        ("3D/_rels/3dmodel.model.rels", rels.as_bytes()),
        ("3D/Objects/parts.model", objects.as_bytes()),
        ("Metadata/model_settings.config", settings.as_bytes()),
        ("Metadata/project_settings.config", project.as_bytes()),
    ])
}

pub fn parts_bytes(parts: &[Part], meta: &Meta, slicer: Slicer) -> Result<Vec<u8>> {
    if parts.is_empty() || parts.iter().any(|p| p.mesh.is_empty()) {
        bail!("nothing to export");
    }
    match slicer {
        Slicer::Prusa => prusa_bytes(parts, meta),
        Slicer::Orca => orca_bytes(parts, meta),
    }
}

/// Writes a multicolor 3MF. The path must end in `.3mf`.
pub fn write_parts(path: &Path, parts: &[Part], meta: &Meta, slicer: Slicer) -> Result<()> {
    if !matches!(Format::from_path(path), Some(Format::ThreeMf)) {
        bail!("a multicolor export must end in .3mf");
    }
    std::fs::write(path, parts_bytes(parts, meta, slicer)?)?;
    Ok(())
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
FILE_NAME('{name}.step','',(''),(''),'cocad','cocad','');\n\
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

/// File name for the save dialog: `stem.ext`, or `stem-set.ext` when a
/// parameter set is active. Characters that file systems reject become `_`.
pub fn suggested_name(stem: &str, set: Option<&str>, fmt: Format) -> String {
    let set = set
        .map(|s| {
            s.trim()
                .chars()
                .map(|c| {
                    if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                        '_'
                    } else {
                        c
                    }
                })
                .collect::<String>()
        })
        .filter(|s| !s.is_empty());
    match set {
        Some(set) => format!("{stem}-{set}.{}", fmt.ext()),
        None => format!("{stem}.{}", fmt.ext()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::cube;

    #[test]
    fn plate_3mf_has_one_item_per_copy() {
        let c = cube(10.0);
        let at = glam::Mat4::from_translation(glam::Vec3::new(20.0, 30.0, 0.0));
        let bytes = plate_3mf_bytes(
            &[("Part 1".into(), &c)],
            &[(0, glam::Mat4::IDENTITY), (0, at)],
            &Meta {
                title: "t".into(),
                params: vec![],
            },
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("3D/3dmodel.model").unwrap(), &mut text)
            .unwrap();
        assert_eq!(text.matches("<item ").count(), 2);
        assert!(
            text.contains("transform=\"1 0 0 0 1 0 0 0 1 20 30 0\""),
            "{text}"
        );
        assert!(text.contains("name=\"Part 1\""));
    }

    #[test]
    fn plate_name_goes_before_the_extension() {
        assert_eq!(plate_name("box-big.3mf", 1), "box-big-plate2.3mf");
        assert_eq!(plate_name("box", 0), "box-plate1");
    }

    #[test]
    fn suggested_name_adds_the_set() {
        assert_eq!(suggested_name("box", None, Format::Stl), "box.stl");
        assert_eq!(
            suggested_name("box", Some("big"), Format::ThreeMf),
            "box-big.3mf"
        );
        assert_eq!(
            suggested_name("box", Some("a/b: c"), Format::Stl),
            "box-a_b_ c.stl"
        );
        assert_eq!(suggested_name("box", Some("  "), Format::Stl), "box.stl");
    }

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

    fn two_parts() -> Vec<Part> {
        ["body", "star"]
            .into_iter()
            .map(|name| Part {
                name: name.into(),
                color: [0xff, 0, 0],
                mesh: cube(1.0),
            })
            .collect()
    }

    fn read(bytes: Vec<u8>, name: &str) -> String {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        std::io::read_to_string(z.by_name(name).unwrap()).unwrap()
    }

    #[test]
    fn prusa_file_has_a_range_for_each_part() {
        let tris = cube(1.0).triangles.len();
        let b = parts_bytes(&two_parts(), &Meta::default(), Slicer::Prusa).unwrap();
        let config = read(b.clone(), "Metadata/Slic3r_PE_model.config");
        assert!(config.contains(&format!("firstid=\"{tris}\" lastid=\"{}\"", 2 * tris - 1)));
        assert!(config.contains("key=\"extruder\" value=\"2\""));
        let model = read(b, "3D/3dmodel.model");
        let last = 2 * cube(1.0).vertices.len() - 1;
        assert!(model.contains(&format!("=\"{last}\"")));
    }

    #[test]
    fn orca_file_has_flush_table_and_extruders() {
        let b = parts_bytes(&two_parts(), &Meta::default(), Slicer::Orca).unwrap();
        let settings = read(b.clone(), "Metadata/model_settings.config");
        assert!(settings.contains("<part id=\"2\""));
        assert!(settings.contains("key=\"extruder\" value=\"2\""));
        let project: serde_json::Value =
            serde_json::from_str(&read(b, "Metadata/project_settings.config")).unwrap();
        assert_eq!(project["filament_colour"][1], "#FF0000");
        assert_eq!(
            project["flush_volumes_matrix"],
            serde_json::json!(["0", "280", "280", "0"])
        );
    }

    #[test]
    fn empty_parts_are_refused() {
        assert!(parts_bytes(&[], &Meta::default(), Slicer::Orca).is_err());
    }

    #[test]
    fn threemf_is_zip() {
        let b = threemf_bytes(&cube(1.0), &Meta::default()).unwrap();
        let z = zip::ZipArchive::new(std::io::Cursor::new(b)).unwrap();
        assert_eq!(z.len(), 3);
    }
}
