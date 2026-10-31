//! CLI wrapper for the official Proton Drive binary.
//!
//! The app-wide lock serializes every call into the upstream CLI so the GUI,
//! the Python engine and this engine never rewrite the keyring session at the
//! same time. Mirrors `cli_lock` in Python and `clilock` in the GUI.

use anyhow::{anyhow, bail, Context, Result};
use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use fs2::FileExt;

fn state_dir() -> PathBuf {
    if let Some(xdg) = env::var_os("XDG_STATE_HOME") {
        PathBuf::from(xdg).join("proton-sync")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(".local/state/proton-sync")
    }
}

fn cli_lock_file() -> PathBuf {
    static LOCK: OnceLock<PathBuf> = OnceLock::new();
    LOCK.get_or_init(|| state_dir().join("cli.lock"))
        .clone()
}

static CLI_MTX: Mutex<()> = Mutex::new(());
thread_local! {
    static CLI_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}
static CLI_FH: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Acquire the app-wide CLI lock, re-entrant per thread.
pub fn with_cli_lock<F, R>(f: F) -> Result<R>
where
    F: FnOnce() -> Result<R>,
{
    let depth = CLI_DEPTH.get();
    if depth == 0 {
        let _guard = CLI_MTX.lock().unwrap();
        std::fs::create_dir_all(state_dir()).ok();
        if let Ok(fh) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(cli_lock_file())
        {
            let _ = fh.lock_exclusive();
            *CLI_FH.lock().unwrap() = Some(fh);
        }
        let result = f();
        CLI_DEPTH.set(0);
        if let Some(fh) = CLI_FH.lock().unwrap().take() {
            let _ = FileExt::unlock(&fh);
        }
        result
    } else {
        CLI_DEPTH.set(depth + 1);
        let result = f();
        CLI_DEPTH.set(depth - 1);
        result
    }
}

/// Resolve the official CLI binary.
pub fn bin() -> Option<PathBuf> {
    if let Some(p) = env::var_os("PROTON_DRIVE_BIN") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join("proton-drive");
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}

#[derive(Debug, Clone)]
pub struct Cli {
    bin: PathBuf,
}

impl Cli {
    pub fn new() -> Result<Self> {
        let bin = bin().ok_or_else(|| anyhow!("proton-drive not found (set PROTON_DRIVE_BIN or put it on PATH)"))?;
        Ok(Self { bin })
    }

    pub fn run(&self, args: &[&str], json: bool, _timeout: Option<u64>, locked: bool) -> Result<String> {
        let mut cmd_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        if json {
            cmd_args.push("--json".to_string());
        }
        let bin = self.bin.clone();
        let do_run = move || -> Result<std::process::Output> {
            let mut cmd = Command::new(&bin);
            cmd.args(&cmd_args);
            for key in [
                "PATH",
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_STATE_HOME",
                "XDG_DATA_HOME",
                "LANG",
                "LC_ALL",
                "DBUS_SESSION_BUS_ADDRESS",
                "DISPLAY",
                "WAYLAND_DISPLAY",
            ] {
                if let Some(v) = env::var_os(key) {
                    cmd.env(key, v);
                }
            }
            let out = cmd.output().with_context(|| "running proton-drive")?;
            Ok(out)
        };
        let out = if locked { with_cli_lock(do_run) } else { do_run() }?;
        if !out.status.success() {
            let raw_err = String::from_utf8_lossy(&out.stderr);
            let raw_out = String::from_utf8_lossy(&out.stdout);
            let raw = if raw_err.trim().is_empty() { raw_out.to_string() } else { raw_err.to_string() };
            let lines_vec: Vec<String> = raw
                .split('\n')
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.chars().all(|c| c == '='))
                .collect();
            let joined = lines_vec.join(" / ");
            let msg: String = joined.chars().take(500).collect();
            bail!(
                "{}: {}",
                args.iter().take(4).cloned().collect::<Vec<_>>().join(" "),
                if msg.trim().is_empty() { format!("exit {}", out.status) } else { msg.trim().to_string() }
            );
        }
        if json {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() {
                return Ok("[]".to_string());
            }
            serde_json::from_str::<serde_json::Value>(&s)
                .with_context(|| format!("bad JSON from CLI: {}", &s[..s.len().min(200)]))?;
            return Ok(s);
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    pub fn run_json(&self, args: &[&str], locked: bool) -> Result<serde_json::Value> {
        let s = self.run(args, true, None, locked)?;
        Ok(serde_json::from_str(&s)?)
    }
}

/// Escape a '/' inside a node name.
pub fn escape(component: &str) -> String {
    component.replace('/', r"\/")
}

/// Remote path for a relative one.
pub fn remote_path(remote_root: &str, rel: &str) -> String {
    if rel.is_empty() {
        return remote_root.to_string();
    }
    let mut parts: Vec<String> = vec![remote_root.to_string()];
    for c in rel.split('/') {
        parts.push(escape(c));
    }
    parts.join("/")
}

/// Boolean setting from the environment or `proton-sync.conf`.
pub fn cfg_flag(key: &str, default: bool) -> bool {
    let raw = env::var(key).ok().or_else(|| {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let dir = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        proton_core::conf::read(&dir.join("proton-sync.conf")).get(key).cloned()
    });
    parse_flag(raw.as_deref(), default)
}

fn parse_flag(raw: Option<&str>, default: bool) -> bool {
    match raw.map(str::trim) {
        Some("0" | "false" | "no" | "off") => false,
        Some("1" | "true" | "yes" | "on") => true,
        _ => default,
    }
}

/// Integer setting from the environment or `proton-sync.conf`, clamped to `1..=max`.
pub fn cfg_int(key: &str, default: usize, max: usize) -> usize {
    let raw = env::var(key).ok().or_else(|| {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let dir = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        proton_core::conf::read(&dir.join("proton-sync.conf")).get(key).cloned()
    });
    raw.and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
        .clamp(1, max)
}

/// Run `f` over `items` on up to `workers` threads, pulling from one shared queue.
/// Results come back in input order.
pub fn run_parallel<T, R, F>(items: Vec<T>, workers: usize, f: F) -> Vec<R>
where
    T: Send,
    R: Send,
    F: Fn(T) -> R + Sync,
{
    let n = items.len();
    let queue = Mutex::new(items.into_iter().enumerate().collect::<std::collections::VecDeque<_>>());
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..n).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..workers.max(1).min(n.max(1)) {
            s.spawn(|| loop {
                let next = queue.lock().unwrap().pop_front();
                let Some((i, item)) = next else { break };
                let r = f(item);
                results.lock().unwrap()[i] = Some(r);
            });
        }
    });
    results.into_inner().unwrap().into_iter().map(|r| r.expect("every item runs")).collect()
}

#[cfg(test)]
mod flag_tests {
    use super::parse_flag;

    #[test]
    fn flags_accept_the_usual_spellings() {
        assert!(!parse_flag(Some("0"), true));
        assert!(!parse_flag(Some("false"), true));
        assert!(!parse_flag(Some("off"), true));
        assert!(parse_flag(Some("1"), false));
        assert!(parse_flag(Some("true"), false));
        assert!(parse_flag(Some("on"), false));
        assert!(parse_flag(None, true));
        assert!(!parse_flag(None, false));
    }
}
