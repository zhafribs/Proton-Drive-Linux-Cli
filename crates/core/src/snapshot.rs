//! `remote-snapshot.json` - the cached remote walk plus its change signal.
//!
//! The Python engine writes it compactly (`separators=(",", ":")`), and only
//! trusts it while `sig` matches the effective selection/exclusion settings.
//! Relative paths are the keys; a node is a directory or file with the remote
//! uid/mtime/size the CLI reports.

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
    pub tree: RemoteTree,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct RemoteEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    /// "dir" or "file".
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default)]
    pub doc: bool,
}

/// The settings that change what a remote walk returns.
pub fn walk_signature(remote_root: &str, subfolders: &str, exclude: &str) -> String {
    format!("{remote_root}|{subfolders}|{exclude}")
}

/// Parse a snapshot, returning `None` for a missing or malformed file.
pub fn read(path: &Path) -> Option<Snapshot> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
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
    fn signature_matches_python() {
        assert_eq!(walk_signature("/my-files", "", ""), "/my-files||");
        assert_eq!(
            walk_signature("/my-files", "Docs,Photos", "*.tmp"),
            "/my-files|Docs,Photos|*.tmp"
        );
    }
}
