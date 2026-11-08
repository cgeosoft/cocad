//! Checks GitHub for a newer release and tells the user.
//!
//! The app does not download or install a release. The user gets a link to the
//! download section of the website. Only the window checks: the CLI and the
//! `cocad mcp` process do not. A debug build does not check, unless it has a
//! test address.

use eframe::egui;
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

const RELEASES_URL: &str = "https://api.github.com/repos/cgeosoft/cocad/releases?per_page=100";
/// Only for tests against a local server.
const TEST_URL: Option<&str> = option_env!("COCAD_RELEASES_URL");
pub const DOWNLOAD_URL: &str = "https://cocad.cgeosoft.com/#downloads";
const TIMEOUT: Duration = Duration::from_secs(10);
/// A window that stays open checks again after this time.
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// A release number. The order of the fields gives the order of the versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64, u64, u64);

impl Version {
    /// Reads `1.2.3` or `v1.2.3`. A suffix such as `-rc1` gives `None`.
    fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
        let mut number = || parts.next()?.parse().ok();
        let version = Version(number()?, number()?, number()?);
        parts.next().is_none().then_some(version)
    }

    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).expect("package version has three numbers")
    }

    fn same_minor(self, other: Version) -> bool {
        (self.0, self.1) == (other.0, other.1)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// One published version and the bullets of its release notes.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    pub changes: Vec<String>,
}

/// The bullets of a release note, without the authors and links that GitHub adds.
fn changes(notes: &str) -> Vec<String> {
    let mut list = Vec::new();
    let mut contributors = false;
    for line in notes.lines().map(str::trim) {
        if line.starts_with('#') {
            contributors = line.contains("New Contributors");
        }
        let bullet = line.strip_prefix("* ").or(line.strip_prefix("- "));
        if let Some(text) = bullet.filter(|_| !contributors) {
            let text = text.rsplit_once(" by @").map_or(text, |(text, _)| text);
            list.push(text.trim().to_string());
        }
    }
    list
}

/// The published versions in a GitHub answer, newest first.
fn parse_releases(json: &str) -> Vec<Release> {
    let value: Value = serde_json::from_str(json).unwrap_or_default();
    let mut list: Vec<Release> = value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|r| r["draft"] != true && r["prerelease"] != true)
        .filter_map(|r| {
            Some(Release {
                version: Version::parse(r["tag_name"].as_str()?)?,
                changes: changes(r["body"].as_str().unwrap_or_default()),
            })
        })
        .collect();
    list.sort_by_key(|r| std::cmp::Reverse(r.version));
    list
}

/// The releases to show, newest first. Empty when `installed` is the latest.
/// For a latest version of 0.4.5 this is 0.4.0 to 0.4.5, plus all versions
/// between `installed` and 0.4.0.
fn to_show(mut list: Vec<Release>, installed: Version) -> Vec<Release> {
    let Some(latest) = list.first().map(|r| r.version) else {
        return list;
    };
    if latest <= installed {
        return Vec::new();
    }
    list.retain(|r| r.version > installed || r.version.same_minor(latest));
    list
}

