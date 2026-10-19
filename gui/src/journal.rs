//! Engine journal reading (Activity page + notifications): timestamps,
//! bodies, and the UPLOAD/DOWNLOAD/MKDIR/TRASH/CONFLICT change rows.

use crate::util::parse_iso;
use chrono::{DateTime, FixedOffset};
use regex::Regex;
use std::sync::OnceLock;

fn change_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(UPLOAD|DOWNLOAD|MKDIR|TRASH|CONFLICT)\s*(.*)$").unwrap())
}

fn iso_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2})").unwrap()
    })
}

/// Timestamp at the head of a `short-iso` journal line.
pub fn ts(line: &str) -> Option<DateTime<FixedOffset>> {
    iso_re().captures(line).and_then(|c| parse_iso(&c[1]))
}

/// Basename of the change path inside a body (`CONFLICT a/b.md (kept)` ->
/// "b.md", capped at 60 chars); "" when the body carries no change verb.
/// Matches the PyQt notification's `name` computation.
pub fn change_name(body: &str) -> String {
    change_re()
        .captures(body)
        .map(|c| {
            let full = c[2].trim();
            let base = full.rsplit('/').next().unwrap_or(full);
            base.chars().take(60).collect()
        })
        .unwrap_or_default()
}

/// The message part after `]: `.
pub fn body(line: &str) -> &str {
    match line.split_once("]: ") {
        Some((_, b)) => b,
        None => line,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangeKind {
    Uploaded,
    Downloaded,
    FolderCreated,
    Removed,
    Conflict,
}

#[derive(Clone, Debug)]
pub struct ChangeRow {
    pub kind: ChangeKind,
    pub name: String,
    pub rel: String,
    pub ts: Option<DateTime<FixedOffset>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Up,
    Down,
    Other,
    Warn,
}

impl ChangeRow {
    pub fn label(&self) -> &'static str {
        match self.kind {
            ChangeKind::Uploaded => "Uploaded",
            ChangeKind::Downloaded => "Downloaded",
            ChangeKind::FolderCreated => "Folder created",
            ChangeKind::Removed => "Removed",
            ChangeKind::Conflict => "Conflict - both kept",
        }
    }
    /// Row direction/color bucket (the PyQt app's `dir` field).
    pub fn dir(&self) -> Dir {
        match self.kind {
            ChangeKind::Uploaded => Dir::Up,
            ChangeKind::Downloaded => Dir::Down,
            ChangeKind::FolderCreated => Dir::Up,
            ChangeKind::Removed => Dir::Other,
            ChangeKind::Conflict => Dir::Warn,
        }
    }
    /// Short icon-ish glyph (the PyQt UI used colored arrows/marks).
    pub fn glyph(&self) -> &'static str {
        match self.kind {
            ChangeKind::Uploaded => "\u{2191}",   // up
            ChangeKind::Downloaded => "\u{2193}", // down
            ChangeKind::FolderCreated => "\u{2191}",
            ChangeKind::Removed => "\u{d7}",      // x
            ChangeKind::Conflict => "!",          // bang
        }
    }
}

/// Parse journal lines into change rows (chronological, like the PyQt app's
/// `_changes`; lines without a known verb drop out; a trailing "(...)"
/// annotation is stripped from the path).
pub fn parse_changes(lines: &[String]) -> Vec<ChangeRow> {
    let mut rows: Vec<ChangeRow> = Vec::new();
    for ln in lines {
        let b = body(ln);
        if let Some(caps) = change_re().captures(b) {
            let verb = &caps[1];
            let rest = caps[2].trim();
            let kind = match verb {
                "UPLOAD" => ChangeKind::Uploaded,
                "DOWNLOAD" => ChangeKind::Downloaded,
                "MKDIR" => ChangeKind::FolderCreated,
                "TRASH" => ChangeKind::Removed,
                _ => ChangeKind::Conflict,
            };
            let rest = strip_trailing_annotation(rest);
            let name = rest.rsplit('/').next().unwrap_or(&rest).to_string();
            rows.push(ChangeRow { kind, name, rel: rest, ts: ts(ln) });
        }
    }
    rows
}

/// "personel/x.md (kept both)" -> "personel/x.md" (Qt did the same before
/// rendering rows).
fn strip_trailing_annotation(rest: &str) -> String {
    match rest.rfind(" (") {
        Some(i) if rest.ends_with(')') => rest[..i].to_string(),
        _ => rest.to_string(),
    }
}

/// "12:03" if the timestamp carries one, else the date - the Activity row's
/// leading time column.
pub fn short_time(row: &ChangeRow) -> String {
    match row.ts {
        Some(dt) => format!("{:02}:{:02}", dt.hour(), dt.minute()),
        None => String::new(),
    }
}

use chrono::Timelike;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_journal_change_lines() {
        let lines = vec![
            "2026-10-19T23:22:01+08:00 host proton-sync[123]: UPLOAD personel/notes.md".into(),
            "2026-10-19T23:22:05+08:00 host proton-sync[123]: DOWNLOAD personel/pic.png".into(),
            "2026-10-19T23:22:09+08:00 host proton-sync[123]: TRASH remote personel/old.txt".into(),
            "2026-10-19T23:22:12+08:00 host proton-sync[123]: MKDIR personel/newdir".into(),
            "2026-10-19T23:22:15+08:00 host proton-sync[123]: CONFLICT personel/both.md (kept both)".into(),
            "2026-10-19T23:22:16+08:00 host proton-sync[123]: cycle complete, no changes".into(),
        ];
        let rows = parse_changes(&lines);
        assert_eq!(rows.len(), 5);
        // chronological order, like the PyQt app's _changes
        assert_eq!(rows[0].kind, ChangeKind::Uploaded);
        assert_eq!(rows[0].name, "notes.md");
        assert_eq!(rows[0].dir(), Dir::Up);
        assert_eq!(rows[1].kind, ChangeKind::Downloaded);
        assert_eq!(rows[1].dir(), Dir::Down);
        assert_eq!(rows[2].kind, ChangeKind::Removed);
        assert_eq!(rows[2].dir(), Dir::Other);
        assert_eq!(rows[3].rel, "personel/newdir");
        assert_eq!(rows[4].kind, ChangeKind::Conflict);
        // trailing "(...)" annotation stripped
        assert_eq!(rows[4].rel, "personel/both.md");
        assert_eq!(rows[4].dir(), Dir::Warn);
        assert_eq!(short_time(&rows[0]), "23:22");
    }

    #[test]
    fn body_splits_after_bracket() {
        assert_eq!(
            body("2026-10-19T23:22:01+08:00 h u[1]: UPLOAD x/y.md"),
            "UPLOAD x/y.md"
        );
    }
}
