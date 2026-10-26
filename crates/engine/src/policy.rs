//! Adaptive worker/transfer limits. Pure policy, identical inputs to the
//! Python helpers so the Python policy tests carry over directly.

pub const CLI_PROCESS_MB: u64 = 100;
pub const MEM_RESERVE_MB: u64 = 256;

/// Cap scan workers by the configured limit and available RAM.
pub fn adaptive_worker_limit(configured: u64, available_mb: Option<u64>) -> u64 {
    let limit = configured.max(1);
    let Some(available) = available_mb else {
        return limit;
    };
    let reserve = MEM_RESERVE_MB.max(available / 5);
    let budget = available.saturating_sub(reserve);
    (budget / CLI_PROCESS_MB).clamp(1, limit)
}

/// Back off on throttling, then creep back up after three clean chunks. The
/// ceiling is the user's configured worker limit.
pub fn adjust_worker_limit(current: u64, saw_rate_limit: bool, clean_chunks: u64, ceiling: u64) -> u64 {
    if saw_rate_limit {
        return current.saturating_sub(2).max(1);
    }
    if clean_chunks >= 3 {
        return (current + 1).min(ceiling.max(1)).max(1);
    }
    current
}

pub fn is_rate_limit(message: &str) -> bool {
    let m = message.to_lowercase();
    ["429", "rate limit", "rate-limit", "too many requests", "quota exceeded", "throttl"]
        .iter()
        .any(|t| m.contains(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_budget_caps_workers() {
        assert_eq!(adaptive_worker_limit(12, Some(1024)), 7);
        assert_eq!(adaptive_worker_limit(12, None), 12);
        assert_eq!(adaptive_worker_limit(12, Some(100)), 1);
        assert_eq!(adaptive_worker_limit(0, Some(1024)), 1);
    }

    #[test]
    fn rate_limit_lowers_and_clean_chunks_raise() {
        assert_eq!(adjust_worker_limit(8, true, 0, 16), 6);
        assert_eq!(adjust_worker_limit(6, false, 2, 16), 6);
        assert_eq!(adjust_worker_limit(6, false, 3, 16), 7);
        assert_eq!(adjust_worker_limit(16, false, 3, 16), 16);
    }

    #[test]
    fn clean_chunks_never_exceed_the_ceiling() {
        assert_eq!(adjust_worker_limit(4, false, 3, 4), 4);
        assert_eq!(adjust_worker_limit(3, false, 3, 4), 4);
        assert_eq!(adjust_worker_limit(9, false, 3, 10), 10);
    }

    #[test]
    fn detects_rate_limit_messages() {
        assert!(is_rate_limit("429 too many requests"));
        assert!(is_rate_limit("Quota exceeded"));
        assert!(is_rate_limit("server throttled us"));
        assert!(!is_rate_limit("file not found"));
    }
}
