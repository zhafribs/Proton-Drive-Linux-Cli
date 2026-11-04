//! End-to-end plan against a fake official CLI: the engine must plan a
//! correct upload for a local file the (empty) remote does not know, without
//! ever needing the real Proton Drive binary.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn dry_run_plans_upload_against_fake_cli() {
    let dir = std::env::temp_dir().join(format!("pd-e2e-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let conf = dir.join("conf");
    let local = dir.join("local");
    let fake = dir.join("fake-proton-drive");
    fs::create_dir_all(&conf).unwrap();
    fs::create_dir_all(&local).unwrap();
    fs::write(local.join("hello.txt"), b"hi").unwrap();
    fs::write(
        conf.join("proton-sync.conf"),
        format!(
            "PROTON_SYNC_LOCAL={}\nPROTON_SYNC_REMOTE=/my-files\nPROTON_SYNC_WORKERS=2\nPROTON_SYNC_RETRIES=1\n",
            local.display()
        ),
    )
    .unwrap();
    // `fs size` answers the fingerprint; everything else lists nothing.
    fs::write(
        &fake,
        "#!/bin/sh\ncase \"$1\" in\n  fs) case \"$2\" in\n        size) echo '{\"size\":0,\"numberOfDescendants\":0}' ;;\n        *) echo '[]' ;;\n      esac ;;\n  *) echo '[]' ;;\nesac\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&fake).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake, perms).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_proton-engine"))
        .args(["--dry-run"])
        .env("XDG_CONFIG_HOME", &conf)
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("PROTON_DRIVE_BIN", &fake)
        .env("PROTON_GUI_VERSION", "test")
        .output()
        .expect("engine runs");
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    let plan: serde_json::Value = serde_json::from_str(&text).expect("plan is JSON");
    let actions = plan["actions"].as_array().expect("actions array");
    assert!(!actions.is_empty(), "local file must be planned for upload");
    assert_eq!(plan["holds"], 0);

    let _ = fs::remove_dir_all(&dir);
}

/// "Run full check now": the engine must consume the request file *and* list
/// every folder even when the cached snapshot would answer the cycle. Before
/// 3.16.91 the file had no reader at all, so the button did nothing.
#[test]
fn a_full_check_request_forces_a_walk() {
    let dir = std::env::temp_dir().join(format!("pd-fullcheck-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let conf = dir.join("conf");
    let local = dir.join("local");
    let state = dir.join("state");
    let fake = dir.join("fake-proton-drive");
    let log = dir.join("cli.log");
    fs::create_dir_all(&conf).unwrap();
    fs::create_dir_all(local.join("Docs")).unwrap();
    fs::write(
        conf.join("proton-sync.conf"),
        format!(
            "PROTON_SYNC_LOCAL={}\nPROTON_SYNC_REMOTE=/my-files\nPROTON_SYNC_WORKERS=2\nPROTON_SYNC_RETRIES=1\n",
            local.display()
        ),
    )
    .unwrap();
    // Every listing is recorded, and the remote keeps one folder so a walk is
    // distinguishable from a cache hit (the cache needs a non-empty tree).
    fs::write(
        &fake,
        concat!(
            "#!/bin/sh\n",
            "echo \"$@\" >> \"$PROTON_CLI_LOG\"\n",
            "case \"$1\" in\n",
            "  fs) case \"$2\" in\n",
            "        size) echo '{\"size\":0,\"numberOfDescendants\":1}' ;;\n",
            "        list) case \"$3\" in\n",
            "                /my-files) echo '[{\"name\":{\"ok\":true,\"value\":\"Docs\"},\"type\":\"folder\",\"uid\":\"uid-docs\",\"modificationTime\":\"2026-10-26T00:00:00.000Z\"}]' ;;\n",
            "                *) echo '[]' ;;\n",
            "              esac ;;\n",
            "        *) echo '[]' ;;\n",
            "      esac ;;\n",
            "  *) echo '[]' ;;\n",
            "esac\n",
        ),
    )
    .unwrap();
    let mut perms = fs::metadata(&fake).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake, perms).unwrap();

    // One cycle, returning how many `fs list` calls it made.
    let cycle = || {
        let out = Command::new(env!("CARGO_BIN_EXE_proton-engine"))
            .args(["--once"])
            .env("XDG_CONFIG_HOME", &conf)
            .env("XDG_STATE_HOME", &state)
            .env("PROTON_DRIVE_BIN", &fake)
            .env("PROTON_CLI_LOG", &log)
            .env("PROTON_GUI_VERSION", "test")
            .output()
            .expect("engine runs");
        assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        let calls = fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("fs list"))
            .count();
        let _ = fs::remove_file(&log);
        calls
    };

    assert_eq!(cycle(), 2, "the first cycle lists the root and its folder");

    assert_eq!(cycle(), 0, "a fresh snapshot answers the next cycle");

    let request = state.join("proton-sync/audit-request");
    fs::write(&request, b"").unwrap();
    assert_eq!(
        cycle(),
        2,
        "\"Run full check now\" must list every folder"
    );
    assert!(!request.exists(), "the engine consumes the request it acted on");

    let _ = fs::remove_dir_all(&dir);
}
