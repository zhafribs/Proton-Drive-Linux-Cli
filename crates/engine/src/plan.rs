//! The planner: a pure function of (remote tree, local tree, state).
//!
//! This is a direct port of `plan_cycle` / `detect_moves` from
//! `scripts/proton-sync.py`. Keep the decision order identical: moves first,
//! then the wipe guard, then remote-only / local-only / both-sides in sorted
//! path order. The Python tests and the fixture harness are the contract.

use crate::action::{Action, LocalEntry, LocalTree};
use proton_core::snapshot::{RemoteEntry, RemoteTree};
use proton_core::state::{State, StateEntry};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug)]
pub struct PlanConfig {
    pub propagate_deletes: bool,
    pub wipe_min: usize,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            propagate_deletes: true,
            wipe_min: 5,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PlanFixture {
    #[serde(default)]
    pub remote: RemoteTree,
    #[serde(default)]
    pub local: LocalTree,
    #[serde(default)]
    pub state: State,
    #[serde(default = "default_true")]
    pub propagate_deletes: bool,
    #[serde(default = "default_wipe_min")]
    pub wipe_min: usize,
}

fn default_true() -> bool {
    true
}

fn default_wipe_min() -> usize {
    5
}

impl PlanFixture {
    pub fn config(&self) -> PlanConfig {
        PlanConfig {
            propagate_deletes: self.propagate_deletes,
            wipe_min: self.wipe_min,
        }
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WipeSide {
    Local,
    Cloud,
}

#[derive(Serialize, Clone, Debug)]
pub struct PlanResult {
    pub actions: Vec<Action>,
    pub holds: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub wipe_guard: Vec<WipeSide>,
}

fn depth(rel: &str) -> usize {
    rel.matches('/').count()
}

pub fn ts_ns(iso: Option<&str>) -> Option<i64> {
    let iso = iso?;
    if iso.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()?
        .timestamp_nanos_opt()
}

pub fn local_changed(local: &LocalEntry, state: &StateEntry) -> bool {
    state.l_mtime_ns.is_none() || state.l_mtime_ns != local.mtime_ns() || state.l_size != local.size()
}

pub fn remote_changed(remote: &RemoteEntry, state: &StateEntry) -> bool {
    match remote.mtime.as_deref() {
        Some(mtime) if !mtime.is_empty() => state.r_mtime.as_deref() != Some(mtime),
        _ => false,
    }
}

/// rclone `--track-renames` equivalent. Returns `(local_moves, remote_moves)`,
/// each a list of `(old_rel, new_rel)`.
///
/// * cloud-side moves pair a vanished cloud uid with its new path -> one local
///   rename;
/// * local moves pair a vanished local fingerprint with an identical newcomer
///   -> one server-side rename/move;
/// * a whole folder moved intact collapses into one folder move.
pub fn detect_moves(
    remote: &RemoteTree,
    local: &LocalTree,
    state: &State,
    busy: &BTreeSet<String>,
) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let mut remote_moves = Vec::new();
    let mut local_moves = Vec::new();

    // ---- cloud side: exact match on uid ----
    let mut uid_at: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
    for (rel, node) in remote {
        if let Some(uid) = node.uid.as_deref() {
            uid_at.entry(uid).or_default().push(rel);
        }
    }
    let mut taken = busy.clone();
    let mut old_keys: Vec<&String> = state
        .iter()
        .filter(|(rel, s)| {
            s.r_uid.is_some() && !remote.contains_key(rel.as_str()) && local.contains_key(rel.as_str())
        })
        .map(|(rel, _)| rel)
        .collect();
    old_keys.sort_by_key(|p| depth(p)); // stable: BTreeMap order breaks ties
    for old in old_keys {
        if taken.contains(old.as_str()) {
            continue;
        }
        let Some(uid) = state.get(old.as_str()).and_then(|s| s.r_uid.as_deref()) else {
            continue;
        };
        let Some(candidates) = uid_at.get(uid) else {
            continue;
        };
        let candidates: Vec<&String> = candidates
            .iter()
            .filter(|c| {
                c.as_str() != old.as_str()
                    && !state.contains_key(c.as_str())
                    && !local.contains_key(c.as_str())
                    && !taken.contains(c.as_str())
            })
            .copied()
            .collect();
        if candidates.len() != 1 {
            continue;
        }
        let new = candidates[0];
        // a parent already moving carries its children along
        if taken.iter().any(|t| new.starts_with(&format!("{t}/"))) {
            continue;
        }
        remote_moves.push((old.to_string(), new.to_string()));
        taken.insert(old.to_string());
        taken.insert(new.to_string());
    }

    // ---- local side: exact match on (size, mtime_ns) ----
    let mut key_at: BTreeMap<(u64, i64), Vec<&String>> = BTreeMap::new();
    for (rel, node) in local {
        if let LocalEntry::File { mtime_ns, size } = node {
            key_at.entry((*size, *mtime_ns)).or_default().push(rel);
        }
    }
    let mut taken = busy.clone();
    for (old, new) in &remote_moves {
        taken.insert(old.clone());
        taken.insert(new.clone());
    }
    let mut matched: Vec<(String, String)> = Vec::new();
    let mut old_keys: Vec<&String> = state
        .iter()
        .filter(|(rel, s)| {
            s.l_mtime_ns.is_some()
                && s.l_size.is_some()
                && !local.contains_key(rel.as_str())
                && remote
                    .get(rel.as_str())
                    .is_some_and(|n| n.kind == "file")
        })
        .map(|(rel, _)| rel)
        .collect();
    old_keys.sort_by_key(|p| depth(p));
    for old in old_keys {
        if taken.contains(old.as_str()) {
            continue;
        }
        let Some(s) = state.get(old.as_str()) else {
            continue;
        };
        let key = (s.l_size.unwrap(), s.l_mtime_ns.unwrap());
        let Some(candidates) = key_at.get(&key) else {
            continue;
        };
        let candidates: Vec<&String> = candidates
            .iter()
            .filter(|c| {
                !state.contains_key(c.as_str())
                    && !remote.contains_key(c.as_str())
                    && !taken.contains(c.as_str())
            })
            .copied()
            .collect();
        if candidates.len() == 1 {
            matched.push((old.to_string(), candidates[0].to_string()));
            taken.insert(old.to_string());
            taken.insert(candidates[0].to_string());
        }
    }

    // ---- collapse child moves into one folder move ----
    let mut consumed: BTreeSet<String> = BTreeSet::new();
    let mut old_dirs: Vec<&String> = state
        .iter()
        .filter(|(rel, _)| {
            !local.contains_key(rel.as_str())
                && remote
                    .get(rel.as_str())
                    .is_some_and(|n| n.kind == "dir")
        })
        .map(|(rel, _)| rel)
        .collect();
    old_dirs.sort_by_key(|p| depth(p));
    let new_dirs: Vec<&String> = local
        .iter()
        .filter(|(rel, node)| {
            matches!(node, LocalEntry::Dir)
                && !state.contains_key(rel.as_str())
                && !remote.contains_key(rel.as_str())
        })
        .map(|(rel, _)| rel)
        .collect();

    for old_dir in old_dirs {
        for new_dir in new_dirs.iter().copied() {
            if consumed.contains(old_dir.as_str()) || consumed.contains(new_dir.as_str()) {
                continue;
            }
            let pairs: Vec<(String, String)> = matched
                .iter()
                .filter(|(o, n)| {
                    o.starts_with(&format!("{old_dir}/")) && n.starts_with(&format!("{new_dir}/"))
                })
                .cloned()
                .collect();
            if pairs.is_empty() {
                continue;
            }
            let mut suf_old = BTreeSet::new();
            let mut suf_new = BTreeSet::new();
            for (o, n) in &pairs {
                suf_old.insert(o.strip_prefix(&format!("{old_dir}/")).unwrap());
                suf_new.insert(n.strip_prefix(&format!("{new_dir}/")).unwrap());
            }
            let new_files = local
                .iter()
                .filter(|(rel, node)| node.is_file() && rel.starts_with(&format!("{new_dir}/")))
                .count();
            // collapse only when the whole folder moved intact
            if suf_old != suf_new || pairs.len() != suf_old.len() || new_files != pairs.len() {
                continue;
            }
            local_moves.push((old_dir.to_string(), new_dir.to_string()));
            consumed.insert(old_dir.to_string());
            consumed.insert(new_dir.to_string());
            for (o, n) in pairs {
                consumed.insert(o);
                consumed.insert(n);
            }
        }
    }
    for (old, new) in matched {
        if !consumed.contains(&old) && !consumed.contains(&new) {
            local_moves.push((old, new));
        }
    }

    (local_moves, remote_moves)
}

pub fn plan_cycle(
    remote: &RemoteTree,
    local: &LocalTree,
    state: &State,
    config: &PlanConfig,
) -> PlanResult {
    let mut actions = Vec::new();
    let mut holds = 0;

    // Moves first: cheapest sync (no bytes move) and they pre-empt the
    // delete+recreate interpretation of the same paths.
    let (local_moves, remote_moves) =
        detect_moves(remote, local, state, &BTreeSet::new());
    for (old, new) in &local_moves {
        actions.push(Action::MvRemote {
            old: old.clone(),
            new: new.clone(),
        });
    }
    for (old, new) in &remote_moves {
        actions.push(Action::MvLocal {
            old: old.clone(),
            new: new.clone(),
        });
    }
    let mut moved: BTreeSet<String> = BTreeSet::new();
    for (old, new) in local_moves.iter().chain(remote_moves.iter()) {
        moved.insert(old.clone());
        moved.insert(new.clone());
        for rel in remote.keys().chain(local.keys()) {
            if rel.starts_with(&format!("{old}/")) || rel.starts_with(&format!("{new}/")) {
                moved.insert(rel.clone());
            }
        }
    }

    // Wipe guard: one side scanned empty while state remembers real content.
    let state_local = state
        .values()
        .filter(|s| s.l_mtime_ns.is_some())
        .count();
    let state_remote = state
        .values()
        .filter(|s| s.r_mtime.as_deref().is_some_and(|m| !m.is_empty()))
        .count();
    let local_has_file = local.values().any(LocalEntry::is_file);
    let remote_has_file = remote.values().any(|n| n.kind == "file");
    let local_dead = !local_has_file && state_local >= config.wipe_min && remote_has_file;
    let remote_dead = !remote_has_file && state_remote >= config.wipe_min && local_has_file;
    let wipe_guard = if local_dead {
        vec![WipeSide::Local]
    } else if remote_dead {
        vec![WipeSide::Cloud]
    } else {
        Vec::new()
    };

    let mut keys: BTreeSet<&str> = BTreeSet::new();
    keys.extend(remote.keys().map(String::as_str));
    keys.extend(local.keys().map(String::as_str));
    let mut keys: Vec<&str> = keys.into_iter().collect();
    keys.sort_by_key(|p| depth(p)); // depth first, then BTreeSet order

    let mut trashed: BTreeSet<String> = BTreeSet::new();
    for rel in keys {
        if moved.contains(rel) {
            continue;
        }
        if trashed.iter().any(|p| rel.starts_with(&format!("{p}/"))) {
            continue; // child rides along inside its trashed folder
        }
        let r = remote.get(rel);
        let l = local.get(rel);
        let s = state.get(rel);

        if let Some(r) = r {
            if r.doc {
                holds += 1; // Proton Doc/Sheet: the CLI cannot download it
                continue;
            }
        }

        // ---- only on the remote side ----
        if let (Some(r), None) = (r, l) {
            match s {
                None => actions.push(Action::Dl {
                    rel: rel.to_string(),
                    node: r.clone(),
                }),
                Some(s) => {
                    if r.kind == "dir"
                        && s.r_uid == r.uid
                        && config.propagate_deletes
                        && !local_dead
                    {
                        actions.push(Action::TrashRemote {
                            rel: rel.to_string(),
                            node: r.clone(),
                        });
                        trashed.insert(rel.to_string());
                    } else if config.propagate_deletes
                        && !local_dead
                        && s.l_mtime_ns.is_some()
                    {
                        actions.push(Action::TrashRemote {
                            rel: rel.to_string(),
                            node: r.clone(),
                        });
                    } else if remote_changed(r, s) {
                        actions.push(Action::Dl {
                            rel: rel.to_string(),
                            node: r.clone(),
                        });
                    } else {
                        holds += 1;
                    }
                }
            }
            continue;
        }

        // ---- only on the local side ----
        if let (None, Some(l)) = (r, l) {
            match s {
                None => actions.push(Action::Up {
                    rel: rel.to_string(),
                    node: l.clone(),
                }),
                Some(s) => {
                    if config.propagate_deletes
                        && !remote_dead
                        && s.r_mtime.as_deref().is_some_and(|m| !m.is_empty())
                    {
                        actions.push(Action::TrashLocal {
                            rel: rel.to_string(),
                            node: l.clone(),
                        });
                    } else if l.is_file() && local_changed(l, s) {
                        actions.push(Action::Up {
                            rel: rel.to_string(),
                            node: l.clone(),
                        });
                    } else {
                        holds += 1;
                    }
                }
            }
            continue;
        }

        // ---- present on both sides ----
        if let (Some(r), Some(l)) = (r, l) {
            if l.is_file() {
                match s {
                    None => {
                        let local_size = l.size().unwrap_or(0);
                        if r.size.unwrap_or(0) == local_size {
                            holds += 1; // identical: nothing to do
                        } else if l.mtime_ns().unwrap_or(0)
                            > ts_ns(r.mtime.as_deref()).unwrap_or(0)
                        {
                            actions.push(Action::Up {
                                rel: rel.to_string(),
                                node: l.clone(),
                            });
                        } else {
                            actions.push(Action::Dl {
                                rel: rel.to_string(),
                                node: r.clone(),
                            });
                        }
                    }
                    Some(s) => {
                        let lc = local_changed(l, s);
                        let rc = remote_changed(r, s);
                        if lc && rc {
                            if r.size.unwrap_or(0) == l.size().unwrap_or(0) {
                                holds += 1; // same bytes, different clocks
                            } else {
                                actions.push(Action::Conflict {
                                    rel: rel.to_string(),
                                    remote: r.clone(),
                                    local: l.clone(),
                                });
                            }
                        } else if lc {
                            actions.push(Action::Up {
                                rel: rel.to_string(),
                                node: l.clone(),
                            });
                        } else if rc {
                            actions.push(Action::Dl {
                                rel: rel.to_string(),
                                node: r.clone(),
                            });
                        } else {
                            holds += 1;
                        }
                    }
                }
            } else {
                holds += 1; // folders: presence handled above
            }
        }
    }

    PlanResult {
        actions,
        holds,
        wipe_guard,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_file(size: u64, mtime_ns: i64) -> LocalEntry {
        LocalEntry::File { size, mtime_ns }
    }

    fn remote_file(uid: &str, size: u64, mtime: &str) -> RemoteEntry {
        RemoteEntry {
            uid: Some(uid.into()),
            kind: "file".into(),
            mtime: Some(mtime.into()),
            size: Some(size),
            doc: false,
        }
    }

    fn remote_dir(uid: &str) -> RemoteEntry {
        RemoteEntry {
            uid: Some(uid.into()),
            kind: "dir".into(),
            mtime: None,
            size: Some(0),
            doc: false,
        }
    }

    fn state(l: Option<(u64, i64)>, r: Option<(&str, &str)>) -> StateEntry {
        StateEntry {
            l_size: l.map(|x| x.0),
            l_mtime_ns: l.map(|x| x.1),
            r_mtime: r.map(|x| x.0.to_string()),
            r_uid: r.map(|x| x.1.to_string()),
        }
    }

    fn plan(remote: RemoteTree, local: LocalTree, state: State) -> PlanResult {
        plan_cycle(&remote, &local, &state, &PlanConfig::default())
    }

    #[test]
    fn first_meeting_holds_when_sizes_match() {
        let r = BTreeMap::from([("a.txt".into(), remote_file("u", 3, "2026-01-01T00:00:00Z"))]);
        let l = BTreeMap::from([("a.txt".into(), local_file(3, 1))]);
        let out = plan(r, l, State::new());
        assert!(out.actions.is_empty());
        assert_eq!(out.holds, 1);
    }

    #[test]
    fn new_files_sync_in_both_directions() {
        let r = BTreeMap::from([("remote.txt".into(), remote_file("u", 1, "2026-01-01T00:00:00Z"))]);
        let l = BTreeMap::from([("local.txt".into(), local_file(1, 1))]);
        let out = plan(r, l, State::new());
        assert_eq!(out.actions.len(), 2);
        // paths are ordered depth-then-name, so local.txt comes first
        assert_eq!(out.actions[0].kind(), "up");
        assert_eq!(out.actions[0].rel(), "local.txt");
        assert_eq!(out.actions[1].kind(), "dl");
        assert_eq!(out.actions[1].rel(), "remote.txt");
    }

    #[test]
    fn local_edit_uploads_and_remote_edit_downloads() {
        let remote_old = "2026-01-01T00:00:00Z";
        let r = BTreeMap::from([
            ("down.txt".into(), remote_file("u1", 2, "2026-02-01T00:00:00Z")),
            ("up.txt".into(), remote_file("u2", 2, remote_old)),
        ]);
        let l = BTreeMap::from([
            ("down.txt".into(), local_file(1, 1)),
            ("up.txt".into(), local_file(2, 2)),
        ]);
        let st = BTreeMap::from([
            ("down.txt".into(), state(Some((1, 1)), Some((remote_old, "u1")))),
            ("up.txt".into(), state(Some((1, 1)), Some((remote_old, "u2")))),
        ]);
        let out = plan(r, l, st);
        assert_eq!(out.actions.len(), 2);
        assert_eq!(out.actions[0].kind(), "dl");
        assert_eq!(out.actions[0].rel(), "down.txt");
        assert_eq!(out.actions[1].kind(), "up");
        assert_eq!(out.actions[1].rel(), "up.txt");
    }

    #[test]
    fn both_edited_conflicts_unless_sizes_match() {
        let r = BTreeMap::from([
            ("conf.txt".into(), remote_file("u1", 2, "2026-02-01T00:00:00Z")),
            ("same.txt".into(), remote_file("u2", 2, "2026-02-01T00:00:00Z")),
        ]);
        let l = BTreeMap::from([
            ("conf.txt".into(), local_file(3, 2)),
            ("same.txt".into(), local_file(2, 2)),
        ]);
        let st = BTreeMap::from([
            ("conf.txt".into(), state(Some((1, 1)), Some(("2026-01-01T00:00:00Z", "u1")))),
            ("same.txt".into(), state(Some((1, 1)), Some(("2026-01-01T00:00:00Z", "u2")))),
        ]);
        let out = plan(r, l, st);
        assert_eq!(out.actions.len(), 1);
        assert_eq!(out.actions[0].kind(), "conflict");
        assert_eq!(out.actions[0].rel(), "conf.txt");
        assert_eq!(out.holds, 1);
    }

    #[test]
    fn deletes_on_trashes_both_sides() {
        // Fewer than wipe_min state rows, so the wipe guard stays idle.
        let r = BTreeMap::from([(
            "deleted-locally.txt".into(),
            remote_file("u1", 1, "2026-01-01T00:00:00Z"),
        )]);
        let l = BTreeMap::from([("deleted-remotely.txt".into(), local_file(1, 1))]);
        let st = BTreeMap::from([
            (
                "deleted-locally.txt".into(),
                state(Some((1, 1)), Some(("2026-01-01T00:00:00Z", "u1"))),
            ),
            (
                "deleted-remotely.txt".into(),
                state(Some((1, 1)), Some(("2026-01-01T00:00:00Z", "u2"))),
            ),
        ]);
        let out = plan(r, l, st);
        assert_eq!(out.actions.len(), 2);
        assert_eq!(out.actions[0].kind(), "trash_remote");
        assert_eq!(out.actions[0].rel(), "deleted-locally.txt");
        assert_eq!(out.actions[1].kind(), "trash_local");
        assert_eq!(out.actions[1].rel(), "deleted-remotely.txt");
    }

    #[test]
    fn deletes_off_holds_local_delete_instead_of_trashing_remote() {
        let r = BTreeMap::from([("a.txt".into(), remote_file("u", 1, "2026-01-01T00:00:00Z"))]);
        let l = LocalTree::new();
        let st = BTreeMap::from([(
            "a.txt".into(),
            state(Some((1, 1)), Some(("2026-01-01T00:00:00Z", "u"))),
        )]);
        let out = plan_cycle(
            &r,
            &l,
            &st,
            &PlanConfig {
                propagate_deletes: false,
                ..Default::default()
            },
        );
        assert!(out.actions.is_empty());
        assert_eq!(out.holds, 1);
    }

    #[test]
    fn wipe_guard_holds_deletions_when_one_side_is_empty() {
        let mut r = RemoteTree::new();
        let mut st = State::new();
        for i in 0..5 {
            let rel = format!("f{i}.txt");
            r.insert(rel.clone(), remote_file(&format!("u{i}"), 1, "2026-01-01T00:00:00Z"));
            st.insert(rel, state(Some((1, 1)), Some(("2026-01-01T00:00:00Z", "u"))));
        }
        let out = plan(r, LocalTree::new(), st);
        assert_eq!(out.wipe_guard, vec![WipeSide::Local]);
        assert!(out.actions.is_empty());
        assert_eq!(out.holds, 5);
    }

    #[test]
    fn docs_are_held() {
        let r = BTreeMap::from([(
            "doc".into(),
            RemoteEntry {
                doc: true,
                ..remote_file("u", 0, "2026-01-01T00:00:00Z")
            },
        )]);
        let l = BTreeMap::from([("doc".into(), local_file(0, 1))]);
        let out = plan(r, l, State::new());
        assert!(out.actions.is_empty());
        assert_eq!(out.holds, 1);
    }

    #[test]
    fn local_move_becomes_one_remote_rename() {
        let r = BTreeMap::from([("old.txt".into(), remote_file("u", 3, "2026-01-01T00:00:00Z"))]);
        let l = BTreeMap::from([("new.txt".into(), local_file(3, 7))]);
        let st = BTreeMap::from([(
            "old.txt".into(),
            state(Some((3, 7)), Some(("2026-01-01T00:00:00Z", "u"))),
        )]);
        let out = plan(r, l, st);
        assert_eq!(out.actions.len(), 1);
        assert_eq!(
            out.actions[0],
            Action::MvRemote {
                old: "old.txt".into(),
                new: "new.txt".into()
            }
        );
    }

    #[test]
    fn remote_move_becomes_one_local_rename() {
        let r = BTreeMap::from([("new.txt".into(), remote_file("u", 3, "2026-01-01T00:00:00Z"))]);
        let l = BTreeMap::from([("old.txt".into(), local_file(3, 7))]);
        let st = BTreeMap::from([(
            "old.txt".into(),
            state(Some((3, 7)), Some(("2026-01-01T00:00:00Z", "u"))),
        )]);
        let out = plan(r, l, st);
        assert_eq!(out.actions.len(), 1);
        assert_eq!(
            out.actions[0],
            Action::MvLocal {
                old: "old.txt".into(),
                new: "new.txt".into()
            }
        );
    }

    #[test]
    fn folder_move_collapses_into_one_move() {
        let r = BTreeMap::from([
            ("old".into(), remote_dir("d-old")),
            ("old/a.txt".into(), remote_file("u1", 3, "2026-01-01T00:00:00Z")),
        ]);
        let l = BTreeMap::from([
            ("new".into(), LocalEntry::Dir),
            ("new/a.txt".into(), local_file(3, 7)),
        ]);
        let st = BTreeMap::from([
            ("old".into(), state(None, Some(("", "d-old")))),
            (
                "old/a.txt".into(),
                state(Some((3, 7)), Some(("2026-01-01T00:00:00Z", "u1"))),
            ),
        ]);
        let out = plan(r, l, st);
        assert_eq!(out.actions.len(), 1);
        assert_eq!(
            out.actions[0],
            Action::MvRemote {
                old: "old".into(),
                new: "new".into()
            }
        );
    }

    #[test]
    fn ts_ns_parses_cli_timestamps() {
        assert!(ts_ns(Some("2026-09-12T05:04:56.000Z")).is_some());
        assert_eq!(ts_ns(None), None);
        assert_eq!(ts_ns(Some("")), None);
        assert_eq!(ts_ns(Some("nonsense")), None);
    }
}
