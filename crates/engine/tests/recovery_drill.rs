//! Recovery drill: an interrupted download leaves a staging folder, then a
//! later start finds a corrupt `state.json`. The snapshot must restore the
//! history, and planning against the real local file must not delete it.

use proton_core::snapshot::{RemoteEntry, RemoteTree};
use proton_core::state::{self, State, StateEntry};
use proton_core::state_snapshot;
use proton_engine::action::{LocalEntry, LocalTree};
use proton_engine::filter::Filter;
use proton_engine::plan::{plan_cycle, PlanConfig};
use proton_engine::scan::{mtime_ns, scan_local};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "pd-recovery-drill-{}-{}-{name}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ))
}

#[test]
fn corrupt_state_and_a_leftover_stage_never_lose_a_local_file() {
    let state_dir = temp("state");
    let root = temp("root");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("keep.txt"), b"hello").unwrap();
    let meta = fs::symlink_metadata(root.join("keep.txt")).unwrap();
    let mtime = mtime_ns(&meta);
    let remote_mtime = "2026-01-01T00:00:00.000Z";

    let mut local = LocalTree::new();
    local.insert(
        "keep.txt".into(),
        LocalEntry::File {
            mtime_ns: mtime,
            size: 5,
        },
    );
    let mut remote = RemoteTree::new();
    remote.insert(
        "keep.txt".into(),
        RemoteEntry {
            uid: Some("u1".into()),
            kind: "file".into(),
            mtime: Some(remote_mtime.into()),
            size: Some(5),
            doc: false,
        },
    );
    let mut history = State::new();
    history.insert(
        "keep.txt".into(),
        StateEntry {
            l_mtime_ns: Some(mtime),
            l_size: Some(5),
            r_mtime: Some(remote_mtime.into()),
            r_uid: Some("u1".into()),
        },
    );
    fs::create_dir_all(&state_dir).unwrap();
    fs::write(
        state_dir.join("sync-roots"),
        format!("{}\n/my-files\n", root.display()),
    )
    .unwrap();
    state::write(&state_dir.join("state.json"), &history).unwrap();
    let snapshot = state_snapshot::create(&state_dir, "drill").unwrap().unwrap();
    assert!(snapshot.valid && snapshot.roots_match);

    // A download was killed after staging but before placement.
    fs::create_dir_all(root.join(".proton-sync-staging")).unwrap();
    fs::write(root.join(".proton-sync-staging/partial"), b"partial").unwrap();

    // The next start finds a truncated state file. Planning must not run on
    // this; the snapshot is the recovery path.
    fs::write(state_dir.join("state.json"), b"{not-json").unwrap();
    assert!(state::read_checked(&state_dir.join("state.json")).is_err());

    state_snapshot::restore(&state_dir, &snapshot).unwrap();
    let restored = state::read(&state_dir.join("state.json"));
    assert_eq!(restored["keep.txt"].r_uid.as_deref(), Some("u1"));

    let local = scan_local(&root, &Filter::default()).unwrap();
    assert!(!local.contains_key(".proton-sync-staging"));
    assert_eq!(fs::read(root.join("keep.txt")).unwrap(), b"hello");

    let plan = plan_cycle(&remote, &local, &restored, &PlanConfig::default());
    assert!(
        !plan
            .actions
            .iter()
            .any(|a| matches!(a.kind(), "trash_local" | "trash_remote" | "dl")),
        "recovery drill produced a destructive or download action: {:?}",
        plan.actions.iter().map(|a| a.kind()).collect::<Vec<_>>()
    );
    assert_eq!(plan.deletes_held, 0);
    assert_eq!(fs::read(root.join("keep.txt")).unwrap(), b"hello");
    assert!(
        state_snapshot::list(&state_dir)
            .iter()
            .any(|s| s.reason == "pre-restore")
            || fs::read_dir(&state_dir).unwrap().flatten().any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("state.json.corrupt-")
            }),
        "the corrupt state was not preserved before the restore"
    );

    let _ = fs::remove_dir_all(&state_dir);
    let _ = fs::remove_dir_all(&root);
}
