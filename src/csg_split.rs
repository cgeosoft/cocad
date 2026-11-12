//! Finds the parts of an assembly in the CSG tree of a model.
//!
//! OpenSCAD unions all top-level geometry, so parts that touch fuse into one
//! shell. But `openscad -o model.csg` writes the evaluated tree without the
//! geometry, and module instances are `group() { ... }` nodes in it. This
//! module finds the level of the tree where the assembly divides into module
//! instances, writes one .csg file for each piece and renders the pieces in
//! parallel.
//!
//! Limits:
//! - A model that is one part made of modules divides into these modules.
//! - Fonts from `use <x.ttf>` in the main file are added to each piece.
//!   Fonts that only an included or used library adds are not.
//! - OpenSCAD writes strings into the .csg file without escapes. The parser
//!   ends a string at a quote that `,`, `)` or `]` follows, and the piece
//!   files get the escapes. A string that holds such a quote, for example
//!   `text("a\", b")`, is not read correctly.

use crate::mesh::Mesh;
use crate::openscad::{self, Config};
use crate::parts::Part;
use anyhow::{Context, Result, bail};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// More pieces than this give no split. The app keeps the shells then.
pub const MAX_PIECES: usize = 64;
/// The first level and one level of sub-assemblies, for example
/// `assembly() { board(); armies(); }`. Deeper levels often divide one
/// part into the modules that make it.
pub const LEVELS: usize = 2;

/// Nodes that union their children. The split goes down through them.
const DESCEND: &[&str] = &["group", "union", "multmatrix", "color", "render"];
/// Nodes whose children can become pieces. `union()` is not one: an
/// explicit union joins its children into one part.
const SPLIT_KINDS: &[&str] = &["group", "multmatrix", "color", "render"];
/// Nodes that only move, color or cache their children.
const WRAPPERS: &[&str] = &["multmatrix", "color", "render"];
/// Nodes that need children to make geometry.
const OPERATORS: &[&str] = &[
    "group",
    "union",
    "difference",
    "intersection",
    "hull",
    "minkowski",
    "multmatrix",
    "color",
    "render",
    "linear_extrude",
    "rotate_extrude",
    "offset",
    "projection",
    "resize",
    "mirror",
    "roof",
    "fill",
];

/// One `name(args);` or `name(args) { children }` statement of a .csg file.
#[derive(Debug)]
pub struct Node {
    pub name: String,
    /// The `%`, `#`, `!` and `*` prefixes.
    pub modifiers: String,
    /// The full text of the node, with the prefixes and the children.
    pub span: Range<usize>,
    /// `name(args)` without the prefixes.
    pub header: Range<usize>,
    pub children: Vec<Node>,
}

/// A string literal in the .csg text.
#[derive(Debug)]
struct Literal {
    /// The literal with its quotes.
    span: Range<usize>,
    /// The `file` argument of `import()` or `surface()`.
    file: bool,
}

/// A parsed .csg file.
#[derive(Debug)]
pub struct Tree {
    pub text: String,
    /// The top-level statements.
    pub nodes: Vec<Node>,
    /// All string literals, in text order.
    literals: Vec<Literal>,
}

/// One piece of the assembly: a node and the nodes that contain it.
#[derive(Clone, Debug)]
pub struct Piece<'a> {
    /// From the top level down to the parent of `node`.
    pub ancestors: Vec<&'a Node>,
    pub node: &'a Node,
}

