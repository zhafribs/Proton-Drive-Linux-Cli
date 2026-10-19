//! `~/.config/proton-sync.conf` - the engine's settings file.
//!
//! Byte-compatible with the PyQt app's reader/writer: defaults for the nine
//! known keys, `KEY=VALUE` lines, comments ignored on read, extras (proxy
//! etc.) round-tripped after the known keys on write.

use std::collections::BTreeMap;
use std::path::Path;

/// The nine keys the app owns, in the fixed order they are written.
pub const CONF_KEYS: [&str; 9] = [
    "PROTON_SYNC_LOCAL",
    "PROTON_SYNC_REMOTE",
    "PROTON_SYNC_INTERVAL",
    "PROTON_SYNC_DELETES",
    "PROTON_SYNC_INOTIFY",
    "PROTON_SYNC_WORKERS",
    "PROTON_SYNC_RETRIES",
    "PROTON_SYNC_SUBFOLDERS",
    "PROTON_SYNC_EXCLUDE",
];

/// Friendly "check every ..." choices -> seconds (same order as the combo).
pub const INTERVAL_CHOICES: [(&str, u64); 6] = [
    ("Instant (30 seconds)", 30),
    ("Every minute", 60),
    ("Every 5 minutes", 300),
    ("Every 15 minutes", 900),
    ("Every 30 minutes", 1800),
    ("Every hour", 3600),
];

pub fn defaults() -> BTreeMap<&'static str, String> {
    let home = dirs::home_dir().unwrap_or_else(|| "/".into());
    let mut m = BTreeMap::new();
    m.insert("PROTON_SYNC_LOCAL", home.join("ProtonDrive").to_string_lossy().into_owned());
    m.insert("PROTON_SYNC_REMOTE", "/my-files".into());
    m.insert("PROTON_SYNC_INTERVAL", "300".into());
    m.insert("PROTON_SYNC_DELETES", "1".into());
    m.insert("PROTON_SYNC_INOTIFY", "1".into());
    m.insert("PROTON_SYNC_WORKERS", "12".into());
    m.insert("PROTON_SYNC_RETRIES", "3".into());
    m.insert("PROTON_SYNC_SUBFOLDERS", String::new());
    m.insert("PROTON_SYNC_EXCLUDE", String::new());
    m
}

/// Read the conf: defaults overlaid with every `KEY=VALUE` line found.
pub fn read(path: &Path) -> BTreeMap<String, String> {
    let mut values: BTreeMap<String, String> = defaults()
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                if let Some((k, v)) = line.split_once('=') {
                    values.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
    }
    values
}

/// Non-default KEY=VALUE lines, in file order (the "extra environment
/// variables" editor round-trips these).
pub fn read_extras(path: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines() {
            let s = line.trim();
            if s.is_empty() || s.starts_with('#') {
                continue;
            }
            if let Some((k, _)) = s.split_once('=') {
                if !CONF_KEYS.contains(&k.trim()) {
                    out.push(s.to_string());
                }
            }
        }
    }
    out
}

/// Rewrite the whole file: header, the nine known keys, then extras.
pub fn write(path: &Path, values: &BTreeMap<String, String>, extras: &[String]) {
    let mut out = String::from(
        "# Proton Drive sync settings\n# (edited by the app - changes apply after a restart)\n",
    );
    let d = defaults();
    for k in CONF_KEYS {
        let v = values
            .get(k)
            .cloned()
            .unwrap_or_else(|| d.get(k).cloned().unwrap_or_default());
        out.push_str(&format!("{k}={v}\n"));
    }
    for e in extras {
        out.push_str(e.trim_end());
        out.push('\n');
    }
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
    let _ = std::fs::write(path, out);
}

/// Which combo entry an interval maps to (nearest choice, like the PyQt app).
pub fn interval_index(secs: u64) -> usize {
    INTERVAL_CHOICES
        .iter()
        .enumerate()
        .min_by_key(|(_, (_, v))| v.abs_diff(secs))
        .map(|(i, _)| i)
        .unwrap_or(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pdconf-{}-{name}", std::process::id()))
    }

    #[test]
    fn roundtrip_preserves_extras_and_defaults() {
        let p = tmp("roundtrip");
        let mut v: BTreeMap<String, String> = defaults()
            .into_iter()
            .map(|(k, val)| (k.to_string(), val))
            .collect();
        v.insert("PROTON_SYNC_INTERVAL".into(), "60".into());
        write(&p, &v, &["HTTPS_PROXY=http://proxy:3128".into()]);
        let back = read(&p);
        assert_eq!(back["PROTON_SYNC_INTERVAL"], "60");
        assert_eq!(back["PROTON_SYNC_REMOTE"], "/my-files");
        assert_eq!(read_extras(&p), vec!["HTTPS_PROXY=http://proxy:3128".to_string()]);
        // exact header + nine keys, like the PyQt writer
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# Proton Drive sync settings\n# (edited by the app"));
        assert_eq!(text.lines().filter(|l| l.starts_with("PROTON_SYNC_")).count(), 9);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn comments_and_unknown_lines_are_not_conf_values() {
        let p = tmp("comments");
        std::fs::write(&p, "# comment\nPROTON_SYNC_DELETES=0\n\nWEIRD\n").unwrap();
        let v = read(&p);
        assert_eq!(v["PROTON_SYNC_DELETES"], "0");
        assert_eq!(read_extras(&p), Vec::<String>::new()); // "WEIRD" has no '='
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn interval_index_picks_nearest() {
        assert_eq!(interval_index(30), 0);
        assert_eq!(interval_index(70), 1); // nearer 60 than 30
        assert_eq!(interval_index(600), 2);
        assert_eq!(interval_index(100_000), 5); // clamps to the last choice
    }
}
