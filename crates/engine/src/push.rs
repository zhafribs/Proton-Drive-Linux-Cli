//! Push listener skeleton (Phase 4).
//! Proton's event stream requires Secret Service session; implemented as best-effort.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone)]
pub struct PushListener {
    pending: Arc<Mutex<BTreeSet<String>>>,
}

impl PushListener {
    pub fn start(wake_event: Arc<std::sync::Condvar>, wake_mutex: Arc<Mutex<()>>) -> Self {
        let pending = Arc::new(Mutex::new(BTreeSet::new()));
        let p = pending.clone();
        thread::spawn(move || {
            loop {
                // best-effort polling; full implementation would use Proton API
                thread::sleep(Duration::from_secs(5));
                // noop for now
                let _ = (&p, &wake_event, &wake_mutex);
            }
        });
        Self { pending }
    }

    pub fn drain(&self) -> BTreeSet<String> {
        let mut g = self.pending.lock().unwrap();
        let out = g.clone();
        g.clear();
        out
    }
}
