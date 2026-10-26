//! Timestamp helpers shared by every persisted format.

use chrono::{DateTime, FixedOffset};

/// RFC 3339 / `journalctl short-iso` timestamps:
/// `2026-10-19T23:22:01+08:00`.
pub fn parse_iso(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn parses_journal_stamp() {
        let dt = parse_iso("2026-10-19T23:22:01+08:00").unwrap();
        assert_eq!(dt.offset().local_minus_utc(), 8 * 3600);
        assert_eq!(dt.hour(), 23);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_iso("").is_none());
        assert!(parse_iso("not a date").is_none());
    }
}
