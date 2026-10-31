//! Status mapping for the engine's health snapshot - the struct itself lives
//! in `proton-core` so the GUI and the engine share one schema.

pub use proton_core::health::{Health, is_login_error, read};

use crate::util::rel_time;

/// The states the Home orb can show. Each has its own colour and glyph.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiStatus {
    Synced,
    Syncing,
    /// The user paused syncing (a resume button is available).
    Paused,
    /// The stored session is gone - only a fresh sign-in brings syncing back.
    NeedsLogin,
    /// Nobody is signed in (welcome screen).
    SignedOut,
    /// A sign-in is in progress (the external terminal is open).
    SigningIn,
    /// Signed in, but the sync service is not running (stopped/crashed).
    EngineStopped,
    /// The service is up but has not reported a cycle yet.
    Starting,
    Error,
    Unknown,
}

impl UiStatus {
    pub fn key(self) -> &'static str {
        match self {
            UiStatus::Synced => "synced",
            UiStatus::Syncing => "syncing",
            UiStatus::Paused => "paused",
            UiStatus::NeedsLogin => "needs_login",
            UiStatus::SignedOut => "signed_out",
            UiStatus::SigningIn => "signing_in",
            UiStatus::EngineStopped => "engine_stopped",
            UiStatus::Starting => "starting",
            UiStatus::Error => "error",
            UiStatus::Unknown => "unknown",
        }
    }
}

/// The full status derivation: engine liveness, the engine's own report, the
/// account probe and the app's transient login/pause state.
///
/// Engine activity wins over the account probe: if the service is already
/// checking/syncing, the Home must say so immediately, even while the identity
/// probe is still in flight.
pub fn derive_status(
    active: bool,
    hstatus: &str,
    identity: Option<bool>,
    signing_in: bool,
    paused_by_user: bool,
    has_last_email: bool,
    session_error: bool,
) -> UiStatus {
    // `None` = the account probe has not returned yet; `Some(false)` = probed
    // and signed out; `Some(true)` = signed in.
    let identity_known = identity.is_some();
    let logged_in = identity == Some(true);
    // The engine is already working: show that, not a generic "Starting".
    if active && hstatus == "syncing" {
        return UiStatus::Syncing;
    }
    // The sign-in terminal is open; this outranks a stale needs-login state.
    if signing_in && !logged_in {
        return UiStatus::SigningIn;
    }
    // A lost session is only "expired" if this account was signed in before.
    // A fresh install (no last email) gets the welcome screen instead.
    // Health wins over a stale identity probe: the probe can still say
    // "logged in" while the engine's CLI calls fail with a login error.
    if hstatus == "needs_login" || session_error {
        return if has_last_email {
            UiStatus::NeedsLogin
        } else {
            UiStatus::SignedOut
        };
    }
    if active && hstatus == "error" {
        return UiStatus::Error;
    }
    if identity_known && !logged_in {
        return UiStatus::SignedOut;
    }
    if !identity_known {
        // The account probe is still running. If the engine already finished a
        // cycle, show its result; otherwise the engine is starting.
        return if active && hstatus == "ok" {
            UiStatus::Synced
        } else {
            UiStatus::Starting
        };
    }
    if active {
        match hstatus {
            "ok" => UiStatus::Synced,
            "" => UiStatus::Starting,
            _ => UiStatus::Unknown,
        }
    } else if paused_by_user {
        UiStatus::Paused
    } else {
        UiStatus::EngineStopped
    }
}

/// (title, subtitle-template) per status - verbatim PyQt strings where they
/// exist, plain product wording for the states the Python app never had.
pub fn status_text(status: UiStatus) -> (&'static str, &'static str) {
    match status {
        UiStatus::Synced => ("You're up to date", ""),
        UiStatus::Syncing => ("Syncing your files...", "Just a moment..."),
        UiStatus::Paused => ("Sync is paused", "Nothing is syncing. Your files are safe."),
        UiStatus::NeedsLogin => ("Your session expired", "Sign in again to resume syncing."),
        UiStatus::SignedOut => (
            "Welcome to Proton Drive",
            "Sign in to start - your files will appear here.",
        ),
        UiStatus::SigningIn => (
            "Signing in...",
            "Finish signing in in the terminal window.",
        ),
        UiStatus::EngineStopped => (
            "Sync engine isn't running",
            "Start it to keep your files in sync.",
        ),
        UiStatus::Starting => ("Starting the sync engine...", "Just a moment..."),
        UiStatus::Error => ("Couldn't sync", "{error}"),
        UiStatus::Unknown => ("Checking your files...", ""),
    }
}

