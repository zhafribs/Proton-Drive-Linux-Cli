//! Experimental Rust engine CLI.
//!
//! Nothing here runs on a schedule yet: `plan`/`refresh` take fixtures for
//! parity, `scan-local` prints a local tree, and `--dry-run`/`dry-run` scan
//! real local/remote trees and print the action plan without mutating anything.

use anyhow::{bail, Context, Result};
use proton_engine::cli::Cli;
use proton_engine::filter::Filter;
use proton_engine::loop_::run_loop;
use proton_engine::plan::{plan_cycle, PlanConfig, PlanFixture};
use proton_engine::scan::{scan_local, scan_remote};
use proton_engine::state_update::{refresh_state, RefreshFixture, RefreshOptions};
use proton_core::{conf, state};
use std::path::Path;

fn main() {
    if let Err(err) = run() {
        eprintln!("proton-engine: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("--dry-run") {
        return dry_run_cmd(&args[1..]);
    }
    match args.first().map(String::as_str) {
        Some("plan") => plan_cmd(&args),
        Some("scan-local") => scan_local_cmd(&args),
        Some("refresh") => refresh_cmd(&args),
        Some("dry-run") => dry_run_cmd(&args[1..]),
        Some("--once") | Some("once") => {
            // run single cycle
            let _lock = {
                let sd = {
                    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME") {
                        std::path::PathBuf::from(xdg).join("proton-sync")
                    } else {
                        dirs::home_dir().unwrap_or_else(|| std::path::Path::new("/").to_path_buf()).join(".local/state/proton-sync")
                    }
                };
                std::fs::create_dir_all(&sd)?;
                let lock_path = sd.join("sync.lock");
                let fh = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&lock_path)?;
                fs2::FileExt::try_lock_exclusive(&fh).map_err(|_| anyhow::anyhow!("another proton-sync run is already active"))?;
                fh
            };
            let home = dirs::home_dir().unwrap_or_else(|| Path::new("/").to_path_buf());
            let config_dir = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
            let values = proton_core::conf::read(&config_dir.join("proton-sync.conf"));
            let local_root = values.get("PROTON_SYNC_LOCAL").cloned().unwrap_or_else(|| home.join("ProtonDrive").to_string_lossy().into_owned());
            let remote_root = values.get("PROTON_SYNC_REMOTE").cloned().unwrap_or_else(|| "/my-files".to_string());
            let subfolders = values.get("PROTON_SYNC_SUBFOLDERS").cloned().unwrap_or_default();
            let exclude = values.get("PROTON_SYNC_EXCLUDE").cloned().unwrap_or_default();
            let propagate_deletes = values.get("PROTON_SYNC_DELETES").map(|v| v != "0").unwrap_or(true);
            let cli = Cli::new()?;
            let filter = Filter::new(&subfolders, &exclude);
            proton_engine::loop_::run_once_cycle(&cli, &local_root, &remote_root, &filter, propagate_deletes)
        }
        Some("--loop") | Some("loop") => run_loop(),
        Some("help") | Some("--help") | Some("-h") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => bail!("unknown command {other:?}; try --help"),
    }
}

fn plan_cmd(args: &[String]) -> Result<()> {
    let path = arg_value(args, "--fixture").context("plan needs --fixture <file.json>")?;
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?;
    let fixture: PlanFixture =
        serde_json::from_str(&text).with_context(|| format!("parsing {path}"))?;
    let result = plan_cycle(
        &fixture.remote,
        &fixture.local,
        &fixture.state,
        &fixture.config(),
    );
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn scan_local_cmd(args: &[String]) -> Result<()> {
    let root = arg_value(args, "--root").context("scan-local needs --root <dir>")?;
    let subfolders = arg_value(args, "--subfolders").unwrap_or_default();
    let exclude = arg_value(args, "--exclude").unwrap_or_default();
    let filter = Filter::new(&subfolders, &exclude);
    let tree = scan_local(Path::new(&root), &filter);
    println!("{}", serde_json::to_string_pretty(&tree)?);
    Ok(())
}

fn refresh_cmd(args: &[String]) -> Result<()> {
    let path = arg_value(args, "--fixture").context("refresh needs --fixture <file.json>")?;
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?;
    let fixture: RefreshFixture =
        serde_json::from_str(&text).with_context(|| format!("parsing {path}"))?;
    let options = RefreshOptions {
        propagate_deletes: fixture.propagate_deletes,
        failed: &fixture.failed,
        acted: &fixture.acted,
        planned_local: fixture.planned_local.as_ref(),
        downloaded: Some(&fixture.downloaded),
        preserve: Some(&fixture.preserve),
    };
    let state = refresh_state(
        &fixture.remote,
        &fixture.local,
        &fixture.old_state,
        &options,
    );
    println!("{}", serde_json::to_string_pretty(&state)?);
    Ok(())
}

fn dry_run_cmd(_args: &[String]) -> Result<()> {
    let home = dirs::home_dir().unwrap_or_else(|| Path::new("/").to_path_buf());
    let config_dir = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"));
    let state_dir = state_home.join("proton-sync");

    let cfg_path = config_dir.join("proton-sync.conf");
    let values = conf::read(&cfg_path);
    let local_root = values
        .get("PROTON_SYNC_LOCAL")
        .cloned()
        .unwrap_or_else(|| home.join("ProtonDrive").to_string_lossy().into_owned());
    let remote_root = values
        .get("PROTON_SYNC_REMOTE")
        .cloned()
        .unwrap_or_else(|| "/my-files".to_string());
    let subfolders = values.get("PROTON_SYNC_SUBFOLDERS").cloned().unwrap_or_default();
    let exclude = values.get("PROTON_SYNC_EXCLUDE").cloned().unwrap_or_default();
    let propagate_deletes = values
        .get("PROTON_SYNC_DELETES")
        .map(|v| v != "0")
        .unwrap_or(true);

    let cli = Cli::new().context("locating official proton-drive CLI")?;
    let filter = Filter::new(&subfolders, &exclude);

    let local = scan_local(Path::new(&local_root), &filter);
    let remote = scan_remote(&cli, &remote_root, &filter).context("scanning remote tree")?;
    let st = state::read(&state_dir.join("state.json"));

    let config = PlanConfig { propagate_deletes, wipe_min: 5 };
    let result = plan_cycle(&remote, &local, &st, &config);
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    let index = args.iter().position(|a| a == name)?;
    args.get(index + 1).cloned()
}

fn print_usage() {
    eprintln!(
        "proton-engine (experimental)\n\
         \n\
         Usage:\n\
         \x20 proton-engine plan --fixture <file.json>\n\
         \x20 proton-engine refresh --fixture <file.json>\n\
         \x20 proton-engine scan-local --root <dir> [--subfolders LIST] [--exclude LIST]\n\
         \x20 proton-engine dry-run\n\
         \x20 proton-engine --dry-run\n\
         \x20 proton-engine --loop\n"
    );
}
