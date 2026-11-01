//! Reads OpenSCAD Customizer parameters from a .scad file.
//!
//! Supported syntax (same as the OpenSCAD Customizer):
//!
//! ```text
//! /* [Group] */
//! // Description on the line above
//! width = 40;           // [10:100]        slider
//! step = 2.5;           // [0:0.5:10]      slider with step
//! shape = "round";      // [round, square] dropdown
//! size = 2;             // [1:Small, 2:Big] dropdown with labels
//! hollow = true;        // checkbox
//! offset = [0, 0, 5];   // [-50:50]        vector, one field per item
//! label = "Hello";      // [12]            text, max length
//! /* [Hidden] */
//! ```
//!
//! Parsing stops at the first top-level `module` or `function`.
//! Assignments with expressions (e.g. `h = w * 2;`) are not parameters.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(f64),
    Bool(bool),
    Str(String),
    Vec(Vec<f64>),
}

impl Value {
    /// The value as OpenSCAD source, for `-D name=value`.
    pub fn to_scad(&self) -> String {
        match self {
            Value::Num(n) => fmt_num(*n),
            Value::Bool(b) => b.to_string(),
            Value::Str(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
            Value::Vec(v) => format!(
                "[{}]",
                v.iter().map(|n| fmt_num(*n)).collect::<Vec<_>>().join(", ")
            ),
        }
    }
}

/// Equal values, ignoring float noise from slider steps.
pub fn same(a: &Value, b: &Value) -> bool {
    let close = |x: f64, y: f64| (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0);
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => close(*x, *y),
        (Value::Vec(x), Value::Vec(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| close(*x, *y))
        }
        _ => a == b,
    }
}

/// Removes float noise such as 140.00000000000003.
pub fn clean(n: f64) -> f64 {
    let r = (n * 1e9).round() / 1e9;
    if r == 0.0 { 0.0 } else { r }
}

