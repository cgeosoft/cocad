//! The consent record. It stays on this computer, in `<config>/consent`.
//!
//! This file has its own place, so that a window with an old copy of `Config`
//! cannot write an old choice back. Only `Client::set_consent` writes it.
//!
//! A choice is valid only for the notice version that the user saw. When
//! `NOTICE_VERSION` goes up, the app sends nothing until the user answers again.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The version of the consent text and the Privacy Notice. Raise it only when
/// the data, the purpose or the recipient changes.
pub const NOTICE_VERSION: u32 = 2;

const FILE: &str = "consent";

/// Where the user made a choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Setup,
    Settings,
    UpdatePrompt,
}

impl Source {
    fn as_str(self) -> &'static str {
        match self {
            Source::Setup => "setup",
            Source::Settings => "settings",
            Source::UpdatePrompt => "update-prompt",
        }
    }

    fn parse(text: &str) -> Option<Source> {
        match text {
            "setup" => Some(Source::Setup),
            "settings" => Some(Source::Settings),
            "update-prompt" => Some(Source::UpdatePrompt),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choice {
    pub granted: bool,
    /// UTC time of the last change, such as `2026-11-06T21:00:00Z`.
    pub at: String,
    pub source: Source,
}

/// The default has version 0, so that the app asks, and sends nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub notice_version: u32,
    pub analytics: Choice,
    pub errors: Choice,
}

impl Record {
    pub fn allows_analytics(&self) -> bool {
        self.notice_version == NOTICE_VERSION && self.analytics.granted
    }

    pub fn allows_errors(&self) -> bool {
        self.notice_version == NOTICE_VERSION && self.errors.granted
    }

    /// Records the answer. The prompt records both options, also a no. Settings
    /// records only the option that changed.
    pub fn choose(&mut self, analytics: bool, errors: bool, source: Source, now: &str) {
        let prompt = self.notice_version != NOTICE_VERSION;
        if prompt || self.analytics.granted != analytics {
            self.analytics = Choice {
                granted: analytics,
                at: now.to_string(),
                source,
            };
        }
        if prompt || self.errors.granted != errors {
            self.errors = Choice {
                granted: errors,
                at: now.to_string(),
                source,
            };
        }
        self.notice_version = NOTICE_VERSION;
    }

    fn text(&self) -> String {
        format!(
            "notice_version = {}\nanalytics = {}\nanalytics_at = {}\nanalytics_source = {}\n\
             error_reports = {}\nerror_reports_at = {}\nerror_reports_source = {}\n",
            self.notice_version,
            self.analytics.granted,
            self.analytics.at,
            self.analytics.source.as_str(),
            self.errors.granted,
            self.errors.at,
            self.errors.source.as_str(),
        )
    }

    /// `None` if a line is missing or wrong. The caller then asks again.
    fn parse(text: &str) -> Option<Record> {
        let value = |key: &str| {
            text.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim())
        };
        let choice = |key: &str| {
            Some(Choice {
                granted: value(key)?.parse().ok()?,
                at: value(&format!("{key}_at"))?.to_string(),
                source: Source::parse(value(&format!("{key}_source"))?)?,
            })
        };
        Some(Record {
            notice_version: value("notice_version")?.parse().ok()?,
            analytics: choice("analytics")?,
            errors: choice("error_reports")?,
        })
    }
}

/// The record, and where the question comes from if the app must ask.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Loaded {
    pub record: Record,
    pub prompt: Option<Source>,
}

pub fn load(dir: &Path) -> Loaded {
    let Ok(text) = std::fs::read_to_string(dir.join(FILE)) else {
        return migrate(dir);
    };
    let record = Record::parse(&text).unwrap_or_default();
    let prompt = (record.notice_version != NOTICE_VERSION).then_some(Source::UpdatePrompt);
    Loaded { record, prompt }
}

/// Reads the keys that version 0.3.1 kept in `config`. An old "on" applies to
/// analytics only: the old screen described a different error report. The
/// version stays 0, so nothing is sent until the user answers the prompt.
fn migrate(dir: &Path) -> Loaded {
    let text = std::fs::read_to_string(dir.join("config")).unwrap_or_default();
    let on = |key: &str| {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .any(|(k, v)| k.trim() == key && v.trim() == "true")
    };
    if !on("privacy_asked") {
        return Loaded {
            record: Record::default(),
            prompt: Some(Source::Setup),
        };
    }
    let mut record = Record::default();
    record.analytics.granted = on("analytics");
    Loaded {
        record,
        prompt: Some(Source::UpdatePrompt),
    }
}

