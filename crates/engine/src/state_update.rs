//! `refresh_state`: the pure update of `state.json` after a cycle.
//!
//! The Python docstring is the specification, and every rule here exists
//! because breaking it loses data:
//!
//! * failed / audit-preserved paths keep their old record so they retry;
//! * a both-sides file records the **plan-time** local fingerprint, not the
//!   post-transfer rescan, so an edit that lands mid-cycle is planned again;
//! * a completed download records the fresh local file (that is the new
//!   reality);
//! * one-sided paths with no history and no action get no record;
//! * with deletes ON, a vanished side keeps its last-known values (a held
//!   deletion must still fire next cycle); with deletes OFF it is nulled so an
//!   old deletion never fires if the user enables deletes later.

use crate::action::LocalTree;
use proton_core::snapshot::RemoteTree;
use proton_core::state::{State, StateEntry};
use serde::Deserialize;
use std::collections::BTreeSet;

pub struct RefreshOptions<'a> {
    pub propagate_deletes: bool,
    pub failed: &'a BTreeSet<String>,
    pub acted: &'a BTreeSet<String>,
    pub planned_local: Option<&'a LocalTree>,
    pub downloaded: Option<&'a BTreeSet<String>>,
    pub preserve: Option<&'a BTreeSet<String>>,
}

impl<'a> RefreshOptions<'a> {
    pub fn new(
        propagate_deletes: bool,
        failed: &'a BTreeSet<String>,
        acted: &'a BTreeSet<String>,
    ) -> Self {
        Self {
            propagate_deletes,
            failed,
            acted,
            planned_local: None,
            downloaded: None,
            preserve: None,
        }
    }
}

pub fn refresh_state(
    remote: &RemoteTree,
    local: &LocalTree,
    old_state: &State,
    options: &RefreshOptions<'_>,
) -> State {
    let empty = BTreeSet::new();
    let planned_local = options.planned_local.unwrap_or(local);
    let downloaded = options.downloaded.unwrap_or(&empty);
    let preserve = options.preserve.unwrap_or(&empty);

    let mut keys: BTreeSet<&str> = BTreeSet::new();
    keys.extend(remote.keys().map(String::as_str));
    keys.extend(local.keys().map(String::as_str));

    let mut new_state = State::new();
    for rel in keys {
        let old = old_state.get(rel);
        if options.failed.contains(rel) || preserve.contains(rel) {
            if let Some(old) = old {
                new_state.insert(rel.to_string(), old.clone());
            }
            continue;
        }

        let remote_node = remote.get(rel);
        let local_node = local.get(rel);

        // ---- both sides: record a complete row ----
        if let (Some(remote_node), Some(local_node)) = (remote_node, local_node) {
            let mut entry = StateEntry::default();
            if local_node.is_file() {
                // fresh local after a download; otherwise keep the plan-time
                // view so a mid-cycle edit is not stamped as synced
                let source = if downloaded.contains(rel) {
                    local_node
                } else {
                    planned_local.get(rel).unwrap_or(local_node)
                };
                if source.is_file() {
                    entry.l_mtime_ns = source.mtime_ns();
                    entry.l_size = source.size();
                }
            }
            entry.r_mtime = remote_node.mtime.clone();
            entry.r_uid = remote_node.uid.clone();
            new_state.insert(rel.to_string(), entry);
            continue;
        }

        // ---- one side only ----
        if old.is_none() && !options.acted.contains(rel) {
            continue; // no history, nothing happened: plan it again next cycle
        }

        let mut entry = StateEntry::default();
        let mut has_entry = false;
        if let Some(local_node) = local_node
            && local_node.is_file() {
                entry.l_mtime_ns = local_node.mtime_ns();
                entry.l_size = local_node.size();
                has_entry = true;
            }
        if let Some(remote_node) = remote_node {
            entry.r_mtime = remote_node.mtime.clone();
            entry.r_uid = remote_node.uid.clone();
            has_entry = true;
        }
        if let Some(old) = old {
            if local_node.is_none() && old.l_mtime_ns.is_some() {
                has_entry = true;
                if options.propagate_deletes {
                    entry.l_mtime_ns = old.l_mtime_ns;
                    entry.l_size = old.l_size;
                } else {
                    entry.l_mtime_ns = None;
                    entry.l_size = None;
                }
            }
            if remote_node.is_none() && old.r_mtime.is_some() {
                has_entry = true;
                if options.propagate_deletes {
                    entry.r_mtime = old.r_mtime.clone();
                    entry.r_uid = old.r_uid.clone();
                } else {
                    entry.r_mtime = None;
                    entry.r_uid = None;
                }
            }
        }
        if has_entry {
            new_state.insert(rel.to_string(), entry);
        }
    }
    new_state
}

#[derive(Deserialize)]
pub struct RefreshFixture {
    #[serde(default)]
    pub remote: RemoteTree,
    #[serde(default)]
    pub local: LocalTree,
    #[serde(default)]
    pub old_state: State,
    #[serde(default)]
    pub failed: BTreeSet<String>,
    #[serde(default)]
    pub acted: BTreeSet<String>,
    #[serde(default)]
    pub planned_local: Option<LocalTree>,
    #[serde(default)]
    pub downloaded: BTreeSet<String>,
    #[serde(default)]
    pub preserve: BTreeSet<String>,
    #[serde(default = "default_true")]
    pub propagate_deletes: bool,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::LocalEntry;
    use proton_core::snapshot::RemoteEntry;
    use std::collections::BTreeMap;

