# Milestone M9 — Sync Reliability & Diagnostics

**Status:** complete — released as `v0.9.0-gui3.30.0`
**Theme:** distinguish a broken network from a lost session, stop concurrent
session refreshes, and make a stuck machine explain itself.

## Scope

- **Failure classification:** `is_network_error` separates transport failures
  from lost sessions. Network failures retry after 30 s and never tell the
  user to sign in again.
- **Serialized session probe:** one `fs info` call under the app-wide lock
  refreshes an expired token before parallel transfers, so workers do not
  rotate the single-use refresh token concurrently.
- **`--diagnose`:** a windowless support report with health, CLI probe,
  network reachability, service state and recent engine logs.
- **Copy diagnostics:** About includes status, error, failure kind, cycle
  details and the last 40 log lines.
- **Troubleshooting guide:** sleep, VPN/proxy, session loss and local errors.

## Not covered

- **Onboarding, localization and Flatpak.** Those are M10; the first-run
  welcome/sign-in flow already exists, and a Flatpak build needs a sandboxed
  CI runtime.
- **Automatic re-login.** A truly invalidated refresh token still needs the
  user to sign in; the CLI session is single-use and rotates.
- **Concurrent upstream CLI instances outside this app.** The probe and lock
  protect this app's engine and GUI, not a second copy of the app running as
  another user.

## Release verification

- Tag `v0.9.0-gui3.30.0`; CI `43256388766` verify + release succeeded.
- 313 tests, clippy clean, release gate 12/12, UI audit passed.
- Both AppImages and both `.deb` packages verified, including the published
  `--diagnose` command.
- Evidence: `docs/audit/evidence/release-3.30.0.txt`.
