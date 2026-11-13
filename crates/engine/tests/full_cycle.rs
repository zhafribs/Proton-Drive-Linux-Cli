//! Full-cycle E2E: a stateful fake Proton CLI stores the remote tree on disk,
//! so one `--once` run exercises list/upload/download/trash/state together.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const FAKE: &str = r#"#!/usr/bin/env python3
import json, os, shutil, sys
from pathlib import Path

store = Path(os.environ["FAKE_REMOTE"])
trash = store.parent / "remote-trash"
trash.mkdir(parents=True, exist_ok=True)
root = "/my-files"
args = sys.argv[1:]

def rel_of(rpath):
    if rpath == root:
        return ""
    if rpath.startswith(root + "/"):
        return rpath[len(root) + 1:]
    return rpath.lstrip("/")

def path_of(rel):
    return store / rel if rel else store

def listing(rpath):
    d = path_of(rel_of(rpath))
    out = []
    if not d.is_dir():
        print("[]")
        return
    for child in sorted(d.iterdir(), key=lambda p: p.name):
        is_dir = child.is_dir()
        item = {
            "name": {"ok": True, "value": child.name},
            "type": "folder" if is_dir else "file",
            "uid": "uid-" + child.name,
            "modificationTime": "2026-01-01T00:00:00.000Z",
        }
        if not is_dir:
            item["activeRevision"] = {"claimedSize": child.stat().st_size}
        out.append(item)
    print(json.dumps(out))

def total():
    size = 0
    count = 0
    for p in store.rglob("*"):
        if p.is_file():
            size += p.stat().st_size
            count += 1
        elif p.is_dir() and p != store:
            count += 1
    print(json.dumps({"size": size, "numberOfDescendants": count}))

if not args:
    sys.exit(0)
cmd = args[0]
if cmd == "fs":
    sub = args[1] if len(args) > 1 else ""
    if sub == "size":
        total()
    elif sub == "list":
        listing(args[2])
    elif sub == "upload":
        if os.environ.get("FAKE_FAIL_UPLOAD") == "1":
            sys.exit(1)
        try:
            flag = args.index("-f")
        except ValueError:
            flag = 2
        files = args[flag + 2:len(args) - 1]
        parent = path_of(rel_of(args[-1]))
        parent.mkdir(parents=True, exist_ok=True)
        for f in files:
            src = Path(f)
            if src.is_file():
                shutil.copy2(src, parent / src.name)
        print("{}")
    elif sub == "download":
        try:
            flag = args.index("-f")
        except ValueError:
            flag = 2
        rpaths = args[2:flag]
        dest = Path(args[-1])
        dest.mkdir(parents=True, exist_ok=True)
        for rpath in rpaths:
            src = path_of(rel_of(rpath))
            if src.is_file():
                shutil.copy2(src, dest / src.name)
        print("{}")
    elif sub == "trash":
        src = path_of(rel_of(args[2]))
        if src.exists():
            target = trash / src.name
            n = 1
            while target.exists():
                n += 1
                target = trash / (src.name + "." + str(n))
            shutil.move(str(src), str(target))
        print("{}")
    elif sub == "create-folder":
        d = path_of(rel_of(args[2]))
        d.mkdir(parents=True, exist_ok=True)
        print("{}")
    elif sub == "move":
        src = path_of(rel_of(args[2]))
        parent = path_of(rel_of(args[3]))
        parent.mkdir(parents=True, exist_ok=True)
        if src.exists():
            shutil.move(str(src), str(parent / args[4]))
        print("{}")
    else:
        print("{}")
else:
    print("{}")
"#;

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("pd-full-cycle-{}-{}", std::process::id(), name))
}

fn setup(name: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let dir = temp(name);
    let _ = fs::remove_dir_all(&dir);
    let conf = dir.join("conf");
    let local = dir.join("local");
    let state = dir.join("state");
    let remote = dir.join("remote");
    fs::create_dir_all(&conf).unwrap();
    fs::create_dir_all(&local).unwrap();
    fs::create_dir_all(&remote).unwrap();
    fs::write(
        conf.join("proton-sync.conf"),
        format!(
            "PROTON_SYNC_LOCAL={}\nPROTON_SYNC_REMOTE=/my-files\nPROTON_SYNC_WORKERS=2\nPROTON_SYNC_TRANSFERS=1\nPROTON_SYNC_RETRIES=1\nPROTON_SYNC_DELETES=1\n",
            local.display()
        ),
    )
    .unwrap();
    let fake = dir.join("fake-proton-drive");
    fs::write(&fake, FAKE).unwrap();
    let mut perms = fs::metadata(&fake).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake, perms).unwrap();
    (dir, conf, local, state, remote)
}

fn cycle(conf: &Path, state: &Path, remote: &Path, fail_upload: bool) -> String {
    let fake = conf.parent().unwrap().join("fake-proton-drive");
    let log = conf.parent().unwrap().join("cli.log");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_proton-engine"));
    cmd.args(["--once"])
        .env("XDG_CONFIG_HOME", conf)
        .env("XDG_STATE_HOME", state)
        .env("PROTON_DRIVE_BIN", &fake)
        .env("FAKE_REMOTE", remote)
        .env("PROTON_CLI_LOG", &log)
        .env("PROTON_GUI_VERSION", "test");
    if fail_upload {
        cmd.env("FAKE_FAIL_UPLOAD", "1");
    }
    let out = cmd.output().expect("engine runs");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "cycle failed\nstdout:\n{}\nstderr:\n{stderr}",
        String::from_utf8_lossy(&out.stdout)
    );
    stderr
}

