//! `state.json` - the last synced fingerprint per relative path.
//!
//! Shape written by the Python engine:
//! `{"rel": {"l_mtime_ns": 1, "l_size": 2, "r_mtime": "...", "r_uid": "..."}}`.
//! Missing keys and explicit nulls both mean "this side is unknown"; the
//! reader treats them the same, and the writer omits unknown sides.

use serde::Serialize;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

pub type State = BTreeMap<String, StateEntry>;

#[derive(Serialize, serde::Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub struct StateEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l_mtime_ns: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r_mtime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r_uid: Option<String>,
}

/// Parse a state file, returning an empty map for a missing or malformed one.
pub fn read(path: &Path) -> State {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Atomically replace the state file, indented one space and key-sorted like
/// `json.dump(..., indent=1, sort_keys=True)`.
pub fn write(path: &Path, state: &State) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name("state.json.tmp");
    let file = std::fs::File::create(&tmp)?;
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(file, formatter);
    state.serialize(&mut ser).map_err(io::Error::other)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_state_shape() {
        let json = r#"{
            "Personel": {
                "r_mtime": "2026-09-12T05:04:56.000Z",
                "r_uid": "uid-dir"
            },
            "Personel/Renewal.docx": {
                "l_mtime_ns": 1789961340000000000,
                "l_size": 1069040,
                "r_mtime": "2026-09-21T03:29:00.000Z",
                "r_uid": "uid-file"
            },
            "OnlyLocal.txt": {
                "l_mtime_ns": 5,
                "l_size": 10,
                "r_mtime": null,
                "r_uid": null
            }
        }"#;
        let state: State = serde_json::from_str(json).unwrap();
        assert_eq!(state.len(), 3);
        assert_eq!(state["Personel"].l_mtime_ns, None);
        assert_eq!(state["Personel"].l_size, None);
        assert_eq!(state["Personel"].r_uid.as_deref(), Some("uid-dir"));
        let file = &state["Personel/Renewal.docx"];
        assert_eq!(file.l_size, Some(1069040));
        assert_eq!(file.r_mtime.as_deref(), Some("2026-09-21T03:29:00.000Z"));
        assert_eq!(state["OnlyLocal.txt"].r_mtime, None);
    }

    #[test]
    fn roundtrip_is_sorted_and_atomic() {
        let path = std::env::temp_dir().join(format!("pdstate-{}.json", std::process::id()));
        let mut state = State::new();
        state.insert(
            "b/file.txt".into(),
            StateEntry {
                l_mtime_ns: Some(1),
                l_size: Some(2),
                r_mtime: Some("2026-01-01T00:00:00.000Z".into()),
                r_uid: Some("uid".into()),
            },
        );
        state.insert(
            "a/dir".into(),
            StateEntry {
                r_uid: Some("dir-uid".into()),
                ..Default::default()
            },
        );
        write(&path, &state).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("{\n \"a/dir\": {"));
        assert!(!text.contains("null"));
        assert_eq!(read(&path), state);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(path.with_file_name("state.json.tmp")).ok();
    }

    #[test]
    fn malformed_file_is_empty() {
        let path = std::env::temp_dir().join(format!("pdstate-bad-{}.json", std::process::id()));
        std::fs::write(&path, "{not json").unwrap();
        assert!(read(&path).is_empty());
        std::fs::remove_file(&path).ok();
    }
    #[test]
    fn garbage_state_falls_back_to_empty() {
        let p = std::env::temp_dir().join(format!("pdstate-garbage-{}.json", std::process::id()));
        std::fs::write(&p, "{not json at all").unwrap();
        let st = read(&p);
        assert!(st.is_empty());
        std::fs::remove_file(&p).ok();
    }

    /// Fuzz-lite: arbitrary bytes must never panic the state reader.
    #[test]
    fn fuzz_state_read_never_panics() {
        let p = std::env::temp_dir().join(format!("pdstate-fuzz-{}.json", std::process::id()));
        let mut x: u64 = 0x9E3779B97F4A7C15;
        for _ in 0..300 {
            x ^= x << 13; x ^= x >> 7; x ^= x << 17;
            let len = (x % 200) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; (x % 256) as u8 }).collect();
            std::fs::write(&p, &bytes).unwrap();
            let _ = read(&p);
        }
        std::fs::remove_file(&p).ok();
    }

}
