//! `remote-snapshot.json` - the cached remote walk plus its change signal.
//!
//! The Python engine writes it compactly (`separators=(",", ":")`), and only
//! trusts it while `sig` matches the effective selection/exclusion settings.
//! Relative paths are the keys; a node is a directory or file with the remote
//! uid/mtime/size the CLI reports.

use crate::state::State;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

pub type RemoteTree = BTreeMap<String, RemoteEntry>;

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Snapshot {
    /// `remote_root|subfolders|exclude` - see [`walk_signature`].
    pub sig: String,
    /// `fs size` fingerprint `[size, entries]` used for cheap change checks.
    pub fp: Vec<i64>,
    /// Unix seconds when the walk was saved.
    pub ts: f64,
    /// Consecutive cycles that reused this walk without a full re-list.
    /// A full walk resets it; the skip budget bounds same-size renames.
    #[serde(default)]
    pub skips: u32,
    pub tree: RemoteTree,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct RemoteEntry {
    pub uid: Option<String>,
    /// "dir" or "file".
    #[serde(rename = "type")]
    pub kind: String,
    pub mtime: Option<String>,
    pub size: Option<u64>,
    #[serde(default)]
    pub doc: bool,
}

/// The settings that change what a remote walk returns.
pub fn walk_signature(remote_root: &str, subfolders: &str, exclude: &str) -> String {
    format!("{remote_root}|{subfolders}|{exclude}")
}

/// Parse a snapshot, returning `None` for a missing or malformed file.
///
/// `sig`, `fp` and `tree` must be present with the right JSON types, matching
/// the Python `load_snapshot` gate; a snapshot missing its fingerprint is not
/// trustworthy even if the rest parses.
pub fn read(path: &Path) -> Option<Snapshot> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    if !value.get("sig").is_some_and(|v| v.is_string()) {
        return None;
    }
    if !value.get("fp").is_some_and(|v| v.is_array()) {
        return None;
    }
    if !value.get("tree").is_some_and(|v| v.is_object()) {
        return None;
    }
    serde_json::from_value(value).ok()
}

/// Atomically replace the snapshot, compactly like the Python writer.
pub fn write(path: &Path, snapshot: &Snapshot) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name("remote-snapshot.json.tmp");
    let text = serde_json::to_string(snapshot).map_err(io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

impl Snapshot {
    /// The usable remote view: only nodes with a uid and a known type, exactly
    /// like the Python `snapshot_tree` filter.
    pub fn view(&self) -> RemoteTree {
        self.tree
            .iter()
            .filter(|(_, n)| n.uid.is_some() && matches!(n.kind.as_str(), "dir" | "file"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Whether this cached walk can seed a fast start, and why not otherwise.
    /// A cache with entries but no state would make every path look new.
    pub fn usable(&self, state: &State) -> (bool, &'static str) {
        if !self.tree.is_empty() && state.is_empty() {
            return (false, "state empty");
        }
        (true, "ok")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_snapshot_shape() {
        let json = r#"{"sig":"/my-files||","fp":[1692971932,1910],"ts":1793019445.1959643,"tree":{"UUM-Bungalow":{"uid":"uid-1","type":"dir","mtime":"2026-09-11T08:35:35.000Z","size":0,"doc":false},"UUM-Bungalow/x.pdf":{"uid":"uid-2","type":"file","mtime":"2026-09-11T08:00:00.000Z","size":123,"doc":false}}}"#;
        let snap: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snap.sig, "/my-files||");
        assert_eq!(snap.fp, vec![1692971932, 1910]);
        assert_eq!(snap.tree.len(), 2);
        assert_eq!(snap.tree["UUM-Bungalow/x.pdf"].size, Some(123));
        assert_eq!(snap.view().len(), 2);
    }

    #[test]
    fn view_drops_incomplete_nodes() {
        let json = r#"{"sig":"s","fp":[],"ts":0,"tree":{"ok":{"uid":"u","type":"dir"},"none":{"type":"dir"},"bad":{"uid":"u","type":"link"}}}"#;
        let snap: Snapshot = serde_json::from_str(json).unwrap();
        let view = snap.view();
        assert_eq!(view.len(), 1);
        assert!(view.contains_key("ok"));
    }

    #[test]
    fn roundtrip_is_compact() {
        let path = std::env::temp_dir().join(format!("pdsnap-{}.json", std::process::id()));
        let mut snap = Snapshot {
            sig: "/my-files||".into(),
            fp: vec![1, 2],
            ts: 3.5,
            skips: 0,
            tree: RemoteTree::new(),
        };
        snap.tree.insert(
            "a".into(),
            RemoteEntry {
                uid: Some("u".into()),
                kind: "dir".into(),
                mtime: None,
                size: Some(0),
                doc: false,
            },
        );
        write(&path, &snap).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains('\n'));
        assert!(text.starts_with(r#"{"sig":"/my-files||"#));
        let back = read(&path).unwrap();
        assert_eq!(back.sig, snap.sig);
        assert_eq!(back.tree["a"].uid.as_deref(), Some("u"));
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(path.with_file_name("remote-snapshot.json.tmp")).ok();
    }

    #[test]
    fn usable_requires_state_for_a_nonempty_tree() {
        let mut snap = Snapshot::default();
        assert_eq!(snap.usable(&State::new()), (true, "ok"));
        snap.tree.insert(
            "a.txt".into(),
            RemoteEntry {
                uid: Some("u".into()),
                kind: "file".into(),
                ..Default::default()
            },
        );
        assert_eq!(snap.usable(&State::new()), (false, "state empty"));
        let mut state = State::new();
        state.insert("a.txt".into(), Default::default());
        assert_eq!(snap.usable(&state), (true, "ok"));
    }

    #[test]
    fn read_rejects_missing_metadata() {
        let path = std::env::temp_dir().join(format!("pdsnap-bad-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"tree":{}}"#).unwrap();
        assert!(read(&path).is_none()); // no sig, no fp
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn signature_matches_python() {
        assert_eq!(walk_signature("/my-files", "", ""), "/my-files||");
        assert_eq!(
            walk_signature("/my-files", "Docs,Photos", "*.tmp"),
            "/my-files|Docs,Photos|*.tmp"
        );
    }
    #[test]
    fn garbage_snapshot_is_not_usable() {
        let p = std::env::temp_dir().join(format!("pdsnap-garbage-{}.json", std::process::id()));
        std::fs::write(&p, "{truncated").unwrap();
        assert!(read(&p).is_none());
        std::fs::remove_file(&p).ok();
    }

}