fn fmt_num(n: f64) -> String {
    let n = clean(n);
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Widget {
    Plain,
    Slider {
        min: f64,
        max: f64,
        step: Option<f64>,
    },
    Dropdown(Vec<(Value, String)>),
    TextMax(usize),
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub group: String,
    pub description: String,
    pub default: Value,
    pub value: Value,
    pub widget: Widget,
    /// The default was written without a decimal point.
    pub integer: bool,
}

impl Param {
    pub fn changed(&self) -> bool {
        !same(&self.value, &self.default)
    }

    /// Step for sliders and drag fields.
    pub fn step(&self) -> f64 {
        match &self.widget {
            Widget::Slider { step: Some(s), .. } => *s,
            Widget::Slider { min, max, .. }
                if self.integer && min.fract() == 0.0 && max.fract() == 0.0 =>
            {
                1.0
            }
            _ if self.integer => 1.0,
            _ => 0.1,
        }
    }
}

/// Parses all Customizer parameters of a file.
pub fn parse(src: &str) -> Vec<Param> {
    let mut out = Vec::new();
    let mut group = String::from("Parameters");
    let mut depth: i32 = 0;
    let mut in_block = false;
    let mut last_comment = String::new();

    for raw in src.lines() {
        let line = raw.trim();

        if in_block {
            if let Some(end) = line.find("*/") {
                in_block = false;
                depth += brace_delta(&line[end + 2..]);
            }
            continue;
        }

        if depth == 0 {
            if let Some(g) = group_header(line) {
                group = g;
                last_comment.clear();
                continue;
            }
            if line.starts_with("module ")
                || line.starts_with("function ")
                || line == "module"
                || line == "function"
            {
                break;
            }
            if let Some(c) = line.strip_prefix("//") {
                last_comment = c.trim().to_string();
                continue;
            }
            if line.is_empty() {
                last_comment.clear();
                continue;
            }
            if let Some(p) = assignment(line, &group, &last_comment) {
                if group.trim().to_lowercase() != "hidden" {
                    out.push(p);
                }
                last_comment.clear();
                continue;
            }
            last_comment.clear();
        }

        // Track braces and block comments outside of strings.
        let code = strip_line_comment(line);
        if let Some(start) = find_outside_string(code, "/*")
            && find_outside_string(&code[start..], "*/").is_none()
        {
            depth += brace_delta(&code[..start]);
            in_block = true;
            continue;
        }
        depth += brace_delta(code);
        if depth < 0 {
            depth = 0;
        }
    }
    out
}

fn group_header(line: &str) -> Option<String> {
    let inner = line.strip_prefix("/*")?.strip_suffix("*/")?.trim();
    let name = inner.strip_prefix('[')?.strip_suffix(']')?;
    Some(name.trim().to_string())
}

fn brace_delta(code: &str) -> i32 {
    let mut d = 0;
    let mut in_str = false;
    let mut esc = false;
    for c in code.chars() {
        if in_str {
            match (esc, c) {
                (true, _) => esc = false,
                (false, '\\') => esc = true,
                (false, '"') => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => d += 1,
            '}' => d -= 1,
            _ => {}
        }
    }
    d
}

/// Byte index of `pat` outside of string literals.
fn find_outside_string(s: &str, pat: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if in_str {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
        } else if c == b'"' {
            in_str = true;
        } else if s[i..].starts_with(pat) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn strip_line_comment(s: &str) -> &str {
    match find_outside_string(s, "//") {
        Some(i) => &s[..i],
        None => s,
    }
}

fn assignment(line: &str, group: &str, description: &str) -> Option<Param> {
    let eq = line.find('=')?;
    let name = line[..eq].trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        || name.chars().next()?.is_ascii_digit()
        || name.starts_with('$')
    {
        return None;
    }
    let rest = &line[eq + 1..];
    if rest.starts_with('=') {
        return None; // `==`
    }
    let semi = find_outside_string(rest, ";")?;
    let value = parse_value(rest[..semi].trim())?;
    let after = rest[semi + 1..].trim();
    let annotation = after.strip_prefix("//").map(str::trim).unwrap_or("");
    let integer = match &value {
        Value::Num(_) => !rest[..semi].contains(['.', 'e', 'E']),
        Value::Vec(_) => !rest[..semi].contains('.'),
        _ => false,
    };
    let widget = parse_widget(annotation, &value);
    // A trailing comment that is not a widget is a description too.
    let description = if description.is_empty() && widget == Widget::Plain && !annotation.is_empty()
    {
        annotation.to_string()
    } else {
        description.to_string()
    };
    Some(Param {
        name: name.to_string(),
        group: group.to_string(),
        description,
        default: value.clone(),
        value,
        widget,
        integer,
    })
}

fn parse_value(s: &str) -> Option<Value> {
    match s {
        "true" => return Some(Value::Bool(true)),
        "false" => return Some(Value::Bool(false)),
        _ => {}
    }
    if let Some(inner) = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        let mut out = String::new();
        let mut esc = false;
        for c in inner.chars() {
            if esc {
                out.push(match c {
                    'n' => '\n',
                    't' => '\t',
                    c => c,
                });
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                return None; // more than one string literal: an expression
            } else {
                out.push(c);
            }
        }
        return Some(Value::Str(out));
    }
    if let Some(inner) = s.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        let items: Option<Vec<f64>> = inner
            .split(',')
            .map(|x| x.trim().parse::<f64>().ok())
            .collect();
        let items = items?;
        if items.is_empty() || items.len() > 4 {
            return None;
        }
        return Some(Value::Vec(items));
    }
    s.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(Value::Num)
}

fn parse_widget(annotation: &str, value: &Value) -> Widget {
    let Some(inner) = annotation
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
    else {
        return Widget::Plain;
    };
    let inner = inner.trim();

    // Range: [max], [min:max] or [min:step:max]. A label list such as
    // [1:Small, 2:Big] also has ':' but has ',' too.
    if !inner.contains(',') {
        let nums: Option<Vec<f64>> = inner
            .split(':')
            .map(|x| x.trim().parse::<f64>().ok())
            .collect();
        if let Some(n) = nums {
            match (value, n.as_slice()) {
                (Value::Str(_), [max]) => return Widget::TextMax(*max as usize),
                (Value::Num(_) | Value::Vec(_), [max]) => {
                    return Widget::Slider {
                        min: 0.0,
                        max: *max,
                        step: None,
                    };
                }
                (Value::Num(_) | Value::Vec(_), [min, max]) => {
                    return Widget::Slider {
                        min: *min,
                        max: *max,
                        step: None,
                    };
                }
                (Value::Num(_) | Value::Vec(_), [min, step, max]) => {
                    return Widget::Slider {
                        min: *min,
                        max: *max,
                        step: Some(*step),
                    };
                }
                _ => {}
            }
        }
    }

    // Dropdown.
    let mut opts = Vec::new();
    for item in inner.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let (v, label) = match item.split_once(':') {
            Some((v, l)) => (v.trim(), l.trim().to_string()),
            None => (item, item.trim_matches('"').to_string()),
        };
        let val = match value {
            Value::Num(_) => match v.parse::<f64>() {
                Ok(n) => Value::Num(n),
                Err(_) => return Widget::Plain,
            },
            Value::Str(_) => Value::Str(v.trim_matches('"').to_string()),
            _ => return Widget::Plain,
        };
        opts.push((val, label));
    }
    if opts.is_empty() {
        Widget::Plain
    } else {
        Widget::Dropdown(opts)
    }
}