/// Parses the text of a .csg file.
pub fn parse(text: String) -> Result<Tree> {
    let mut p = Parser {
        s: text.as_bytes(),
        pos: 0,
        literals: Vec::new(),
    };
    let nodes = p.list(false)?;
    let literals = p.literals;
    Ok(Tree {
        text,
        nodes,
        literals,
    })
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
    literals: Vec<Literal>,
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    /// Statements up to the end of the text, or up to and with the `}`
    /// that closes `nested`.
    fn list(&mut self, nested: bool) -> Result<Vec<Node>> {
        let mut out = Vec::new();
        loop {
            self.skip_space();
            match self.peek() {
                None if nested => bail!("a '}}' is missing at the end"),
                None => return Ok(out),
                Some(b'}') if nested => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'}') => bail!("unexpected '}}' at byte {}", self.pos),
                Some(b';') => self.pos += 1,
                Some(_) => out.push(self.node()?),
            }
        }
    }

    fn node(&mut self) -> Result<Node> {
        let start = self.pos;
        let mut modifiers = String::new();
        while let Some(c @ (b'%' | b'#' | b'!' | b'*')) = self.peek() {
            modifiers.push(c as char);
            self.pos += 1;
            // OpenSCAD writes the prefix before the indent.
            self.skip_space();
        }
        let name_start = self.pos;
        while self.peek().is_some_and(is_ident) {
            self.pos += 1;
        }
        if self.pos == name_start {
            bail!("a name is missing at byte {}", self.pos);
        }
        let name = String::from_utf8_lossy(&self.s[name_start..self.pos]).into_owned();
        self.skip_space();
        if self.peek() != Some(b'(') {
            bail!("'(' is missing after '{name}' at byte {}", self.pos);
        }
        self.args(&name)?;
        let header = name_start..self.pos;
        self.skip_space();
        let children = match self.peek() {
            Some(b';') => {
                self.pos += 1;
                Vec::new()
            }
            Some(b'{') => {
                self.pos += 1;
                self.list(true)?
            }
            _ => bail!(
                "';' or '{{' is missing after '{name}(...)' at byte {}",
                self.pos
            ),
        };
        Ok(Node {
            name,
            modifiers,
            span: start..self.pos,
            header,
            children,
        })
    }

    /// The arguments from `(` up to and with the matching `)`.
    fn args(&mut self, name: &str) -> Result<()> {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                None => bail!("')' is missing after '{name}('"),
                Some(b'"') => self.literal(name)?,
                Some(b'(' | b'[') => {
                    depth += 1;
                    self.pos += 1;
                }
                Some(b')' | b']') => {
                    depth = depth.saturating_sub(1);
                    self.pos += 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                Some(_) => self.pos += 1,
            }
        }
    }

    /// A string literal. OpenSCAD writes the value without escapes, so a
    /// quote ends the literal only when `,`, `)`, `]` or the end follows it.
    fn literal(&mut self, name: &str) -> Result<()> {
        let open = self.pos;
        let mut at = open + 1;
        loop {
            let Some(q) = self.s[at..].iter().position(|&c| c == b'"') else {
                bail!("a string in '{name}(...)' at byte {open} has no end");
            };
            let close = at + q;
            let next = self.s[close + 1..]
                .iter()
                .find(|c| !c.is_ascii_whitespace())
                .copied();
            if matches!(next, None | Some(b',' | b')' | b']')) {
                let file = matches!(name, "import" | "surface") && self.arg_name(open) == "file";
                self.literals.push(Literal {
                    span: open..close + 1,
                    file,
                });
                self.pos = close + 1;
                return Ok(());
            }
            at = close + 1;
        }
    }

    /// The `name` of a `name = "..."` argument, from the position of the quote.
    fn arg_name(&self, quote: usize) -> &str {
        let before = &self.s[..quote];
        let Some(eq) = before.iter().rposition(|c| !c.is_ascii_whitespace()) else {
            return "";
        };
        if before[eq] != b'=' {
            return "";
        }
        let end = before[..eq]
            .iter()
            .rposition(|c| !c.is_ascii_whitespace())
            .map_or(0, |i| i + 1);
        let start = before[..end]
            .iter()
            .rposition(|&c| !is_ident(c))
            .map_or(0, |i| i + 1);
        std::str::from_utf8(&before[start..end]).unwrap_or("")
    }
}

impl Node {
    /// True for `%` (background) and `*` (disabled) nodes. OpenSCAD does not
    /// export them.
    fn is_background(&self) -> bool {
        self.modifiers.contains(['%', '*'])
    }

    /// True when the node makes geometry. An empty `group()` makes none.
    pub fn has_geometry(&self) -> bool {
        if self.is_background() {
            return false;
        }
        if self.children.is_empty() {
            return !OPERATORS.contains(&self.name.as_str());
        }
        let mut solid = self.children.iter().filter(|c| !c.is_background());
        match self.name.as_str() {
            "difference" => solid.next().is_some_and(Node::has_geometry),
            "intersection" => solid.all(Node::has_geometry),
            _ => solid.any(Node::has_geometry),
        }
    }

    fn geometry_children(&self) -> Vec<&Node> {
        geometry(&self.children)
    }

    /// True for a module instance or a union: a `group` or `union` node,
    /// also inside wrappers with one child.
    pub fn is_module_like(&self) -> bool {
        let mut n = self;
        while WRAPPERS.contains(&n.name.as_str()) {
            match n.geometry_children().as_slice() {
                [only] => n = only,
                _ => return false,
            }
        }
        matches!(n.name.as_str(), "group" | "union") && n.has_geometry()
    }
}