    fn lfile(mtime_ns: i64, size: u64) -> LocalEntry {
        LocalEntry::File { mtime_ns, size }
    }

    fn rfile(uid: &str, mtime: &str, size: u64) -> RemoteEntry {
        RemoteEntry {
            uid: Some(uid.into()),
            kind: "file".into(),
            mtime: Some(mtime.into()),
            size: Some(size),
            doc: false,
        }
    }

    fn st(l: Option<(u64, i64)>, r: Option<(&str, &str)>) -> StateEntry {
        StateEntry {
            l_size: l.map(|x| x.0),
            l_mtime_ns: l.map(|x| x.1),
            r_mtime: r.map(|x| x.0.to_string()),
            r_uid: r.map(|x| x.1.to_string()),
        }
    }

    #[test]
    fn both_sides_record_plan_time_fingerprint_not_rescan() {
        let remote = BTreeMap::from([("a".into(), rfile("u", "R", 3))]);
        let local = BTreeMap::from([("a".into(), lfile(2, 3))]);
        let planned = BTreeMap::from([("a".into(), lfile(1, 3))]);
        let failed = BTreeSet::new();
        let acted = BTreeSet::new();
        let mut options = RefreshOptions::new(true, &failed, &acted);
        options.planned_local = Some(&planned);
        let saved = refresh_state(&remote, &local, &State::new(), &options);
        assert_eq!(saved["a"].l_mtime_ns, Some(1));
        assert_eq!(saved["a"].r_mtime.as_deref(), Some("R"));
    }

    #[test]
    fn completed_download_records_fresh_local() {
        let remote = BTreeMap::from([("a".into(), rfile("u", "R", 3))]);
        let local = BTreeMap::from([("a".into(), lfile(2, 3))]);
        let planned = BTreeMap::from([("a".into(), lfile(1, 3))]);
        let failed = BTreeSet::new();
        let acted = BTreeSet::new();
        let downloaded = BTreeSet::from(["a".to_string()]);
        let mut options = RefreshOptions::new(true, &failed, &acted);
        options.planned_local = Some(&planned);
        options.downloaded = Some(&downloaded);
        let saved = refresh_state(&remote, &local, &State::new(), &options);
        assert_eq!(saved["a"].l_mtime_ns, Some(2));
    }

    #[test]
    fn held_paths_keep_their_old_record() {
        let old = BTreeMap::from([(
            "edit.txt".to_string(),
            st(Some((3, 1)), Some(("2026-01-01T00:00:00Z", "u1"))),
        )]);
        let remote = BTreeMap::from([(
            "edit.txt".into(),
            rfile("u1", "2026-01-01T00:00:00Z", 3),
        )]);
        let local = BTreeMap::from([("edit.txt".into(), lfile(2, 4))]);
        let failed = BTreeSet::new();
        let acted = BTreeSet::new();
        let preserve = BTreeSet::from(["edit.txt".to_string()]);
        let mut options = RefreshOptions::new(true, &failed, &acted);
        options.preserve = Some(&preserve);
        let saved = refresh_state(&remote, &local, &old, &options);
        assert_eq!(saved["edit.txt"], old["edit.txt"]);
    }

    #[test]
    fn deletes_on_keeps_vanished_side_for_a_retry() {
        let old = BTreeMap::from([(
            "a".to_string(),
            st(Some((3, 1)), Some(("R", "u"))),
        )]);
        let remote = BTreeMap::from([("a".into(), rfile("u", "R", 3))]);
        let failed = BTreeSet::new();
        let acted = BTreeSet::new();
        let saved = refresh_state(
            &remote,
            &LocalTree::new(),
            &old,
            &RefreshOptions::new(true, &failed, &acted),
        );
        assert_eq!(saved["a"].l_mtime_ns, Some(1));
        assert_eq!(saved["a"].l_size, Some(3));
        assert_eq!(saved["a"].r_uid.as_deref(), Some("u"));
    }

    #[test]
    fn deletes_off_nulls_the_vanished_side() {
        let old = BTreeMap::from([(
            "a".to_string(),
            st(Some((3, 1)), Some(("R", "u"))),
        )]);
        let remote = BTreeMap::from([("a".into(), rfile("u", "R", 3))]);
        let failed = BTreeSet::new();
        let acted = BTreeSet::new();
        let saved = refresh_state(
            &remote,
            &LocalTree::new(),
            &old,
            &RefreshOptions::new(false, &failed, &acted),
        );
        assert_eq!(saved["a"].l_mtime_ns, None);
        assert_eq!(saved["a"].l_size, None);
        assert_eq!(saved["a"].r_uid.as_deref(), Some("u"));
    }

    #[test]
    fn fresh_one_sided_path_needs_an_action_before_it_is_recorded() {
        let remote = BTreeMap::from([("a".into(), rfile("u", "R", 3))]);
        let failed = BTreeSet::new();
        let acted = BTreeSet::new();
        let saved = refresh_state(
            &remote,
            &LocalTree::new(),
            &State::new(),
            &RefreshOptions::new(true, &failed, &acted),
        );
        assert!(saved.is_empty());

        let acted = BTreeSet::from(["a".to_string()]);
        let saved = refresh_state(
            &remote,
            &LocalTree::new(),
            &State::new(),
            &RefreshOptions::new(true, &failed, &acted),
        );
        assert_eq!(saved["a"].r_uid.as_deref(), Some("u"));
    }
}
