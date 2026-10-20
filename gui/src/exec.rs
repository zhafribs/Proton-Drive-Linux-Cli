//! Subprocess execution with a hard deadline.
//!
//! Every external command the app runs (systemctl, journalctl, the official
//! CLI) goes through here: stdout/stderr are drained on their own threads so
//! a chatty child can never fill a pipe and deadlock, and a child that
//! overruns its deadline is SIGKILLed and reported as an error instead of
//! hanging the caller forever.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: i32, // -1 when killed by timeout
    pub timed_out: bool,
}

impl Output {
    pub fn ok(&self) -> bool {
        !self.timed_out && self.code == 0
    }
}

/// Run `cmd` with `args`, wait up to `timeout`, kill on overrun.
pub fn run(cmd: &str, args: &[&str], timeout: Duration) -> Result<Output, String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{cmd}: {e}"))?;

    let mut out_pipe = child.stdout.take().ok_or("no stdout pipe")?;
    let mut err_pipe = child.stderr.take().ok_or("no stderr pipe")?;
    let (tx_out, rx_out) = mpsc::channel();
    let (tx_err, rx_err) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = out_pipe.read_to_string(&mut buf);
        let _ = tx_out.send(buf);
    });
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = err_pipe.read_to_string(&mut buf);
        let _ = tx_err.send(buf);
    });

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().map_err(|e| format!("wait: {e}"))?;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(e) => return Err(format!("wait: {e}")),
        }
    };

    // Pipes close on child exit (or kill); collect with a small grace period.
    let grab = |rx: mpsc::Receiver<String>| -> String {
        let wait = Duration::from_secs(2);
        rx.recv_timeout(wait).unwrap_or_default()
    };
    Ok(Output {
        stdout: grab(rx_out),
        stderr: grab(rx_err),
        code: status.code().unwrap_or(-1),
        timed_out,
    })
}

/// Convenience: succeed/fail only.
pub fn run_ok(cmd: &str, args: &[&str], timeout: Duration) -> bool {
    run(cmd, args, timeout).map(|o| o.ok()).unwrap_or(false)
}