fn geometry(nodes: &[Node]) -> Vec<&Node> {
    nodes.iter().filter(|n| n.has_geometry()).collect()
}

/// The pieces under `start`, or under the top level when `start` is `None`.
/// The ancestors of each piece are relative: they start below `start`.
fn split_below<'a>(start: Option<&'a Node>, nodes: &'a [Node]) -> Option<Vec<Piece<'a>>> {
    if start.is_some_and(|s| !DESCEND.contains(&s.name.as_str())) {
        return None;
    }
    let mut kind = start.map(|s| s.name.as_str());
    let mut chain: Vec<&Node> = Vec::new();
    let mut kids = geometry(nodes);
    // Go down while there is only one child.
    while let [only] = kids.as_slice() {
        if !DESCEND.contains(&only.name.as_str()) {
            return None;
        }
        chain.push(only);
        kind = Some(only.name.as_str());
        kids = only.geometry_children();
    }
    if kids.is_empty() || kind.is_some_and(|k| !SPLIT_KINDS.contains(&k)) {
        return None;
    }
    let mut pieces = Vec::new();
    flatten(&chain, &kids, &mut pieces);
    if pieces.iter().filter(|p| p.node.is_module_like()).count() < 2 {
        // For example a part made of primitives.
        return None;
    }
    Some(pieces)
}

/// Adds each node as a piece. A wrapper with two or more children, for
/// example `translate() { a(); b(); }`, gives one piece for each child.
fn flatten<'a>(ancestors: &[&'a Node], nodes: &[&'a Node], out: &mut Vec<Piece<'a>>) {
    for &n in nodes {
        let kids = n.geometry_children();
        if WRAPPERS.contains(&n.name.as_str()) && kids.len() >= 2 {
            let mut inner = ancestors.to_vec();
            inner.push(n);
            flatten(&inner, &kids, out);
        } else {
            out.push(Piece {
                ancestors: ancestors.to_vec(),
                node: n,
            });
        }
    }
}

/// Finds the pieces of the assembly. `None` when the tree has no level with
/// two or more module instances, or when that level has more than `max`
/// pieces.
///
/// The first level is the highest node with two or more children. Then each
/// level of sub-assemblies divides further, up to `levels` levels in total,
/// while the total stays at `max` or less. A level divides fully or not at
/// all.
pub fn split(tree: &Tree, max: usize, levels: usize) -> Option<Vec<Piece<'_>>> {
    let mut pieces = split_below(None, &tree.nodes)?;
    if pieces.len() > max {
        return None;
    }
    for _ in 1..levels {
        let mut next = Vec::new();
        let mut divided = false;
        for p in &pieces {
            match split_below(Some(p.node), &p.node.children) {
                Some(sub) => {
                    divided = true;
                    for s in sub {
                        let mut ancestors = p.ancestors.clone();
                        ancestors.push(p.node);
                        ancestors.extend(s.ancestors);
                        next.push(Piece {
                            ancestors,
                            node: s.node,
                        });
                    }
                }
                None => next.push(p.clone()),
            }
        }
        if !divided || next.len() > max {
            break;
        }
        pieces = next;
    }
    Some(pieces)
}

/// The innermost `color()` that applies to the whole piece, as RGB.
pub fn piece_color(tree: &Tree, piece: &Piece) -> Option<[u8; 3]> {
    let mut path: Vec<&Node> = piece.ancestors.clone();
    let mut n = piece.node;
    loop {
        path.push(n);
        match n.geometry_children().as_slice() {
            [only] if DESCEND.contains(&n.name.as_str()) => n = only,
            _ => break,
        }
    }
    let node = path.iter().rev().find(|n| n.name == "color")?;
    let args = &tree.text[node.header.clone()];
    let inner = args.split_once('[')?.1.split_once(']')?.0;
    let mut rgb = [0u8; 3];
    let mut values = inner.split(',').map(|v| v.trim().parse::<f32>());
    for c in &mut rgb {
        *c = (values.next()?.ok()?.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    Some(rgb)
}

/// The .csg text of one piece: the headers of its ancestors around the
/// piece. `file` paths of `import()` and `surface()` become absolute, based
/// on `dir`, the folder of the model. `uses` are `use <...>` lines to put
/// first, for example for fonts.
pub fn piece_text(tree: &Tree, piece: &Piece, dir: &Path, uses: &[String]) -> String {
    let mut out = String::new();
    for u in uses {
        out.push_str(u);
        out.push('\n');
    }
    for a in &piece.ancestors {
        write_span(tree, a.header.clone(), dir, &mut out);
        out.push_str(" {\n");
    }
    write_span(tree, piece.node.span.clone(), dir, &mut out);
    out.push('\n');
    for _ in &piece.ancestors {
        out.push_str("}\n");
    }
    out
}

/// Copies a span of the tree text. String literals get escapes, so
/// OpenSCAD reads back the same value.
fn write_span(tree: &Tree, span: Range<usize>, dir: &Path, out: &mut String) {
    let first = tree.literals.partition_point(|l| l.span.start < span.start);
    let mut at = span.start;
    for lit in tree.literals[first..]
        .iter()
        .take_while(|l| l.span.end <= span.end)
    {
        out.push_str(&tree.text[at..lit.span.start]);
        let value = &tree.text[lit.span.start + 1..lit.span.end - 1];
        if lit.file {
            out.push_str(&quote(&scad_path(&dir.join(value))));
        } else {
            out.push_str(&quote(value));
        }
        at = lit.span.end;
    }
    out.push_str(&tree.text[at..span.end]);
}

/// A path with forward slashes, which OpenSCAD reads on all systems.
fn scad_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// An OpenSCAD string literal for `value`.
fn quote(value: &str) -> String {
    let mut s = String::with_capacity(value.len() + 2);
    s.push('"');
    for c in value.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '"' => s.push_str("\\\""),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            _ => s.push(c),
        }
    }
    s.push('"');
    s
}

