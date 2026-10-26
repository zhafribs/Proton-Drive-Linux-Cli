//! Watch the GUI's state directory with inotify.
//!
//! `health.json` (the engine's telemetry) and `show-request` (a second launch
//! asking the running window to come forward) are written by other processes.
//! Watching the directory lets the window react when the write lands instead
//! of discovering it on the next timer tick.
//!
//! This is only a latency improvement: the poller keeps a short heartbeat,
//! because the state directory normally lives on plain local storage and
//! inotify is reliable there, but a fallback must still work if the watch
//! cannot be installed.

use inotify::{EventMask, Inotify, WatchMask};
use std::ffi::OsStr;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::thread;

/// The two files the poller cares about. Everything else in the directory
/// (the lock, the update marker, the snapshot) is not polled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Health,
    Show,
}

fn classify(name: Option<&OsStr>) -> Option<Kind> {
    match name?.to_str()? {
        "health.json" => Some(Kind::Health),
        "show-request" => Some(Kind::Show),
        _ => None,
    }
}

/// Start watching `state_dir`. Returns whether the watch is live; when it is
/// not, the caller keeps its short poll so nothing is ever missed.
///
/// One `()` is sent per batch of relevant events, not per inotify event:
/// `health.json` is written through a temporary file and a rename, which the
/// kernel reports several times.
pub fn spawn(state_dir: &Path, tx: Sender<()>) -> bool {
    let mut inotify = match Inotify::init() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("inotify unavailable ({e}); polling the state directory");
            return false;
        }
    };
    let mask = WatchMask::CREATE
        | WatchMask::CLOSE_WRITE
        | WatchMask::MOVED_TO
        | WatchMask::DELETE
        | WatchMask::DELETE_SELF
        | WatchMask::MOVE_SELF;
    if let Err(e) = inotify.add_watch(state_dir, mask) {
        eprintln!(
            "inotify cannot watch {} ({e}); polling it instead",
            state_dir.display()
        );
        return false;
    }
    let spawned = thread::Builder::new()
        .name("state-watch".into())
        .spawn(move || run(inotify, tx))
        .is_ok();
    if !spawned {
        eprintln!("state watcher thread failed; polling the state directory");
    }
    spawned
}

fn run(mut inotify: Inotify, tx: Sender<()>) {
    let mut buf = [0u8; 4096];
    loop {
        let events = match inotify.read_events_blocking(&mut buf) {
            Ok(events) => events,
            Err(e) => {
                eprintln!("state watcher stopped ({e}); falling back to polling");
                return;
            }
        };
        let mut wake = false;
        for event in events {
            if event.mask.contains(EventMask::Q_OVERFLOW) {
                // Events were dropped: wake once and let the poller re-read
                // everything, rather than pretend nothing happened.
                wake = true;
            } else if event.mask.contains(EventMask::IGNORED)
                || event.mask.contains(EventMask::DELETE_SELF)
            {
                eprintln!("state directory watch ended; falling back to polling");
                return;
            } else if classify(event.name).is_some() {
                wake = true;
            }
        }
        if wake && tx.send(()).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_polled_files_wake_the_ui() {
        assert_eq!(
            classify(Some(OsStr::new("health.json"))),
            Some(Kind::Health)
        );
        assert_eq!(
            classify(Some(OsStr::new("show-request"))),
            Some(Kind::Show)
        );
        // The engine writes through this temporary file first; waking on it
        // would double every update.
        assert_eq!(classify(Some(OsStr::new("health.json.tmp"))), None);
        assert_eq!(classify(Some(OsStr::new("lock"))), None);
        assert_eq!(classify(None), None);
    }

    /// End to end: the engine's write path (temp file + rename) and a second
    /// launch's create both reach the channel.
    #[test]
    fn watches_the_real_state_files() {
        let dir = std::env::temp_dir().join(format!(
            "proton-gui-watch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(spawn(&dir, tx));

        std::fs::write(dir.join("health.json.tmp"), b"{}").unwrap();
        std::fs::rename(dir.join("health.json.tmp"), dir.join("health.json")).unwrap();
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(2)).is_ok(),
            "renaming health.json into place should wake the poller"
        );

        std::fs::write(dir.join("show-request"), b"1").unwrap();
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(2)).is_ok(),
            "creating show-request should wake the poller"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
