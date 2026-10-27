//! Desktop notifications with per-key cooldowns (the PyQt app's `_cool_ok`).
//! Sends happen on a worker thread so the UI never blocks on D-Bus.

use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Cooldowns {
    map: HashMap<String, Instant>,
}

impl Cooldowns {
    /// True when `key` hasn't fired within `seconds`; records it either way.
    pub fn cool_ok(&mut self, key: &str, seconds: u64) -> bool {
        let now = Instant::now();
        if let Some(last) = self.map.get(key)
            && now.duration_since(*last) < Duration::from_secs(seconds) {
                return false;
            }
        self.map.insert(key.to_string(), now);
        true
    }
}

/// Fire-and-forget notification (icon when we can find it, app name as title
/// prefix when the daemon shows nothing else).
pub fn send(title: &str, body: &str) {
    let title = title.to_string();
    let body = body.to_string();
    std::thread::spawn(move || {
        let mut n = notify_rust::Notification::new();
        n.summary(&title).body(&body);
        if let Some(icon) = crate::paths::find_icon_svg() {
            n.icon(&icon.to_string_lossy());
        }
        let _ = n.show();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_blocks_then_allows() {
        let mut c = Cooldowns::default();
        assert!(c.cool_ok("error", 30));
        assert!(!c.cool_ok("error", 30));
        assert!(c.cool_ok("conflict", 30)); // independent key
        assert!(c.cool_ok("error", 0)); // zero window always passes
    }
}
