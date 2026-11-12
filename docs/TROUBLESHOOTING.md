# Sync stopped or the status is red

The app has three different failures that look similar on the Home page. Check
the sentence under the orb first; it names the actual cause.

## 1. Network or proxy failure

Symptoms: "Couldn't sync", a timeout, "connection refused", DNS error, TLS
error or a corporate proxy message. The failure kind is **network**.

The engine retries every 30 seconds and clears the error by itself once the
network is back. Wake-from-sleep often beats the Wi-Fi or VPN; give it a
minute, then use **Sync now** if the network is definitely up.

Corporate proxies and VPNs must allow `drive-api.proton.me:443`. A captive
portal that returns an HTML/401 page is not a Proton session problem.

## 2. Expired or lost session

Symptoms: "Session expired", "You need to login first", "401 Unauthorized" or a
keyring error. The failure kind is **session**.

Use **Sign in again** in the account pill. Restarting the engine or running a
full sync cannot recover a session that Proton has already rotated away; only
a new sign-in can.

A lost session can happen when several CLI processes refresh the single-use
refresh token at the same time. This release serializes a session probe before
parallel transfers, which closes the common window, but the underlying CLI
still owns the session. If it happens repeatedly, send the diagnostics below.

## 3. A real file or state error

Symptoms: "permission denied", a specific path, a corrupt state file, or
"another proton-sync run is already active". The failure kind is **other**.
The message names the path or lock that needs attention.

## Collect a report

On the machine that will not sync:

```bash
./ProtonDrive-CLI-*-x86_64.AppImage --diagnose > proton-diagnose.txt
```

The report contains:

- app, platform, config and state paths
- service active/enabled state
- resolved CLI and a read-only `fs info` session probe
- TCP reachability of `drive-api.proton.me:443`
- the full `health.json`
- the last 80 engine log lines

No file contents, names or tokens are included. You can also use
**About → Diagnostics → Copy diagnostics** in the window; it adds the sync
error and the last 40 log lines.

## Manual checks

```bash
cat ~/.local/state/proton-sync/health.json
journalctl --user -u proton-sync -n 200 --no-pager
systemctl --user status proton-sync
```

The engine writes `health.json` before and after every cycle. A red status
that never changes means the same error is returning every cycle, not a stale
window.