fn state_file(state: &Path) -> PathBuf {
    state.join("proton-sync/state.json")
}

fn wait_for_state(state: &Path) {
    assert!(state_file(state).is_file(), "state.json was not written");
}

#[test]
fn full_cycle_uploads_downloads_and_propagates_both_deletes() {
    let (dir, conf, local, state, remote) = setup("cycle");
    fs::write(local.join("local.txt"), b"from local").unwrap();
    cycle(&conf, &state, &remote, false);
    assert_eq!(fs::read(remote.join("local.txt")).unwrap(), b"from local");
    wait_for_state(&state);
    let saved = proton_core::state::read(&state_file(&state));
    assert_eq!(saved["local.txt"].l_size, Some(10));

    fs::create_dir_all(remote.join("Docs")).unwrap();
    fs::write(remote.join("Docs/remote.txt"), b"from cloud").unwrap();
    cycle(&conf, &state, &remote, false);
    assert_eq!(fs::read(local.join("Docs/remote.txt")).unwrap(), b"from cloud");
    wait_for_state(&state);

    fs::remove_file(remote.join("Docs/remote.txt")).unwrap();
    cycle(&conf, &state, &remote, false);
    assert!(!local.join("Docs/remote.txt").exists());
    assert!(
        state.join("proton-sync/trash/Docs/remote.txt").is_file(),
        "remote delete must move the local copy to local trash"
    );

    // Keep a second local file so this is a normal one-file delete, not the
    // empty-side wipe guard (which correctly holds a near-empty local tree).
    fs::write(local.join("keep.txt"), b"keep").unwrap();
    cycle(&conf, &state, &remote, false);
    assert!(remote.join("keep.txt").is_file());
    fs::remove_file(local.join("local.txt")).unwrap();
    cycle(&conf, &state, &remote, false);
    assert!(!remote.join("local.txt").exists(), "local delete must trash the cloud copy");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn failed_upload_keeps_the_local_file_and_does_not_advance_state() {
    let (dir, conf, local, state, remote) = setup("upload-fail");
    fs::write(local.join("keep.txt"), b"keep me").unwrap();
    cycle(&conf, &state, &remote, true);
    assert_eq!(fs::read(local.join("keep.txt")).unwrap(), b"keep me");
    assert!(!remote.join("keep.txt").exists());
    let saved = proton_core::state::read(&state_file(&state));
    assert!(
        saved.get("keep.txt").is_none_or(|e| e.l_size.is_none()),
        "a failed upload must not be stamped as synced"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn short_download_never_becomes_the_local_file() {
    let (dir, conf, local, state, remote) = setup("download-short");
    fs::write(remote.join("short.txt"), b"0123456789").unwrap();
    // The fake advertises the true size, then truncates the staged file, so
    // placement must reject it and leave the destination absent.
    let fake = dir.join("fake-proton-drive");
    let mut script = fs::read_to_string(&fake).unwrap();
    script = script.replace(
        "shutil.copy2(src, dest / src.name)",
        "shutil.copy2(src, dest / src.name)\n                with open(dest / src.name, 'r+b') as f:\n                    f.truncate(3)",
    );
    fs::write(&fake, script).unwrap();
    let mut perms = fs::metadata(&fake).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake, perms).unwrap();

    cycle(&conf, &state, &remote, false);
    assert!(!local.join("short.txt").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// Sixty consecutive process cycles with local and cloud churn. The E2E tests
/// prove one transition; this proves the loop does not drift, duplicate or
/// silently stop applying changes over time.
#[test]
fn sixty_cycles_keep_both_sides_and_state_converged() {
    fn files(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut out = std::collections::BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    out.insert(rel, fs::read(&path).unwrap());
                }
            }
        }
        out
    }

    let (dir, conf, local, state, remote) = setup("soak");
    for i in 0..12 {
        fs::write(local.join(format!("seed-{i}.txt")), format!("seed {i}")).unwrap();
    }
    cycle(&conf, &state, &remote, false);

    for i in 0..60 {
        fs::write(
            local.join(format!("soak-{i}.txt")),
            format!("local cycle {i}"),
        )
        .unwrap();
        if i % 3 == 0 {
            fs::write(
                remote.join(format!("cloud-{i}.txt")),
                format!("cloud cycle {i}"),
            )
            .unwrap();
        }
        if i > 10 && i % 5 == 0 {
            let _ = fs::remove_file(remote.join(format!("cloud-{}.txt", i - 10)));
        }
        if i > 10 && i % 7 == 0 {
            let _ = fs::remove_file(local.join(format!("soak-{}.txt", i - 10)));
        }
        cycle(&conf, &state, &remote, false);
    }

    // One settling cycle, then both trees and the state must agree.
    cycle(&conf, &state, &remote, false);
    let l = files(&local);
    let r = files(&remote);
    assert_eq!(l.keys().collect::<Vec<_>>(), r.keys().collect::<Vec<_>>());
    for (name, bytes) in &l {
        assert_eq!(r.get(name), Some(bytes), "{name} differs between sides");
    }
    let saved = proton_core::state::read(&state_file(&state));
    for name in l.keys() {
        assert!(saved.contains_key(name), "state lost {name}");
    }
    assert!(
        !l.keys().any(|n| n.contains("conflict")),
        "a soak cycle created a conflict: {:?}",
        l.keys().filter(|n| n.contains("conflict")).collect::<Vec<_>>()
    );
    let _ = fs::remove_dir_all(&dir);
}
