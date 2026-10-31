//! Fast remote change watcher.
//!
//! The official CLI has no event/watch command, so a cheap `fs size`
//! fingerprint (total bytes + descendants) is polled every few seconds. When
//! it changes, the sync loop is woken immediately. The configured check
//! interval remains the fallback for fingerprint-invisible changes (for
//! example, an edit that keeps the same total size).
//!
//! Disable with `PROTON_SYNC_PUSH=0`; tune with `PROTON_SYNC_PUSH_INTERVAL`
//! (seconds, 1-30).

use crate::cli::{self, Cli};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub fn start(
    cli: Cli,
    remote_root: String,
    wake: Arc<AtomicBool>,
    wake_event: Arc<Condvar>,
    wake_mutex: Arc<Mutex<()>>,
) {
    if !cli::cfg_flag("PROTON_SYNC_PUSH", true) {
        return;
    }
    let seconds = cli::cfg_int("PROTON_SYNC_PUSH_INTERVAL", 3, 30).max(1);
    std::thread::spawn(move || {
        let mut last: Option<Vec<i64>> = None;
        loop {
            std::thread::sleep(Duration::from_secs(seconds as u64));
            // Serialize with the GUI and the sync cycle: concurrent CLI calls
            // can lose the keyring session. A cycle just delays the next poll.
            let Ok(value) = cli.run_json(&["fs", "size", &remote_root], true) else {
                continue;
            };
            let Some(fingerprint) = fingerprint(&value) else {
                continue;
            };
            if last.as_ref().is_some_and(|prev| *prev != fingerprint) {
                wake.store(true, Ordering::Relaxed);
                let _guard = wake_mutex.lock().unwrap();
                wake_event.notify_all();
            }
            last = Some(fingerprint);
        }
    });
}

/// `fs size` fingerprint: total bytes and descendant count.
fn fingerprint(value: &serde_json::Value) -> Option<Vec<i64>> {
    Some(vec![
        value.get("size")?.as_i64()?,
        value.get("numberOfDescendants")?.as_i64()?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_reads_size_and_count() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"size":1100000000,"numberOfDescendants":1254}"#).unwrap();
        assert_eq!(fingerprint(&v), Some(vec![1_100_000_000, 1254]));
        assert_eq!(
            fingerprint(&serde_json::from_str(r#"{"size":1}"#).unwrap()),
            None
        );
    }
}
