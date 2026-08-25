//! Client-side entry filtering.

use anyhow::{Result, bail};
use regex::Regex;

use crate::record::Record;

/// Filters applied to parsed entries before they enter the ordering buffer.
///
/// Unit and priority narrowing is also pushed down to the remote
/// `journalctl`; applying it again locally keeps the output correct when a
/// remote emits more than asked, for example right after a cursor-based
/// resume.
pub struct Filters {
    units: Vec<String>,
    priority_limit: Option<u8>,
    grep: Option<Regex>,
}

impl Filters {
    pub fn new(units: &[String], priority_limit: Option<u8>, grep: Option<Regex>) -> Self {
        Self {
            units: units.iter().map(|u| normalize_unit(u)).collect(),
            priority_limit,
            grep,
        }
    }

    /// Returns whether the entry should be shown.
    ///
    /// Entries without a `PRIORITY` field pass a priority filter: hiding an
    /// entry the user cannot know exists is worse than showing an unlabeled
    /// one.
    pub fn matches(&self, record: &Record) -> bool {
        if !self.units.is_empty() {
            let Some(unit) = &record.unit else {
                return false;
            };
            if !self.units.iter().any(|u| u == unit) {
                return false;
            }
        }
        if let (Some(limit), Some(p)) = (self.priority_limit, record.priority) {
            if p > limit {
                return false;
            }
        }
        if let Some(re) = &self.grep {
            if !re.is_match(&record.message) {
                return false;
            }
        }
        true
    }
}

/// Expands a bare unit name to its `.service` form, matching what systemctl
/// does; names that already carry a unit type suffix pass through.
pub fn normalize_unit(unit: &str) -> String {
    if unit.contains('.') {
        unit.to_string()
    } else {
        format!("{unit}.service")
    }
}

/// Converts a syslog priority name or numeric level into its numeric value.
pub fn parse_priority(s: &str) -> Result<u8> {
    let p = match s {
        "emerg" | "panic" => 0,
        "alert" => 1,
        "crit" => 2,
        "err" | "error" => 3,
        "warning" | "warn" => 4,
        "notice" => 5,
        "info" => 6,
        "debug" => 7,
        other => match other.parse::<u8>() {
            Ok(n) if n <= 7 => n,
            _ => bail!("invalid priority {other:?}; expected 0-7 or a syslog level name"),
        },
    };
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(unit: Option<&str>, priority: Option<u8>, message: &str) -> Record {
        Record {
            host: "h".into(),
            timestamp_us: 0,
            cursor: None,
            unit: unit.map(Into::into),
            pid: None,
            priority,
            message: message.into(),
        }
    }

    #[test]
    fn bare_unit_names_match_their_service() {
        let f = Filters::new(&["backend".into()], None, None);
        assert!(f.matches(&record(Some("backend.service"), None, "")));
        assert!(!f.matches(&record(Some("nginx.service"), None, "")));
        assert!(!f.matches(&record(None, None, "")));
    }

    #[test]
    fn suffixed_unit_names_pass_through() {
        assert_eq!(normalize_unit("docker.socket"), "docker.socket");
        assert_eq!(normalize_unit("backend"), "backend.service");
    }

    #[test]
    fn priority_limit_hides_less_severe_entries() {
        let f = Filters::new(&[], Some(4), None);
        assert!(f.matches(&record(None, Some(3), "")));
        assert!(f.matches(&record(None, Some(4), "")));
        assert!(!f.matches(&record(None, Some(6), "")));
        assert!(f.matches(&record(None, None, "")));
    }

    #[test]
    fn grep_matches_the_message() {
        let f = Filters::new(&[], None, Some(Regex::new("Out.fMemory").unwrap()));
        assert!(f.matches(&record(None, None, "java.lang.OutOfMemoryError")));
        assert!(!f.matches(&record(None, None, "all good")));
    }

    #[test]
    fn priority_names_and_numbers_parse() {
        assert_eq!(parse_priority("warning").unwrap(), 4);
        assert_eq!(parse_priority("err").unwrap(), 3);
        assert_eq!(parse_priority("7").unwrap(), 7);
        assert!(parse_priority("8").is_err());
        assert!(parse_priority("loud").is_err());
    }
}
