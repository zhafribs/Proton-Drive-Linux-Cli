//! One upstream-CLI process at a time, app-wide.
//!
//! The official Proton Drive CLI keeps its session in the OS keyring and
//! rewrites that item whenever it refreshes the access token. gnome-keyring's
//! `login` collection is an *alias* collection: an item is keyed by its
//! attributes, and registering a second one with the same attributes is
//! refused ("asked to register item .../login/N, but it's already
//! registered"). Two CLI processes rotating the session at the same time
//! therefore lose it outright - the new token is never persisted and the old
//! item is dropped - and every later call comes back 401 "You need to login
//! first", which strands the sync engine until the user signs in again.
//!
//! The GUI and the background sync engine both drive that CLI, so the lock
//! has to live in a file they share rather than in either process.
//!
//! The engine keeps its own scan parallel: it probes the session with a
//! single call (alone, under this lock) before letting its workers run, so
//! no worker ever hits a 401 and rewrites the item. This lock is what
//! keeps the GUI - and a sign-in - out of the way while that happens.

use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a GUI call waits for the engine to finish its CLI batch.
///
/// The engine holds the lock for a whole parallel scan, so this has to be
/// longer than a typical scan; past it the call runs unprotected rather
/// than leaving the user staring at an empty folder.
pub const WAIT: Duration = Duration::from_secs(60);

/// Held for as long as a CLI call is in flight. Released on drop.
pub struct CliLock {
    file: File,
}

impl CliLock {
    /// Wait up to `wait` for the lock.
    ///
    /// `None` means the lock was not taken (the engine is mid-scan, or the
    /// state dir is not writable). The caller may still run - refusing to
    /// show a folder because the engine is busy would be worse - but it is
    /// then unprotected, so a session loss stays possible in that case.
    pub fn acquire(path: &Path, wait: Duration) -> Option<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)
            .ok()?;
        let deadline = Instant::now() + wait;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Some(Self { file }),
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => {
                    eprintln!("proton-drive: CLI busy for {wait:?} - running unprotected");
                    return None;
                }
            }
        }
    }
}

impl Drop for CliLock {
    fn drop(&mut self) {
        // Fully qualified: std::fs::File grew an inherent `unlock` in Rust
        // 1.89, which would otherwise shadow the fs2 trait method.
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_lock() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("proton-cli-lock-{}.lock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn lock_is_exclusive_and_released_on_drop() {
        let path = tmp_lock();
        let first = CliLock::acquire(&path, Duration::from_secs(5)).expect("first lock");
        // A second handle on the same file must not get in.
        assert!(CliLock::acquire(&path, Duration::from_millis(200)).is_none());
        drop(first);
        // ... and the lock is free again afterwards.
        assert!(CliLock::acquire(&path, Duration::from_secs(5)).is_some());
        let _ = std::fs::remove_file(&path);
    }
}