/// `use <font>` lines with absolute paths for the font files that the
/// model file loads with `use <x.ttf>`.
pub fn font_uses(source: &str, dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(i) = rest.find("use") {
        let after = rest[i + 3..].trim_start();
        rest = &rest[i + 3..];
        let Some(inner) = after.strip_prefix('<') else {
            continue;
        };
        let Some((name, _)) = inner.split_once('>') else {
            break;
        };
        let lower = name.to_ascii_lowercase();
        if [".ttf", ".otf", ".ttc"].iter().any(|e| lower.ends_with(e)) {
            let path = dir.join(name.trim());
            if path.is_file() {
                out.push(format!("use <{}>", scad_path(&path)));
            }
        }
    }
    out
}

/// A temp file path that no other render uses.
fn temp_path(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "cocad-split-{}-{}-{tag}",
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

/// Writes the CSG tree of `file` (an absolute path) to `out`.
fn export_csg(
    cfg: &Config,
    file: &Path,
    defines: &[String],
    out: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    let log_path = out.with_extension("log");
    let _cleanup = TempFiles(vec![log_path.clone()]);
    let log_file = std::fs::File::create(&log_path)?;
    let binary = cfg.binary();
    #[allow(unused_mut)]
    let mut cmd = Command::new(&binary);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.args(cfg.args()).arg("-o").arg(out);
    for d in defines {
        cmd.arg("-D").arg(d);
    }
    cmd.arg(file)
        .stdin(Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file);
    // Relative include<> and import() paths resolve from the file's folder.
    if let Some(dir) = file.parent() {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("cannot start '{binary}'"))?;
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("cancelled");
        }
        if let Some(s) = child.try_wait()? {
            break s;
        }
        std::thread::sleep(Duration::from_millis(15));
    };
    if !status.success() {
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        bail!("the CSG export failed ({status})\n{log}");
    }
    Ok(())
}

/// True for the errors of `openscad::render` that mean the piece has no
/// 3D geometry, for example an empty or 2D piece.
fn is_empty_error(e: &anyhow::Error) -> bool {
    let first = e.to_string();
    let first = first.lines().next().unwrap_or("");
    [
        "top level object is empty",
        "not a 3D object",
        "no 3D geometry",
        "wrote no geometry",
    ]
    .iter()
    .any(|m| first.contains(m))
}

