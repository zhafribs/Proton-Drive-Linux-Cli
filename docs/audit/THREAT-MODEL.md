# Threat model — Proton Drive for Linux

**Program:** Full product audit, G4  
**Date:** 2026-11-08  
**Scope:** GUI, engine, official Proton CLI subprocess, keyring session, Proton
API usage, update pipeline, systemd unit, local files and release artifacts.

## 1. System and data flow

```
Proton account keyring item
        │  (Secret Service / D-Bus, read-only)
        ▼
official `proton-drive` CLI ◄── GUI (identity, folder browsing, upload/download)
        ▲                    ◄── engine (scan, plan, transfers, event stream)
        │
        ├── Proton HTTPS API (TLS, bearer token in memory)
        └── local sync folder, state dir, config, trash

GitHub release ──► update check ──► signed SHA256SUMS ──► staged AppImage
                                    └─ Ed25519 public key in the binary
```

The app never re-implements Proton crypto or file transfer. It invokes the
official CLI for those operations. The engine reads the CLI's keyring session
only for the event stream and the account quota; both are read-only.

## 2. Assets

| Asset | Where it lives | Why it matters |
|---|---|---|
| Proton session token | OS keyring, read by the CLI and the engine | Full account access until revoked |
| User files | Local sync folder, Proton Drive | Confidentiality and availability |
| Deletion history | `state.json`, `deletes-approval.json`, local trash | Prevent accidental or malicious mass deletion |
| Account metadata | `health.json`, `quota.json`, `last-email` | Usage and identity information |
| Update trust | Ed25519 public key in the binary, signed manifest | Prevents running a malicious AppImage |
| Signing key | GitHub `UPDATE_SIGNING_KEY` secret + offline backup | Compromise breaks update trust |
| CI token | `GITHUB_TOKEN`, scoped per job | Repository/release write access |
| Config/env | `proton-sync.conf`, prefs | May contain proxy credentials |

## 3. Trust boundaries

1. **GUI ↔ official CLI**: subprocess args, a shared advisory lock, and the
   keyring. No shell is used except the sign-in terminal, which is quoted.
2. **Engine ↔ Proton API**: TLS with the CLI's bearer token; the token is never
   written to disk by this project.
3. **App ↔ GitHub releases**: untrusted network input, validated by signature,
   digest, host allow-list, size caps and a smoke boot.
4. **App ↔ systemd user manager**: generated unit file and `systemctl` calls.
5. **App ↔ local filesystem**: user files, symlinks, shared folders, state and
   lock files.
6. **CI ↔ release**: repository code, third-party actions, secrets and the
   published artifact.

## 4. Threats and mitigations

| # | Threat | Impact | Mitigation | Status |
|---|---|---|---|---|
| T1 | Two CLI processes refresh the keyring at once and lose the session | Sync stops until re-login | Shared `cli.lock`; sign-in holds the same lock; sign-in probe waits 330 s; login script never runs unprotected | Mitigated; non-identity GUI calls still have a 60 s fallback (F-044) |
| T2 | Token or keyring secret leaks through logs | Account takeover | `events.rs`/`quota.rs` keep the token in memory; no token/header logging; errors truncate to 500 chars | Covered |
| T3 | Command injection through file names or paths | Arbitrary command execution | No shell for normal CLI calls; `escape()` for `/` in remote names; `shell_quote` for the sign-in terminal; argv slices | Covered |
| T4 | Path traversal or symlink escape from cloud names | Write/delete outside sync root | `safe_rel`, `remote_path`, staged downloads, `symlink_metadata` in scan and folder counter, symlink-safe photo marker | Covered; state-signal writes remain same-user |
| T5 | Mass deletion or wrong-side wipe | Data loss | Local trash staging, remote trash, batch approval digest, wipe guard when one side looks empty | Covered |
| T6 | Malicious update AppImage | Code execution as the user | Ed25519-signed `SHA256SUMS`, SHA-256 of the AppImage, exact host allow-list, 256 MB cap, staged `--smoke`, `.old` rollback marker | Covered |
| T7 | Update check/API MITM | Fake release or data exposure | TLS via rustls; signed manifest is authority; GitHub digest is a consistency check only | Covered; rustls-webpki and webpki-roots updated for RUSTSEC-2026-0382/0381 |
| T8 | Config file readable by other local users | Proxy credentials leak | `proton-sync.conf` and prefs written with mode `0600` on Unix | Fixed for 3.20.48 |
| T9 | Shared folder plants a symlink in the photo-hold path | Truncate an arbitrary user file | Symlink marker removed before write; photo folder is user-controlled | Covered |
| T10 | CI action tag is moved or compromised | Malicious code in release job | Actions pinned to commit SHAs; `contents: read` for verify; signing secret only in the sign step | Fixed in 3.20.48 |
| T11 | Dependency vulnerability | Varies | `cargo-audit` in G4; 0 vulnerabilities after updates; 3 unmaintained zbus-family warnings documented | Accepted with re-check |
| T12 | Hanging CLI blocks the only CLI slot | Sync stalls | Engine `run_output` now polls/kills at the deadline; metadata 180 s, transfers 3600 s | Fixed in 3.20.47 |
| T13 | `XDG_*` mismatch splits state from the engine | Controls silently no-op | Unit pins `XDG_STATE_HOME`/`XDG_CONFIG_HOME`; relative/empty values rejected | Fixed in 3.20.47 |
| T14 | Pause is silently undone by a settings save | Deletions resume unexpectedly | Save skips restart while the paused marker exists | Fixed in 3.20.46 |
| T15 | Direct account API use changes or breaks | Quota/event data unavailable | Both calls are read-only GETs; failures are retried hourly/with the event fallback; no write path | Accepted; documented in `docs/COMPATIBILITY.md` |

## 5. Security invariants

1. The project never writes the Proton session token to a file or a log.
2. Every destructive batch is either staged in a trash or approved by digest.
3. No downloaded AppImage is executed before its signed manifest and SHA-256
   are verified; a failed smoke boot rolls back.
4. The release job is the only job with `contents: write`, and the signing
   secret is exposed only to the signing step.
5. State, config and lock paths are resolved once and shared by the GUI and
   the engine.

## 6. Residual risks

- A compromised maintainer signing key can sign a malicious update; the key is
  offline-backed up and kept in a GitHub secret, but there is no revocation
  list. Rotation requires a new release with a new public key.
- The advisory CLI lock can still be bypassed by a non-cooperating process or
  after the 60 s fallback; the keyring itself is the final authority.
- Proton API changes can break the read-only quota/event calls; the app falls
  back to interval polling and a quota waiting state.
- Local malware with the user's privileges can read the sync folder and the
  state directory; the app does not defend against a compromised user account.