fn fetch(url: &str) -> Result<String, ureq::Error> {
    let agent = crate::net::agent(TIMEOUT);
    agent
        .get(url)
        .header("User-Agent", concat!("cocad/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_to_string()
}

fn dismissed_file() -> Option<PathBuf> {
    Some(crate::openscad::config_dir()?.join("update_dismissed"))
}

fn load_dismissed() -> Option<Version> {
    Version::parse(&std::fs::read_to_string(dismissed_file()?).ok()?)
}

fn save_dismissed(version: Version) {
    let Some(file) = dismissed_file() else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, format!("{version}\n"));
}

/// The state of the update check in one window.
pub struct Checker {
    url: Option<&'static str>,
    job: Option<Receiver<Option<Vec<Release>>>>,
    checked: Option<Instant>,
    /// The releases to show, newest first.
    releases: Vec<Release>,
    dismissed: Option<Version>,
}

impl Checker {
    pub fn new() -> Checker {
        Checker {
            url: TEST_URL.or((!cfg!(debug_assertions)).then_some(RELEASES_URL)),
            job: None,
            checked: None,
            releases: Vec::new(),
            dismissed: load_dismissed(),
        }
    }

    /// Starts a check when one is due and takes the result of a finished check.
    /// A failed check changes nothing. The next one comes after `INTERVAL`.
    pub fn poll(&mut self, ctx: &egui::Context, enabled: bool) {
        if let Some(job) = &self.job
            && let Ok(result) = job.try_recv()
        {
            self.job = None;
            if let Some(list) = result {
                self.releases = to_show(list, Version::current());
            }
        }
        let due = self.checked.is_none_or(|t| t.elapsed() > INTERVAL);
        let Some(url) = self.url.filter(|_| enabled && due && self.job.is_none()) else {
            return;
        };
        self.checked = Some(Instant::now());
        let (tx, rx) = channel();
        self.job = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let list = fetch(url).ok().map(|json| parse_releases(&json));
            let _ = tx.send(list);
            ctx.request_repaint();
        });
    }

    /// The releases that the changes window lists, newest first.
    pub fn releases(&self) -> &[Release] {
        &self.releases
    }

    /// The latest version, unless the user dismissed it or a newer one.
    pub fn offer(&self) -> Option<Version> {
        let latest = self.releases.first()?.version;
        self.dismissed.is_none_or(|d| latest > d).then_some(latest)
    }

    /// Hides the notice until a newer version comes.
    pub fn dismiss(&mut self) {
        if let Some(latest) = self.offer() {
            self.dismissed = Some(latest);
            save_dismissed(latest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> Release {
        Release {
            version: Version::parse(version).unwrap(),
            changes: Vec::new(),
        }
    }

    fn shown(versions: &[&str], installed: &str) -> Vec<String> {
        let list = versions.iter().map(|v| release(v)).collect();
        to_show(list, Version::parse(installed).unwrap())
            .iter()
            .map(|r| r.version.to_string())
            .collect()
    }

    #[test]
    fn version_reads_tags_and_compares_as_numbers() {
        assert_eq!(Version::parse("v0.3.2"), Some(Version(0, 3, 2)));
        assert_eq!(Version::parse("1.2.3\n"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("0.3"), None);
        assert_eq!(Version::parse("0.3.2.1"), None);
        assert_eq!(Version::parse("v0.4.0-rc1"), None);
        assert_eq!(Version::parse(""), None);
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert_eq!(Version(0, 3, 2).to_string(), "0.3.2");
    }

    #[test]
    fn current_version_parses() {
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn changes_drop_github_extras() {
        let notes = "## What's Changed\n\
            * Bump zip from 2.6.1 to 9.0.5 by @dependabot[bot] in https://github.com/x/y/pull/1\r\n\
            - Add a thing\n\
            \n\
            ## New Contributors\n\
            * @dependabot[bot] made their first contribution in https://github.com/x/y/pull/1\n\
            \n\
            **Full Changelog**: https://github.com/x/y/compare/v0.3.1...v0.3.2";
        assert_eq!(
            changes(notes),
            ["Bump zip from 2.6.1 to 9.0.5", "Add a thing"]
        );
    }

    #[test]
    fn changes_of_a_note_without_bullets_are_empty() {
        assert!(changes("**Full Changelog**: https://example.com").is_empty());
    }

    #[test]
    fn releases_skip_drafts_and_prereleases_and_sort() {
        let json = r#"[
            {"tag_name": "v0.3.10", "body": "* Ten"},
            {"tag_name": "v0.3.9", "body": null},
            {"tag_name": "v0.4.0", "prerelease": true},
            {"tag_name": "v0.5.0", "draft": true},
            {"tag_name": "nightly"},
            {"body": "no tag"}
        ]"#;
        let list = parse_releases(json);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].version, Version(0, 3, 10));
        assert_eq!(list[0].changes, ["Ten"]);
        assert_eq!(list[1].version, Version(0, 3, 9));
        assert!(list[1].changes.is_empty());
    }

    #[test]
    fn a_bad_answer_gives_no_releases() {
        assert!(parse_releases("not json").is_empty());
        assert!(parse_releases(r#"{"message": "API rate limit exceeded"}"#).is_empty());
    }

    #[test]
    fn a_patch_lists_its_whole_minor_line() {
        let all = ["0.4.5", "0.4.4", "0.4.0", "0.3.2", "0.3.0"];
        assert_eq!(shown(&all, "0.3.2"), ["0.4.5", "0.4.4", "0.4.0"]);
        assert_eq!(shown(&all, "0.4.4"), ["0.4.5", "0.4.4", "0.4.0"]);
    }

    #[test]
    fn a_minor_lists_only_newer_versions() {
        assert_eq!(shown(&["0.5.0", "0.4.5"], "0.4.5"), ["0.5.0"]);
    }

    #[test]
    fn versions_that_the_user_skipped_stay_in_the_list() {
        let all = ["0.4.1", "0.4.0", "0.3.2", "0.3.0"];
        assert_eq!(shown(&all, "0.3.0"), ["0.4.1", "0.4.0", "0.3.2"]);
    }

    #[test]
    fn nothing_shows_when_the_installed_version_is_the_latest() {
        assert!(shown(&["0.3.2", "0.3.0"], "0.3.2").is_empty());
        assert!(shown(&["0.3.2"], "0.4.0").is_empty());
        assert!(shown(&[], "0.3.2").is_empty());
    }

    fn checker(latest: &str, dismissed: Option<&str>) -> Checker {
        Checker {
            url: None,
            job: None,
            checked: None,
            releases: vec![release(latest)],
            dismissed: dismissed.and_then(Version::parse),
        }
    }

    #[test]
    fn a_dismissed_version_stays_hidden_until_a_newer_one() {
        assert_eq!(checker("0.4.0", None).offer(), Some(Version(0, 4, 0)));
        assert_eq!(checker("0.4.0", Some("0.4.0")).offer(), None);
        assert_eq!(
            checker("0.4.1", Some("0.4.0")).offer(),
            Some(Version(0, 4, 1))
        );
    }
}