/// Writes a temporary file and renames it, so a crash leaves no partial record.
pub fn write(dir: &Path, record: &Record) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let temp = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
    std::fs::write(&temp, record.text())?;
    std::fs::rename(&temp, dir.join(FILE)).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

pub fn remove(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(FILE));
}

pub fn mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir.join(FILE)).ok()?.modified().ok()
}

/// The current UTC time in ISO 8601 form.
pub fn now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    iso(secs)
}

fn iso(secs: u64) -> String {
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Civil date from a day count: the algorithm of Howard Hinnant.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::tests::TempDir;

    #[test]
    fn iso_time_is_correct() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(iso(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn record_survives_a_write_and_a_load() {
        let dir = TempDir::new();
        let mut r = Record::default();
        r.choose(true, false, Source::Setup, "2026-11-06T21:00:00Z");
        write(&dir.0, &r).unwrap();
        let loaded = load(&dir.0);
        assert_eq!(loaded.record, r);
        assert_eq!(loaded.prompt, None);
        assert!(r.allows_analytics());
        assert!(!r.allows_errors());
    }

    #[test]
    fn prompt_records_a_no_with_time_source_and_version() {
        let mut r = Record::default();
        r.choose(false, false, Source::Setup, "T1");
        assert_eq!(r.notice_version, NOTICE_VERSION);
        for c in [&r.analytics, &r.errors] {
            assert!(!c.granted);
            assert_eq!(c.at, "T1");
            assert_eq!(c.source, Source::Setup);
        }
    }

    #[test]
    fn settings_stamps_only_the_option_that_changed() {
        let mut r = Record::default();
        r.choose(false, false, Source::Setup, "T1");
        r.choose(true, false, Source::Settings, "T2");
        assert_eq!(r.analytics.at, "T2");
        assert_eq!(r.analytics.source, Source::Settings);
        assert_eq!(r.errors.at, "T1");
        assert_eq!(r.errors.source, Source::Setup);
    }

    #[test]
    fn first_start_asks_with_both_options_off() {
        let dir = TempDir::new();
        let l = load(&dir.0);
        assert_eq!(l.prompt, Some(Source::Setup));
        assert!(!l.record.analytics.granted && !l.record.errors.granted);
    }

    #[test]
    fn old_error_reports_on_never_turns_on_error_reporting() {
        let dir = TempDir::new();
        let old = "theme = dark\nanalytics = false\nerror_reports = true\nprivacy_asked = true\n";
        std::fs::write(dir.0.join("config"), old).unwrap();
        let l = load(&dir.0);
        assert!(!l.record.errors.granted);
        assert!(!l.record.allows_errors());
    }

    #[test]
    fn old_analytics_on_stays_but_sends_nothing_before_the_prompt() {
        let dir = TempDir::new();
        let old = "analytics = true\nerror_reports = true\nprivacy_asked = true\n";
        std::fs::write(dir.0.join("config"), old).unwrap();
        let l = load(&dir.0);
        assert_eq!(l.record.notice_version, 0);
        assert_eq!(l.prompt, Some(Source::UpdatePrompt));
        assert!(l.record.analytics.granted);
        assert!(!l.record.allows_analytics());
    }

    #[test]
    fn a_new_notice_version_asks_again_and_stops_sending() {
        let mut r = Record::default();
        r.choose(true, true, Source::Setup, "T1");
        r.notice_version = NOTICE_VERSION + 1;
        let dir = TempDir::new();
        write(&dir.0, &r).unwrap();
        let l = load(&dir.0);
        assert_eq!(l.prompt, Some(Source::UpdatePrompt));
        assert!(!l.record.allows_analytics() && !l.record.allows_errors());
    }

    #[test]
    fn a_partial_file_reads_as_off() {
        let dir = TempDir::new();
        std::fs::write(dir.0.join(FILE), "notice_version = 1\nanalytics = tr").unwrap();
        let l = load(&dir.0);
        assert_eq!(l.record, Record::default());
        assert_eq!(l.prompt, Some(Source::UpdatePrompt));
    }
}