/// Renders each part of an assembly on its own. Returns `None` when the
/// CSG tree shows no parts, when it has more than `MAX_PIECES` pieces at the
/// first level, or when fewer than two pieces have 3D geometry. The app
/// keeps the shells of the main mesh then.
///
/// The parts are "Part 1", "Part 2", ... in tree order. `color` is the
/// innermost `color()` of the part when the tree has one, else white. It is
/// only a hint: the part has no color of its own.
pub fn render(
    cfg: &Config,
    file: &Path,
    defines: &[String],
    cancel: &Arc<AtomicBool>,
) -> Result<Option<Vec<Part>>> {
    let file = openscad::absolute(file)?;
    let dir = file.parent().unwrap_or(Path::new("")).to_path_buf();
    let csg_path = temp_path("tree.csg");
    let mut temp = TempFiles(vec![csg_path.clone()]);
    export_csg(cfg, &file, defines, &csg_path, cancel)?;
    let text = std::fs::read_to_string(&csg_path).context("cannot read the CSG export")?;
    // The app can still show the shells, so a tree that this parser cannot
    // read is no error.
    let Ok(tree) = parse(text) else {
        return Ok(None);
    };
    let Some(pieces) = split(&tree, MAX_PIECES, LEVELS) else {
        return Ok(None);
    };
    let source = std::fs::read_to_string(&file).unwrap_or_default();
    let uses = font_uses(&source, &dir);
    let mut jobs = Vec::with_capacity(pieces.len());
    for (i, p) in pieces.iter().enumerate() {
        let path = temp_path(&format!("piece{}.csg", i + 1));
        temp.0.push(path.clone());
        std::fs::write(&path, piece_text(&tree, p, &dir, &uses))?;
        let color = piece_color(&tree, p).unwrap_or([0xff; 3]);
        jobs.push((path, color));
    }

    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let results: Mutex<Vec<(usize, Result<Option<Mesh>>)>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(jobs.len());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    if failed.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((path, _)) = jobs.get(i) else {
                        break;
                    };
                    let r = match openscad::render(cfg, path, &[], cancel) {
                        Ok(out) => Ok(Some(out.mesh)),
                        Err(e) if is_empty_error(&e) => Ok(None),
                        Err(e) => {
                            failed.store(true, Ordering::Relaxed);
                            Err(e)
                        }
                    };
                    results.lock().unwrap().push((i, r));
                }
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        bail!("cancelled");
    }
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(i, _)| *i);
    let mut parts = Vec::new();
    for (i, r) in results {
        match r {
            Ok(Some(mesh)) => parts.push(Part {
                name: format!("Part {}", parts.len() + 1),
                color: jobs[i].1,
                mesh,
            }),
            // An empty or 2D piece.
            Ok(None) => {}
            Err(e) => return Err(e.context(format!("piece {} of {}", i + 1, jobs.len()))),
        }
    }
    Ok((parts.len() >= 2).then_some(parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(text: &str) -> Tree {
        parse(text.to_string()).unwrap()
    }

    fn names<'a>(t: &'a Tree, pieces: &[Piece<'a>]) -> Vec<&'a str> {
        pieces
            .iter()
            .map(|p| &t.text[p.node.header.clone()])
            .collect()
    }

    #[test]
    fn parser_reads_strings_with_braces_and_semicolons() {
        let t = tree(concat!(
            "group() {\n",
            "\ttext(text = \"a;{b}(c\", font = \"x\");\n",
            "\ttext(text = \"q\\\", size = 1);\n",
            "\ttext(text = \"x\"y\", size = 1);\n",
            "}\n",
        ));
        assert_eq!(t.nodes.len(), 1);
        let kids = &t.nodes[0].children;
        assert_eq!(kids.len(), 3);
        assert!(
            kids.iter()
                .all(|k| k.name == "text" && k.children.is_empty())
        );
        assert_eq!(t.literals.len(), 4);
        // Raw values get escapes when written back.
        let piece = Piece {
            ancestors: vec![&t.nodes[0]],
            node: &kids[1],
        };
        let text = piece_text(&t, &piece, Path::new("/m"), &[]);
        assert!(text.contains("text = \"q\\\\\", size = 1"), "{text}");
        let piece = Piece {
            ancestors: vec![],
            node: &kids[2],
        };
        let text = piece_text(&t, &piece, Path::new("/m"), &[]);
        assert!(text.contains("text = \"x\\\"y\""), "{text}");
    }

    #[test]
    fn parser_reads_nested_children_and_modifiers() {
        let t = tree(concat!(
            "group();\n",
            "%\t\tgroup() {\n\t\tcube(size = [1, 1, 1], center = false);\n\t}\n",
            "#multmatrix([[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]) {\n",
            "\tdifference() {\n\t\tsphere(r = 2);\n\t\tcube(size = 1);\n\t}\n",
            "}\n",
            "*group() {\n\tcube(size = 1);\n}\n",
        ));
        let n = &t.nodes;
        assert_eq!(n.len(), 4);
        assert_eq!(n[1].modifiers, "%");
        assert_eq!(&t.text[n[1].header.clone()], "group()");
        assert!(t.text[n[1].span.clone()].starts_with('%'));
        assert_eq!(n[2].modifiers, "#");
        assert_eq!(n[2].children[0].children.len(), 2);
        assert_eq!(n[3].modifiers, "*");
        let geometry: Vec<bool> = n.iter().map(Node::has_geometry).collect();
        assert_eq!(geometry, [false, false, true, false]);
        assert!(parse("group() {\n".into()).is_err());
        assert!(parse("cube(size = 1)".into()).is_err());
    }

    /// The structure of the metal-wire-reel model.
    const REEL: &str = "\
group() {
\tgroup() {
\t\tcolor([0.5, 0.8, 0.9, 1]) {
\t\t\tgroup() {
\t\t\t\tdifference() {
\t\t\t\t\tcylinder(h = 2, r = 5);
\t\t\t\t\tcylinder(h = 2, r = 1);
\t\t\t\t}
\t\t\t}
\t\t}
\t\tcolor([0.5, 0.8, 0.9, 1]) {
\t\t\tmultmatrix([[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, -1, 0], [0, 0, 0, 1]]) {
\t\t\t\tgroup() {
\t\t\t\t\tcube(size = [1, 1, 1], center = false);
\t\t\t\t}
\t\t\t}
\t\t}
\t\tcolor([0.1, 0.2, 0.3, 1]) {
\t\t\tgroup() {
\t\t\t\tsphere(r = 1);
\t\t\t}
\t\t}
\t\tcolor([0.7, 0.7, 0.7, 1]) {
\t\t\tgroup() {
\t\t\t\tintersection() {
\t\t\t\t\tcube(size = 3);
\t\t\t\t\tsphere(r = 2);
\t\t\t\t}
\t\t\t}
\t\t}
\t\tcolor([0.7, 0.7, 0.7, 1]) {
\t\t\tgroup() {
\t\t\t\tcube(size = 2);
\t\t\t}
\t\t}
\t}
}
";

    #[test]
    fn split_finds_the_colored_module_groups() {
        let t = tree(REEL);
        let pieces = split(&t, MAX_PIECES, LEVELS).unwrap();
        assert_eq!(pieces.len(), 5);
        assert!(pieces.iter().all(|p| p.node.name == "color"));
        assert!(pieces.iter().all(|p| p.ancestors.len() == 2));
        assert_eq!(piece_color(&t, &pieces[2]), Some([26, 51, 77]));
        let text = piece_text(&t, &pieces[1], Path::new("/m"), &[]);
        assert!(text.starts_with("group() {\ngroup() {\ncolor([0.5, 0.8, 0.9, 1]) {"));
        assert!(text.contains("[0, 0, -1, 0]"));
        assert!(text.ends_with("}\n}\n"));
        assert!(parse(text).is_ok());
        assert!(split(&t, 4, LEVELS).is_none());
    }

    #[test]
    fn no_split_for_a_part_made_of_primitives() {
        let t = tree(concat!(
            "group() {\n",
            "\tcube(size = [1, 1, 1], center = false);\n",
            "\tmultmatrix([[1, 0, 0, 5], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]) {\n",
            "\t\tcube(size = [1, 1, 1], center = false);\n",
            "\t}\n",
            "}\n",
        ));
        assert!(split(&t, MAX_PIECES, LEVELS).is_none());
        // Operators stop the split, also with module groups inside.
        let t = tree(concat!(
            "difference() {\n",
            "\tgroup() {\n\t\tcube(size = 2);\n\t}\n",
            "\tgroup() {\n\t\tcube(size = 1);\n\t}\n",
            "}\n",
        ));
        assert!(split(&t, MAX_PIECES, LEVELS).is_none());
        // One module group and one primitive are not an assembly.
        let t = tree("group() {\n\tcube(size = 1);\n}\nsphere(r = 1);\n");
        assert!(split(&t, MAX_PIECES, LEVELS).is_none());
    }

    #[test]
    fn single_child_chain_descends_and_skips_empty_groups() {
        let t = tree(concat!(
            "group();\n",
            "group() {\n\tgroup();\n\tgroup();\n}\n",
            "render(convexity = 2) {\n",
            "\tunion() {\n",
            "\t\tgroup() {\n",
            "\t\t\tgroup() {\n\t\t\t\tcube(size = 1);\n\t\t\t}\n",
            "\t\t\tgroup() {\n\t\t\t\tsphere(r = 1);\n\t\t\t}\n",
            "\t\t\t%group() {\n\t\t\t\tsphere(r = 9);\n\t\t\t}\n",
            "\t\t}\n",
            "\t}\n",
            "}\n",
        ));
        let pieces = split(&t, MAX_PIECES, LEVELS).unwrap();
        assert_eq!(pieces.len(), 2);
        let chain: Vec<&str> = pieces[0]
            .ancestors
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(chain, ["render", "union", "group"]);
        // An explicit union with two modules is one part.
        let t = tree(
            "union() {\n\tgroup() {\n\t\tcube(size = 1);\n\t}\n\tgroup() {\n\t\tsphere(r = 1);\n\t}\n}\n",
        );
        assert!(split(&t, MAX_PIECES, LEVELS).is_none());
    }

    #[test]
    fn multmatrix_with_two_modules_splits_and_keeps_the_transform() {
        let t = tree(concat!(
            "multmatrix([[1, 0, 0, 7], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]) {\n",
            "\tgroup() {\n\t\tcube(size = 1);\n\t}\n",
            "\tcolor([1, 0, 0, 1]) {\n\t\tgroup() {\n\t\t\tsphere(r = 1);\n\t\t}\n\t}\n",
            "}\n",
        ));
        let pieces = split(&t, MAX_PIECES, LEVELS).unwrap();
        assert_eq!(names(&t, &pieces), ["group()", "color([1, 0, 0, 1])"]);
        for p in &pieces {
            let text = piece_text(&t, p, Path::new("/m"), &[]);
            assert!(text.starts_with("multmatrix([[1, 0, 0, 7], "), "{text}");
        }
        assert_eq!(piece_color(&t, &pieces[0]), None);
        assert_eq!(piece_color(&t, &pieces[1]), Some([255, 0, 0]));
    }

    #[test]
    fn wrapper_with_many_children_gives_one_piece_each() {
        // The lantern: a base, and three rings in one translate().
        let t = tree(concat!(
            "group() {\n",
            "\tcolor([0.5, 0.5, 0.5, 1]) {\n\t\tgroup() {\n\t\t\tcube(size = 1);\n\t\t}\n\t}\n",
            "\tmultmatrix([[1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 2], [0, 0, 0, 1]]) {\n",
            "\t\tcolor([1, 0, 0, 1]) {\n\t\t\tgroup() {\n\t\t\t\tcube(size = 1);\n\t\t\t}\n\t\t}\n",
            "\t\tcolor([0, 1, 0, 1]) {\n\t\t\tgroup() {\n\t\t\t\tcube(size = 1);\n\t\t\t}\n\t\t}\n",
            "\t}\n",
            "}\n",
        ));
        let pieces = split(&t, MAX_PIECES, LEVELS).unwrap();
        assert_eq!(pieces.len(), 3);
        assert_eq!(pieces[2].ancestors.last().unwrap().name, "multmatrix");
        assert_eq!(piece_color(&t, &pieces[2]), Some([0, 255, 0]));
    }

    #[test]
    fn sub_assemblies_divide_while_the_total_fits() {
        // A board with two module groups, and an army of three pieces.
        // The first board group is a part made of two modules.
        let t = tree(concat!(
            "group() {\n",
            "\tgroup() {\n",
            "\t\tgroup() {\n",
            "\t\t\tgroup() {\n\t\t\t\tcube(size = 1);\n\t\t\t}\n",
            "\t\t\tgroup() {\n\t\t\t\tsphere(r = 1);\n\t\t\t}\n",
            "\t\t}\n",
            "\t\tgroup() {\n\t\t\tcube(size = 2);\n\t\t}\n",
            "\t}\n",
            "\tgroup() {\n",
            "\t\tgroup() {\n\t\t\tsphere(r = 1);\n\t\t}\n",
            "\t\tgroup() {\n\t\t\tsphere(r = 2);\n\t\t}\n",
            "\t\tgroup() {\n\t\t\tsphere(r = 3);\n\t\t}\n",
            "\t}\n",
            "}\n",
        ));
        assert_eq!(split(&t, MAX_PIECES, LEVELS).unwrap().len(), 5);
        // Only the first level.
        assert_eq!(split(&t, MAX_PIECES, 1).unwrap().len(), 2);
        // A third level divides the part made of two modules.
        assert_eq!(split(&t, MAX_PIECES, 3).unwrap().len(), 6);
        // A level that does not fit stays whole.
        assert_eq!(split(&t, 4, LEVELS).unwrap().len(), 2);
        assert!(split(&t, 1, LEVELS).is_none());
    }

    #[test]
    fn relative_import_paths_become_absolute() {
        let t = tree(concat!(
            "group() {\n",
            "\tlinear_extrude(height = 1) {\n",
            "\t\timport(file = \"source/a b.svg\", origin = [0, 0], layer = \"x\");\n",
            "\t}\n",
            "\tsurface(file = \"/abs/h.png\", center = false);\n",
            "\ttext(text = \"file\", font = \"Liberation Sans\");\n",
            "}\n",
        ));
        let piece = Piece {
            ancestors: vec![],
            node: &t.nodes[0],
        };
        let text = piece_text(&t, &piece, Path::new("/models/lantern"), &[]);
        assert!(
            text.contains(
                "import(file = \"/models/lantern/source/a b.svg\", origin = [0, 0], layer = \"x\")"
            ),
            "{text}"
        );
        assert!(text.contains("surface(file = \"/abs/h.png\""), "{text}");
        assert!(text.contains("text(text = \"file\""), "{text}");
        assert_eq!(quote(r#"C:\a "b""#), r#""C:\\a \"b\"""#);
        assert_eq!(scad_path(Path::new(r"C:\m\x.svg")), "C:/m/x.svg");
    }

    #[test]
    fn font_files_from_use_statements() {
        let dir = std::env::temp_dir().join(format!("cocad-split-font-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        std::fs::write(dir.join("fonts/A.ttf"), b"").unwrap();
        let source = "use <fonts/A.ttf>\nuse <lib.scad>\nuse <missing.otf>\n";
        let uses = font_uses(source, &dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            uses,
            [format!("use <{}>", scad_path(&dir.join("fonts/A.ttf")))]
        );
    }

    /// Splits and renders real models. Run with
    /// `COCAD_SPLIT_MODELS=a.scad:b.scad cargo test csg_split -- --ignored --nocapture`.
    /// Without the variable it uses the example models.
    #[test]
    #[ignore]
    fn real_models() {
        let list = std::env::var("COCAD_SPLIT_MODELS").unwrap_or_default();
        let files: Vec<PathBuf> = if list.is_empty() {
            let mut v: Vec<PathBuf> = std::fs::read_dir("examples")
                .unwrap()
                .filter_map(|e| Some(e.ok()?.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "scad"))
                .collect();
            v.sort();
            v
        } else {
            std::env::split_paths(&list).collect()
        };
        let cfg = Config::default();
        let cancel = Arc::new(AtomicBool::new(false));
        for f in files {
            let start = std::time::Instant::now();
            let main = openscad::render(&cfg, &f, &[], &cancel);
            let main_time = start.elapsed();
            let start = std::time::Instant::now();
            let parts = render(&cfg, &f, &[], &cancel);
            let time = start.elapsed();
            println!("== {}", f.display());
            let csg = temp_path("test.csg");
            let file = openscad::absolute(&f).unwrap();
            if export_csg(&cfg, &file, &[], &csg, &cancel).is_ok() {
                let t = parse(std::fs::read_to_string(&csg).unwrap()).unwrap();
                for p in split(&t, MAX_PIECES, LEVELS).unwrap_or_default() {
                    let mut line: String =
                        p.ancestors.iter().map(|a| a.name.clone() + " > ").collect();
                    line += &t.text[p.node.header.clone()];
                    println!("  piece: {}", &line[..line.len().min(110)]);
                }
            }
            let _ = std::fs::remove_file(&csg);
            match &main {
                Ok(m) => println!(
                    "main: {} triangles, volume {:.0}, {:.2} s",
                    m.mesh.triangles.len(),
                    m.mesh.volume(),
                    main_time.as_secs_f32()
                ),
                Err(e) => println!("main: error {}", e.to_string().lines().next().unwrap_or("")),
            }
            match parts {
                Ok(Some(parts)) => {
                    let sum: f32 = parts.iter().map(|p| p.mesh.volume()).sum();
                    println!(
                        "split: {} parts, volume sum {sum:.0}, {:.2} s",
                        parts.len(),
                        time.as_secs_f32()
                    );
                    for p in &parts {
                        println!(
                            "  {}: {} triangles, volume {:.0}, color {:?}",
                            p.name,
                            p.mesh.triangles.len(),
                            p.mesh.volume(),
                            p.color
                        );
                    }
                    if let Ok(m) = &main {
                        // The parts that the 3D view shows.
                        let a = crate::assembly::Assembly::from_parts(&m.mesh, &parts, true);
                        let other: Vec<String> = (a.parts.iter())
                            .filter(|p| p.name.starts_with("Other"))
                            .map(|p| format!("{} tris, {:.2} mm3", p.tris.len(), p.volume))
                            .collect();
                        println!("view: {} parts, other: {other:?}", a.parts.len());
                    }
                }
                Ok(None) => println!("split: none, {:.2} s", time.as_secs_f32()),
                Err(e) => println!("split: error {e:#}"),
            }
        }
    }
}
