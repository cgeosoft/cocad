//! Recently opened models: one path per line in `<config>/recent`, newest first.
//! The last export folder: one path in `<config>/export_dir`.

use std::path::{Path, PathBuf};

const MAX: usize = 10;

fn file() -> Option<PathBuf> {
    Some(crate::openscad::config_dir()?.join("recent"))
}

pub fn load() -> Vec<PathBuf> {
    file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// `list` with `path` first, without duplicates, cut to `MAX`.
fn merged(mut list: Vec<PathBuf>, path: &Path) -> Vec<PathBuf> {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX);
    list
}

fn save(list: &[PathBuf]) {
    let Some(file) = file() else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text: Vec<String> = list.iter().map(|p| p.display().to_string()).collect();
    let _ = std::fs::write(file, text.join("\n") + "\n");
}

/// Records `path` as the newest model and returns the new list.
pub fn add(path: &Path) -> Vec<PathBuf> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let list = merged(load(), &path);
    save(&list);
    list
}

pub fn clear() {
    save(&[]);
}

fn export_file() -> Option<PathBuf> {
    Some(crate::openscad::config_dir()?.join("export_dir"))
}

/// The folder of the last export, when it still exists.
pub fn export_dir() -> Option<PathBuf> {
    let text = std::fs::read_to_string(export_file()?).ok()?;
    let dir = PathBuf::from(text.trim_end_matches(['\r', '\n']));
    dir.is_dir().then_some(dir)
}

/// Records the folder of `path`, the file that was exported.
pub fn set_export_dir(path: &Path) {
    let (Some(file), Some(dir)) = (export_file(), path.parent()) else {
        return;
    };
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(file, dir.display().to_string() + "\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_no_duplicates_capped() {
        let p = |n: usize| PathBuf::from(format!("/m/{n}.scad"));
        let mut list = Vec::new();
        for n in 0..MAX + 3 {
            list = merged(list, &p(n));
        }
        assert_eq!(list.len(), MAX);
        assert_eq!(list[0], p(MAX + 2));
        let list = merged(list, &p(MAX));
        assert_eq!(list[0], p(MAX));
        assert_eq!(list.iter().filter(|x| **x == p(MAX)).count(), 1);
        assert_eq!(list.len(), MAX);
    }
}