/// "Last checked just now · Next check in 42s" - any half available is fine.
pub fn synced_line(h: &Health) -> String {
    if h.audit_pending == Some(true) {
        return match h.audit_total.filter(|n| *n > 0) {
            Some(total) => format!(
                "Ready from cache \u{b7} checking {} / {} folders",
                h.audit_done.unwrap_or(0),
                total
            ),
            None => "Ready from cache \u{b7} background check running".to_string(),
        };
    }
    if let Some(age) = h.cache_age {
        return format!("Cached view \u{b7} {age}s old");
    }
    let last = rel_time(h.ts_dt());
    // interval 0 means no scheduled checks, so there is no countdown to show -
    // just the last check, exactly like a normal cycle with no next time.
    let nxt = if h.interval == Some(0) {
        None
    } else {
        // Prefer the engine's own next_poll timestamp; fall back to the
        // interval. A stale/past next_poll clamps to 0 rather than panicking.
        h.next_poll_dt()
            .map(|dt| {
                let secs = (dt - chrono::Utc::now().fixed_offset()).num_seconds();
                secs.max(0) as u64
            })
            .or(h.interval)
    };
    match (last.is_empty(), nxt) {
        (false, Some(n)) => format!("Last checked {last} · Next check in {n}s"),
        (false, None) => format!("Last checked {last}"),
        (true, Some(n)) => format!("Next check in {n}s"),
        (true, None) => "Waiting for the first check".to_string(),
    }
}

