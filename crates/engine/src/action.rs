//! Actions and the trees they operate on.
//!
//! The serde shapes intentionally mirror the Python planner's tuples:
//! `("up", rel, node)` becomes `{"kind":"up","rel":...,"node":...}` and moves
//! become `{"kind":"mv_remote","old":...,"new":...}`. That is what lets the
//! parity harness compare the two engines' plans without translation tables.

use proton_core::snapshot::{RemoteEntry, RemoteTree};
use proton_core::state::{State, StateEntry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type LocalTree = BTreeMap<String, LocalEntry>;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LocalEntry {
    Dir,
    File { mtime_ns: i64, size: u64 },
}

impl LocalEntry {
    pub fn is_file(&self) -> bool {
        matches!(self, LocalEntry::File { .. })
    }

    pub fn mtime_ns(&self) -> Option<i64> {
        match self {
            LocalEntry::File { mtime_ns, .. } => Some(*mtime_ns),
            LocalEntry::Dir => None,
        }
    }

    pub fn size(&self) -> Option<u64> {
        match self {
            LocalEntry::File { size, .. } => Some(*size),
            LocalEntry::Dir => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Up {
        rel: String,
        node: LocalEntry,
    },
    Dl {
        rel: String,
        node: RemoteEntry,
    },
    TrashLocal {
        rel: String,
        node: LocalEntry,
    },
    TrashRemote {
        rel: String,
        node: RemoteEntry,
    },
    MvRemote {
        old: String,
        new: String,
    },
    MvLocal {
        old: String,
        new: String,
    },
    Conflict {
        rel: String,
        remote: RemoteEntry,
        local: LocalEntry,
    },
}

impl Action {
    pub fn kind(&self) -> &'static str {
        match self {
            Action::Up { .. } => "up",
            Action::Dl { .. } => "dl",
            Action::TrashLocal { .. } => "trash_local",
            Action::TrashRemote { .. } => "trash_remote",
            Action::MvRemote { .. } => "mv_remote",
            Action::MvLocal { .. } => "mv_local",
            Action::Conflict { .. } => "conflict",
        }
    }

    /// The path a single-path action applies to (moves report their new path).
    pub fn rel(&self) -> &str {
        match self {
            Action::Up { rel, .. }
            | Action::Dl { rel, .. }
            | Action::TrashLocal { rel, .. }
            | Action::TrashRemote { rel, .. }
            | Action::Conflict { rel, .. } => rel,
            Action::MvRemote { new, .. } | Action::MvLocal { new, .. } => new,
        }
    }
}

// ------------------------------------------------------------------ policies

/// Deletes, moves and conflicts wait while the remote cache is being audited.
/// Edits of files already known to exist remotely wait too; brand-new local
/// files/folders and downloads are safe.
pub fn hold_actions_for_audit(
    actions: Vec<Action>,
    remote: &RemoteTree,
    state: &State,
) -> (Vec<Action>, usize) {
    let mut allowed = Vec::new();
    let mut held = 0;
    for action in actions {
        match &action {
            Action::TrashRemote { .. }
            | Action::TrashLocal { .. }
            | Action::MvRemote { .. }
            | Action::MvLocal { .. }
            | Action::Conflict { .. } => {
                held += 1;
                continue;
            }
            Action::Up {
                rel,
                node: LocalEntry::File { .. },
            } => {
                let prior = state.get(rel);
                let known_remote = prior
                    .is_some_and(|s: &StateEntry| s.r_mtime.as_deref().is_some_and(|m| !m.is_empty()))
                    || remote.contains_key(rel);
                if known_remote {
                    held += 1;
                    continue;
                }
            }
            _ => {}
        }
        allowed.push(action);
    }
    (allowed, held)
}

/// One CLI batch per parent and direction, in action order.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TransferBatch {
    pub parent: String,
    pub uploads: Vec<String>,
    pub downloads: Vec<String>,
    pub bytes: u64,
}

pub fn group_transfer_batches(actions: &[Action]) -> Vec<TransferBatch> {
    let mut batches: Vec<TransferBatch> = Vec::new();
    for action in actions {
        let (upload, rel, size) = match action {
            Action::Up {
                rel,
                node: LocalEntry::File { size, .. },
            } => (true, rel, *size),
            Action::Dl { rel, node } if node.kind == "file" => {
                (false, rel, node.size.unwrap_or(0))
            }
            // Directory downloads/mkdirs are whole-subtree CLI calls, not
            // per-file batches; Python's group_transfer_batches skips them.
            _ => continue,
        };
        let parent = match rel.rsplit_once('/') {
            Some((p, _)) => p.to_string(),
            None => String::new(),
        };
        let slot = batches.iter_mut().find(|b| {
            b.parent == parent
                && if upload {
                    !b.uploads.is_empty()
                } else {
                    !b.downloads.is_empty()
                }
        });
        let batch = match slot {
            Some(b) => b,
            None => {
                batches.push(TransferBatch {
                    parent,
                    uploads: Vec::new(),
                    downloads: Vec::new(),
                    bytes: 0,
                });
                batches.last_mut().unwrap()
            }
        };
        if upload {
            batch.uploads.push(rel.clone());
        } else {
            batch.downloads.push(rel.clone());
        }
        batch.bytes += size;
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(size: u64, mtime_ns: i64) -> LocalEntry {
        LocalEntry::File { size, mtime_ns }
    }

    fn remote_file(size: u64, mtime: &str) -> RemoteEntry {
        RemoteEntry {
            uid: Some("uid".into()),
            kind: "file".into(),
            mtime: Some(mtime.into()),
            size: Some(size),
            doc: false,
        }
    }

    #[test]
    fn action_json_matches_python_tuple_shape() {
        let action = Action::Up {
            rel: "a/one.txt".into(),
            node: file(3, 4),
        };
        let v = serde_json::to_value(&action).unwrap();
        assert_eq!(v["kind"], serde_json::json!("up"));
        assert_eq!(v["rel"], serde_json::json!("a/one.txt"));
        assert_eq!(v["node"]["type"], serde_json::json!("file"));
        assert_eq!(v["node"]["size"], serde_json::json!(3));

        let mv = Action::MvRemote {
            old: "a".into(),
            new: "b".into(),
        };
        let v = serde_json::to_value(&mv).unwrap();
        assert_eq!(v, serde_json::json!({"kind":"mv_remote","old":"a","new":"b"}));
    }

    #[test]
    fn audit_holds_deletes_moves_and_conflicts() {
        let actions = vec![
            Action::TrashRemote {
                rel: "old.txt".into(),
                node: RemoteEntry::default(),
            },
            Action::TrashLocal {
                rel: "old2.txt".into(),
                node: LocalEntry::Dir,
            },
            Action::MvRemote {
                old: "old.txt".into(),
                new: "new.txt".into(),
            },
            Action::MvLocal {
                old: "old2.txt".into(),
                new: "new2.txt".into(),
            },
            Action::Conflict {
                rel: "conf.txt".into(),
                remote: RemoteEntry::default(),
                local: LocalEntry::Dir,
            },
        ];
        let (allowed, held) = hold_actions_for_audit(actions, &RemoteTree::new(), &State::new());
        assert!(allowed.is_empty());
        assert_eq!(held, 5);
    }

    #[test]
    fn audit_holds_edits_of_known_files_but_allows_new_ones() {
        let remote = BTreeMap::from([("known.txt".to_string(), remote_file(1, "x"))]);
        let state = BTreeMap::from([(
            "edit.txt".to_string(),
            StateEntry {
                r_mtime: Some("2026-01-01T00:00:00Z".into()),
                ..Default::default()
            },
        )]);
        let actions = vec![
            Action::Up {
                rel: "edit.txt".into(),
                node: file(2, 2),
            },
            Action::Up {
                rel: "new.txt".into(),
                node: file(2, 2),
            },
            Action::Up {
                rel: "known.txt".into(),
                node: file(1, 1),
            },
            Action::Up {
                rel: "newdir".into(),
                node: LocalEntry::Dir,
            },
        ];
        let (allowed, held) = hold_actions_for_audit(actions, &remote, &state);
        assert_eq!(held, 2);
        assert_eq!(allowed.len(), 2);
        assert_eq!(allowed[0].rel(), "new.txt");
        assert_eq!(allowed[1].rel(), "newdir");
    }

    #[test]
    fn batches_group_by_parent_and_keep_directions_apart() {
        let actions = vec![
            Action::Up {
                rel: "a/one.txt".into(),
                node: file(1, 1),
            },
            Action::Up {
                rel: "a/two.txt".into(),
                node: file(2, 1),
            },
            Action::Dl {
                rel: "a/remote.txt".into(),
                node: remote_file(3, "x"),
            },
            Action::Up {
                rel: "b/three.txt".into(),
                node: file(4, 1),
            },
        ];
        let batches = group_transfer_batches(&actions);
        assert_eq!(batches.len(), 3);
        let up_a = batches
            .iter()
            .find(|b| b.parent == "a" && !b.uploads.is_empty())
            .unwrap();
        assert_eq!(up_a.uploads, vec!["a/one.txt", "a/two.txt"]);
        assert_eq!(up_a.bytes, 3);
        let dl_a = batches
            .iter()
            .find(|b| b.parent == "a" && !b.downloads.is_empty())
            .unwrap();
        assert_eq!(dl_a.downloads, vec!["a/remote.txt"]);
        let up_b = batches.iter().find(|b| b.parent == "b").unwrap();
        assert_eq!(up_b.uploads, vec!["b/three.txt"]);
    }

    #[test]
    fn batches_skip_directory_downloads() {
        let actions = vec![
            Action::Dl {
                rel: "whole-dir".into(),
                node: RemoteEntry {
                    uid: Some("d".into()),
                    kind: "dir".into(),
                    ..Default::default()
                },
            },
            Action::Dl {
                rel: "a/file.txt".into(),
                node: remote_file(7, "x"),
            },
        ];
        let batches = group_transfer_batches(&actions);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].downloads, vec!["a/file.txt"]);
        assert_eq!(batches[0].bytes, 7);
    }
}