/// `-D` arguments for the parameters that differ from the file defaults.
pub fn defines(params: &[Param]) -> Vec<String> {
    params
        .iter()
        .filter(|p| p.changed())
        .map(|p| format!("{}={}", p.name, p.value.to_scad()))
        .collect()
}

/// Applies a `name=value` override (from the command line) to the parameter list.
pub fn apply_define(params: &mut [Param], def: &str) -> anyhow::Result<()> {
    let (name, val) = def
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("expected name=value, got {def}"))?;
    let p = params
        .iter_mut()
        .find(|p| p.name == name.trim())
        .ok_or_else(|| anyhow::anyhow!("unknown parameter {name}"))?;
    let val = val.trim();
    let v = parse_value(val)
        .or_else(|| matches!(p.default, Value::Str(_)).then(|| Value::Str(val.to_string())))
        .ok_or_else(|| anyhow::anyhow!("cannot parse value {val}"))?;
    p.value = v;
    Ok(())
}

/// Sets a value from Customizer text. String parameters take the text as is.
/// Returns false when the text does not fit the parameter type.
pub fn set_from_text(p: &mut Param, text: &str) -> bool {
    let v = match &p.default {
        Value::Str(_) => Some(Value::Str(text.to_string())),
        _ => parse_value(text.trim()),
    };
    match v {
        Some(v) if std::mem::discriminant(&v) == std::mem::discriminant(&p.default) => {
            p.value = v;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
// Overall width
width = 40; // [10:100]
step = 2.5; // [0:0.5:10]

/* [Shape] */
shape = "round"; // [round, square]
size = 2; // [1:Small, 2:Big]
hollow = true;
offset = [0, 0, 5]; // [-50:50]
label = "a;b // c"; // [12]
derived = width * 2;
wall = 1.2; // wall thickness

/* [Hidden] */
$fn = 64;
secret = 3;

module box() {
    inner = 5;
}
after = 1;
"#;

    #[test]
    fn parses_customizer() {
        let p = parse(SRC);
        let names: Vec<_> = p.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "width", "step", "shape", "size", "hollow", "offset", "label", "wall"
            ]
        );
        assert_eq!(p[0].description, "Overall width");
        assert_eq!(
            p[0].widget,
            Widget::Slider {
                min: 10.0,
                max: 100.0,
                step: None
            }
        );
        assert!(p[0].integer);
        assert_eq!(
            p[1].widget,
            Widget::Slider {
                min: 0.0,
                max: 10.0,
                step: Some(0.5)
            }
        );
        assert_eq!(p[2].group, "Shape");
        assert!(matches!(&p[2].widget, Widget::Dropdown(o) if o.len() == 2));
        assert!(
            matches!(&p[3].widget, Widget::Dropdown(o) if o[1] == (Value::Num(2.0), "Big".into()))
        );
        assert_eq!(p[4].value, Value::Bool(true));
        assert_eq!(p[5].value, Value::Vec(vec![0.0, 0.0, 5.0]));
        assert_eq!(p[6].value, Value::Str("a;b // c".into()));
        assert_eq!(p[6].widget, Widget::TextMax(12));
        assert_eq!(p[7].description, "wall thickness");
    }

    #[test]
    fn defines_only_changed() {
        let mut p = parse(SRC);
        assert!(defines(&p).is_empty());
        apply_define(&mut p, "width=55").unwrap();
        apply_define(&mut p, "shape=square").unwrap();
        assert_eq!(defines(&p), ["width=55", "shape=\"square\""]);
        p[0].value = Value::Num(55.0 + 3e-14);
        assert_eq!(defines(&p)[0], "width=55");
        p[0].value = Value::Num(40.0 + 3e-14);
        assert!(!p[0].changed());
    }
}
