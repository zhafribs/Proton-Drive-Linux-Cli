//! One-shot approval for a reviewed mass deletion.
//!
//! The GUI writes this only after the user confirms the exact held path set;
//! the engine consumes it on the next cycle and never lets it bypass the
//! empty-side wipe guard. The digest makes a same-count but different-path
//! batch fail closed.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_AGE_SECS: u64 = 600;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DeleteApproval {
    pub local: u64,
    pub cloud: u64,
    pub local_digest: String,
    pub cloud_digest: String,
    pub created_at: u64,
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Stable digest for the exact set of paths the user reviewed. Length-prefix
/// every path so names containing separators or newlines cannot collide.
pub fn digest_paths(paths: &BTreeSet<String>) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for path in paths {
        for byte in (path.len() as u64)
            .to_le_bytes()
            .iter()
            .chain(path.as_bytes())
        {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

/// Write atomically. The caller must have already confirmed the path set.
pub fn write(path: &Path, approval: &DeleteApproval) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("deletes-approval.json");
    let tmp = path.with_file_name(format!("{name}.tmp"));
    std::fs::write(
        &tmp,
        serde_json::to_vec(approval).map_err(io::Error::other)?,
    )?;
    std::fs::rename(&tmp, path)
}

/// Read and remove the approval in one step. Missing, malformed, future-dated
/// and stale files all fail closed; the file is consumed even when rejected.
pub fn take(path: &Path, now: u64) -> Option<DeleteApproval> {
    let text = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::remove_file(path);
    let approval: DeleteApproval = serde_json::from_str(&text).ok()?;
    if approval.created_at > now.saturating_add(60)
        || now.saturating_sub(approval.created_at) > MAX_AGE_SECS
    {
        return None;
    }
    Some(approval)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pd-approval-{}-{name}", std::process::id()))
    }

    fn approval() -> DeleteApproval {
        DeleteApproval {
            local: 2,
            cloud: 1,
            local_digest: "aa".into(),
            cloud_digest: "bb".into(),
            created_at: 100,
        }
    }

    #[test]
    fn digest_is_path_exact_and_separator_safe() {
        let a = BTreeSet::from(["a/b".into(), "c".into()]);
        let b = BTreeSet::from(["a".into(), "b/c".into()]);
        assert_eq!(digest_paths(&a), digest_paths(&a.clone()));
        assert_ne!(digest_paths(&a), digest_paths(&b));
        assert_ne!(
            digest_paths(&BTreeSet::from(["a b".into()])),
            digest_paths(&BTreeSet::from(["a\nb".into()]))
        );
    }

    #[test]
    fn write_take_consumes_once() {
        let path = temp("once.json");
        write(&path, &approval()).unwrap();
        assert_eq!(take(&path, 100), Some(approval()));
        assert_eq!(take(&path, 100), None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn stale_future_and_malformed_fail_closed() {
        let stale = temp("stale.json");
        let mut a = approval();
        a.created_at = 1;
        write(&stale, &a).unwrap();
        assert_eq!(take(&stale, MAX_AGE_SECS + 2), None);
        assert_eq!(take(&stale, MAX_AGE_SECS + 2), None);

        let future = temp("future.json");
        let mut f = approval();
        f.created_at = 1000;
        write(&future, &f).unwrap();
        assert_eq!(take(&future, 100), None);

        let bad = temp("bad.json");
        std::fs::write(&bad, "{not json").unwrap();
        assert_eq!(take(&bad, 100), None);
        let _ = std::fs::remove_file(bad);
    }
}