/// The subtitle under the hero title, per status.
pub fn hero_sub(status: UiStatus, h: &Health) -> String {
    let (_, sub) = status_text(status);
    match status {
        UiStatus::Synced => synced_line(h),
        UiStatus::Error => h
            .error
            .clone()
            .unwrap_or_else(|| "See Recent activity below for details.".into()),
        _ => sub.replace("{error}", h.error.as_deref().unwrap_or("")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim engine output shape - a single field type mismatch used to
    /// silently fail the whole parse and blank the Home orb.
    #[test]
    fn parses_live_health_json() {
        let json = r#"{
            "status": "ok",
            "started": "2026-10-20T00:29:25+08:00",
            "ts": "2026-10-20T00:30:02+08:00",
            "cycle_seconds": 36.5,
            "actions": 0,
            "holds": 270,
            "failed": 0,
            "uploaded": 0,
            "downloaded": 0,
            "conflicts": 0,
            "remote_entries": 270,
            "local_entries": 270,
            "state_records": 270,
            "interval": 60,
            "error": "",
            "next_poll": "2026-10-20T00:31:02+08:00",
            "mode": "full"
        }"#;
        let h: Health = serde_json::from_str(json).unwrap();
        assert_eq!(h.status, "ok");
        assert_eq!(h.holds, 270);
        assert_eq!(h.interval, Some(60));
        assert!(h.next_poll_dt().is_some());
        assert!(h.ts_dt().is_some());
        assert_eq!(
            derive_status(true, &h.status, Some(true), false, false, false, false),
            UiStatus::Synced
        );
    }

    /// Interval 0 is "Off": no scheduled check, so no countdown - but the
    /// last-check time still shows normally.
    #[test]
    fn manual_interval_has_no_countdown() {
        let h: Health = serde_json::from_str(
            r#"{"status":"ok","ts":"2026-10-20T00:30:02+08:00","interval":0}"#,
        )
        .unwrap();
        let line = synced_line(&h);
        assert!(line.starts_with("Last checked"));
        assert!(!line.contains("Next check"));
        let empty: Health = serde_json::from_str(r#"{"interval":0}"#).unwrap();
        assert_eq!(synced_line(&empty), "Waiting for the first check");
    }

    /// Fast-start telemetry is additive; the GUI must parse it without
    /// changing the existing status behavior.
    #[test]
    fn parses_fast_start_health() {
        let json = r#"{
            "status": "ok",
            "mode": "fast",
            "cache_age": 120,
            "audit_pending": true,
            "audit_done": 420,
            "audit_total": 1230,
            "phase": "audit",
            "cycle_reason": "startup",
            "effective_workers": 12
        }"#;
        let h: Health = serde_json::from_str(json).unwrap();
        assert_eq!(h.mode.as_deref(), Some("fast"));
        assert_eq!(h.cache_age, Some(120));
        assert_eq!(h.audit_pending, Some(true));
        assert_eq!(h.audit_done, Some(420));
        assert_eq!(h.audit_total, Some(1230));
        assert_eq!(h.phase.as_deref(), Some("audit"));
        assert_eq!(h.cycle_reason.as_deref(), Some("startup"));
        assert_eq!(h.effective_workers, Some(12));
    }

    #[test]
    fn cached_age_line_is_replaced_after_the_audit() {
        let cached: Health =
            serde_json::from_str(r#"{"status":"ok","cache_age":120}"#).unwrap();
        assert!(synced_line(&cached).contains("Cached view \u{b7} 120s old"));
        let checked: Health = serde_json::from_str(
            r#"{"status":"ok","ts":"2026-10-20T00:30:02+08:00","cache_age":null}"#,
        )
        .unwrap();
        assert!(synced_line(&checked).starts_with("Last checked"));
    }

    #[test]
    fn fast_start_audit_line_shows_progress() {
        let h: Health = serde_json::from_str(
            r#"{"status":"ok","audit_pending":true,"audit_done":420,"audit_total":1230}"#,
        )
        .unwrap();
        let line = synced_line(&h);
        assert!(line.contains("checking 420 / 1230"), "{line}");
        let queued: Health =
            serde_json::from_str(r#"{"status":"ok","audit_pending":true}"#).unwrap();
        assert!(synced_line(&queued).contains("background check running"));
    }

    /// A lost keyring session is its own state when this account was signed in
    /// before; a fresh install still gets the welcome screen.
    #[test]
    fn needs_login_is_its_own_status() {
        assert_eq!(
            derive_status(true, "needs_login", Some(false), false, false, true, false),
            UiStatus::NeedsLogin
        );
        // Even with the service stopped the session problem is the more
        // actionable message.
        assert_eq!(
            derive_status(false, "needs_login", Some(false), false, false, true, false),
            UiStatus::NeedsLogin
        );
        let (title, sub) = status_text(UiStatus::NeedsLogin);
        assert_eq!(title, "Your session expired");
        assert!(sub.contains("Sign in again"));
        assert_eq!(UiStatus::NeedsLogin.key(), "needs_login");
    }

    #[test]
    fn derives_account_engine_and_pause_states() {
        // Account probe still in flight (engine not confirmed either way).
        assert_eq!(
            derive_status(false, "", None, false, false, false, false),
            UiStatus::Starting
        );
        // A sign-in is running in the terminal.
        assert_eq!(
            derive_status(true, "ok", Some(false), true, false, false, false),
            UiStatus::SigningIn
        );
        // Signed out, engine may be up.
        assert_eq!(
            derive_status(true, "ok", Some(false), false, false, false, false),
            UiStatus::SignedOut
        );
        // Signed in but the service is not running.
        assert_eq!(
            derive_status(false, "ok", Some(true), false, false, false, false),
            UiStatus::EngineStopped
        );
        // ...or the user explicitly paused it.
        assert_eq!(
            derive_status(false, "ok", Some(true), false, true, false, false),
            UiStatus::Paused
        );
        // Service up, no cycle reported yet.
        assert_eq!(
            derive_status(true, "", Some(true), false, false, false, false),
            UiStatus::Starting
        );
        // Normal running states.
        assert_eq!(
            derive_status(true, "ok", Some(true), false, false, false, false),
            UiStatus::Synced
        );
        assert_eq!(
            derive_status(true, "syncing", Some(true), false, false, false, false),
            UiStatus::Syncing
        );
    }

    /// The engine's real state wins over a still-pending account probe: if it
    /// is already syncing, the Home must say so.
    #[test]
    fn engine_syncing_wins_over_the_identity_probe() {
        assert_eq!(
            derive_status(true, "syncing", None, false, false, false, false),
            UiStatus::Syncing
        );
        // Even while the sign-in terminal is open, an active engine shows
        // Syncing rather than "Signing in...".
        assert_eq!(
            derive_status(true, "syncing", Some(false), true, false, false, false),
            UiStatus::Syncing
        );
        // A finished cycle is also shown before the account probe returns.
        assert_eq!(
            derive_status(true, "ok", None, false, false, false, false),
            UiStatus::Synced
        );
    }

    #[test]
    fn session_expiry_needs_a_previous_sign_in() {
        // The identity probe failed with a login error, but this account was
        // signed in before: padlock / "Sign in again".
        assert_eq!(
            derive_status(false, "ok", Some(false), false, false, true, true),
            UiStatus::NeedsLogin
        );
        // Fresh install: no last email -> welcome screen.
        assert_eq!(
            derive_status(true, "needs_login", Some(false), false, false, false, false),
            UiStatus::SignedOut
        );
    }

    #[test]
    fn needs_login_health_beats_a_stale_identity() {
        // The identity probe can still succeed from a cached session while
        // the engine's CLI calls fail; the user must see the expiry.
        assert_eq!(
            derive_status(true, "needs_login", Some(true), false, false, true, false),
            UiStatus::NeedsLogin
        );
        // While the sign-in terminal is open, show progress instead.
        assert_eq!(
            derive_status(false, "needs_login", Some(false), true, false, true, true),
            UiStatus::SigningIn
        );
    }

    #[test]
    fn engine_error_is_shown_even_before_the_identity_probe() {
        assert_eq!(
            derive_status(true, "error", None, false, false, false, false),
            UiStatus::Error
        );
        assert_eq!(
            derive_status(true, "error", Some(false), false, false, false, false),
            UiStatus::Error
        );
    }
}
