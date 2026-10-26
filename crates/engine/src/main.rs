//! Experimental Rust engine CLI.
//!
//! Nothing here runs on a schedule yet. The commands are deliberately
//! read-only: `plan` diffs a fixture (used by the Python parity harness) and
//! `scan-local` prints a local tree for debugging.

use anyhow::{bail, Context, Result};
use proton_engine::filter::Filter;
use proton_engine::plan::{plan_cycle, PlanFixture};
use proton_engine::scan::scan_local;
use proton_engine::state_update::{refresh_state, RefreshFixture, RefreshOptions};
use std::path::Path;

fn main() {
    if let Err(err) = run() {
        eprintln!("proton-engine: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("plan") => plan_cmd(&args),
        Some("scan-local") => scan_local_cmd(&args),
        Some("refresh") => refresh_cmd(&args),
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
         \x20 proton-engine scan-local --root <dir> [--subfolders LIST] [--exclude LIST]\n"
    );
}
