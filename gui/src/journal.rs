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
/// Shares the row parser's cleanup (scope words, trailing slash,
/// annotations), so folder and conflict notifications show real names.
pub fn change_name(body: &str) -> String {
    change_re()
        .captures(body)
        .map(|c| {
            let full = clean_change_body(&strip_trailing_annotation(c[2].trim()));
            let base = full.rsplit('/').next().unwrap_or(&full);
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
            let rest = clean_change_body(&strip_trailing_annotation(rest));
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

/// Reduce an engine change body to the plain path. The engine logs extra
/// shapes the row UI must not show verbatim:
/// * type/scope words - `dir  Personel/ (whole subtree)`,
///   `TRASH remote personel/x`, `TRASH local  x`, `MKDIR  remote x`;
/// * a trailing `/` on whole-subtree downloads (which used to leave the
///   name empty and made Activity render just "/").
///
/// A conflict's `: remote kept as ...` tail is cut at the exact phrase.
/// Root files genuinely named `dir x.md` lose their first word - the same
/// trade-off Qt made - paths never begin with those words plus a space in
/// practice.
fn clean_change_body(rest: &str) -> String {
    let mut s = rest.to_string();
    if let Some(i) = s.find(": remote kept as ") {
        s.truncate(i);
    }
    loop {
        let trimmed = s.trim_start();
        let stripped = ["dir", "file", "remote", "local"]
            .iter()
            .find_map(|w| trimmed.strip_prefix(w))
            .filter(|r| r.starts_with(' ') || r.starts_with('\t'));
        match stripped {
            Some(r) => s = r.to_string(),
            None => break,
        }
    }
    s.trim().trim_end_matches('/').to_string()
}

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
            "2026-10-20T19:38:32+08:00 host proton-sync[123]: DOWNLOAD dir  UUM-Bungalow/ (whole subtree)".into(),
        ];
        let rows = parse_changes(&lines);
        assert_eq!(rows.len(), 6);
        // chronological order, like the PyQt app's _changes
        assert_eq!(rows[0].kind, ChangeKind::Uploaded);
        assert_eq!(rows[0].name, "notes.md");
        assert_eq!(rows[0].dir(), Dir::Up);
        assert_eq!(rows[1].kind, ChangeKind::Downloaded);
        assert_eq!(rows[1].dir(), Dir::Down);
        assert_eq!(rows[2].kind, ChangeKind::Removed);
        assert_eq!(rows[2].dir(), Dir::Other);
        // "TRASH remote ..." scope word stripped, name kept
        assert_eq!(rows[2].name, "old.txt");
        assert_eq!(rows[2].rel, "personel/old.txt");
        assert_eq!(rows[3].rel, "personel/newdir");
        assert_eq!(rows[4].kind, ChangeKind::Conflict);
        // trailing "(...)" annotation stripped
        assert_eq!(rows[4].rel, "personel/both.md");
        assert_eq!(rows[4].dir(), Dir::Warn);
        // whole-subtree download: "dir " token + trailing slash gone, so the
        // row has a real name (it used to render blank on Home and "/" on
        // Activity)
        assert_eq!(rows[5].kind, ChangeKind::Downloaded);
        assert_eq!(rows[5].name, "UUM-Bungalow");
        assert_eq!(rows[5].rel, "UUM-Bungalow");
    }

    #[test]
    fn change_names_cover_engine_shapes() {
        // plain file path
        assert_eq!(change_name("UPLOAD personel/notes.md"), "notes.md");
        // subtree download: type token, trailing slash, annotation
        assert_eq!(
            change_name("DOWNLOAD dir  Personel/ (whole subtree)"),
            "Personel"
        );
        // conflict: engine's colon tail and the "(kept ...)" form
        assert_eq!(
            change_name("CONFLICT personel/both.md: remote kept as 'both (conflict 1700)'"),
            "both.md"
        );
        assert_eq!(change_name("CONFLICT a/b.md (kept)"), "b.md");
        // no verb at all
        assert_eq!(change_name("cycle complete, no changes"), "");
    }

    #[test]
    fn body_splits_after_bracket() {
        assert_eq!(
            body("2026-10-19T23:22:01+08:00 h u[1]: UPLOAD x/y.md"),
            "UPLOAD x/y.md"
        );
    }
    #[test]
    fn malformed_lines_are_skipped_not_panics() {
        let lines = vec![
            "no timestamp at all".into(),
            "2026-10-19T23:22:00+08:00 host app[1]: unrelated message".into(),
            "2026-10-19T23:22:01+08:00 host proton-sync[1]: DOWNLOAD a/b.txt (kept both)".into(),
        ];
        let rows = parse_changes(&lines);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].rel, "a/b.txt");
        assert_eq!(rows[0].name, "b.txt");
    }

    /// Fuzz-lite: random lines must never panic the change parser.
    #[test]
    fn fuzz_parse_changes_never_panics() {
        let mut x: u64 = 0x9E3779B97F4A7C15;
        let mut rand = || {
            x ^= x << 13; x ^= x >> 7; x ^= x << 17;
            x
        };
        for _ in 0..2000 {
            let len = (rand() % 140) as usize;
            let mut s = String::with_capacity(len);
            for _ in 0..len {
                s.push((32 + (rand() % 95) as u8) as char);
            }
            let _ = parse_changes(&[s]);
        }
    }

}
