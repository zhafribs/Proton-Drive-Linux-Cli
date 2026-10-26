//! Small shared helpers: time parsing/formatting, version munging.

use chrono::{DateTime, FixedOffset, Utc};

pub use proton_core::time::parse_iso;

/// "just now" / "3 min ago" / "2 h ago" / "1 d ago" - PyQt `rel_time`.
pub fn rel_time(dt: Option<DateTime<FixedOffset>>) -> String {
    let Some(dt) = dt else { return String::new() };
    let now = Utc::now().with_timezone(&dt.timezone());
    let secs = (now - dt).num_seconds().max(0) as i64;
    if secs < 45 {
        "just now".into()
    } else if secs < 3600 {
        format!("{} min ago", (secs / 60).max(1))
    } else if secs < 86400 {
        format!("{} h ago", secs / 3600)
    } else {
        format!("{} d ago", secs / 86400)
    }
}

/// CLI `--version` first line, with the `+build` suffix stripped (Qt did
/// `re.sub(r"\+\w+$", ...)` for the About line).
pub fn strip_build_suffix(v: &str) -> String {
    match v.rsplit_once('+') {
        Some((base, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_alphanumeric()) => {
            base.to_string()
        }
        _ => v.to_string(),
    }
}

/// Parse a version like "0.9.0" into (major, minor, patch), tolerant of a
/// leading "v" and trailing junk.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches('v');
    let mut it = s.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next().unwrap_or("0").parse().unwrap_or(0);
    let patch = it.next().unwrap_or("0").parse().unwrap_or(0);
    Some((major, minor, patch))
}

/// True when `other` is a strictly newer version than `current`.
pub fn is_newer(current: &str, other: &str) -> bool {
    match (parse_version(current), parse_version(other)) {
        (Some(a), Some(b)) => b > a,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn rel_time_buckets() {
        let now = Utc::now().fixed_offset();
        let mk = |secs: i64| Some(now - chrono::Duration::seconds(secs));
        assert_eq!(rel_time(mk(10)), "just now");
        assert_eq!(rel_time(mk(60)), "1 min ago");
        assert_eq!(rel_time(mk(3600)), "1 h ago");
        assert_eq!(rel_time(mk(90_000)), "1 d ago");
        assert_eq!(rel_time(None), "");
    }

    #[test]
    fn iso_parse_of_journal_stamp() {
        let dt = parse_iso("2026-10-19T23:22:01+08:00").unwrap();
        assert_eq!(dt.offset().local_minus_utc(), 8 * 3600);
        assert_eq!(dt.hour(), 23);
    }

    #[test]
    fn build_suffix_stripped() {
        assert_eq!(strip_build_suffix("proton-drive 0.9.0+official"), "proton-drive 0.9.0");
        assert_eq!(strip_build_suffix("0.9.0"), "0.9.0");
    }

    #[test]
    fn newer_version_logic() {
        assert!(is_newer("3.13.2", "3.14.0"));
        assert!(!is_newer("3.14.0", "3.13.2"));
        assert!(!is_newer("3.14.0", "3.14.0"));
        assert!(is_newer("0.9.0", "0.10.0"));
    }
}
