//! Release checking (GitHub `latest` + ETag cache). The download/swap part
//! of the update flow lands with its own milestone; checking is independent
//! of the identity fetch and must stay that way.

use crate::paths::{Paths, RELEASE_API};
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    /// ("0.9.0", "3.14.0") from a `v0.9.0-gui3.14.0` tag.
    pub cli: String,
    pub gui: String,
    /// published date, first 10 chars ("2026-10-18")
    pub published: String,
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn asset_url(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.url.as_str())
    }
}

/// "v0.9.0-gui3.14.0" -> ("0.9.0", "3.14.0").
pub fn parse_tag(tag: &str) -> Option<(String, String)> {
    let t = tag.trim().trim_start_matches('v');
    let (cli, rest) = t.split_once("-gui")?;
    let gui = rest.trim().trim_start_matches('v');
    if cli.split('.').count() == 3 && !gui.is_empty() {
        Some((cli.to_string(), gui.to_string()))
    } else {
        None
    }
}

fn asset_from(v: &Value) -> Option<Asset> {
    Some(Asset {
        name: v.get("name")?.as_str()?.to_string(),
        url: v.get("browser_download_url")?.as_str()?.to_string(),
    })
}

fn parse_release(v: &Value) -> Option<Release> {
    let tag = v.get("tag_name")?.as_str()?.to_string();
    let (cli, gui) = parse_tag(&tag)?;
    let published: String = v
        .get("published_at")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(10)
        .collect();
    let assets = v
        .get("assets")?
        .as_array()?
        .iter()
        .filter_map(asset_from)
        .collect();
    Some(Release { tag, cli, gui, published, assets })
}

/// GET the latest release (conditional on the cached ETag). Returns the
/// release and whether it came from the cache (304).
pub fn check(paths: &Paths) -> Result<(Release, bool), String> {
    let cached: Value = std::fs::read_to_string(&paths.release_cache_file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let etag = cached
        .get("etag")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let agent = ureq::Agent::new_with_defaults();
    let mut req = agent
        .get(RELEASE_API)
        .header("User-Agent", "Proton-Drive-Linux-Cli")
        .header("Accept", "application/vnd.github+json");
    if !etag.is_empty() {
        req = req.header("If-None-Match", &etag);
    }
    let resp = req
        .call()
        .map_err(|e| format!("release check failed: {e}"))?;
    let status = resp.status().as_u16();

    if status == 304 {
        if let Some(rel) = cached.get("release").and_then(parse_release) {
            return Ok((rel, true));
        }
        // fall through to a full refetch when the cache is unusable
        let resp = agent
            .get(RELEASE_API)
            .header("User-Agent", "Proton-Drive-Linux-Cli")
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| format!("release check failed: {e}"))?;
        return read_fresh(resp, &paths.release_cache_file);
    }
    read_fresh(resp, &paths.release_cache_file)
}

fn read_fresh(
    resp: ureq::http::Response<ureq::Body>,
    cache_file: &std::path::Path,
) -> Result<(Release, bool), String> {
    if resp.status().as_u16() != 200 {
        return Err(format!("release check: HTTP {}", resp.status()));
    }
    let new_etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp
        .into_body()
        .read_to_string()
        .map_err(|e| format!("release body: {e}"))?;
    let v: Value = serde_json::from_str(&body).map_err(|e| format!("release json: {e}"))?;
    let rel = parse_release(&v).ok_or_else(|| "release tag unrecognized".to_string())?;

    let cache = serde_json::json!({ "etag": new_etag, "release": v });
    if let Some(dir) = cache_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(cache_file, serde_json::to_vec_pretty(&cache).unwrap_or_default());
    Ok((rel, false))
}

/// Download an asset to a temp path; returns bytes written.
pub fn download(url: &str, dest: &std::path::Path) -> Result<u64, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(1800)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut resp = agent
        .get(url)
        .header("User-Agent", "Proton-Drive-Linux-Cli")
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    if resp.status().as_u16() != 200 {
        return Err(format!("download: HTTP {}", resp.status()));
    }
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let n = std::io::copy(&mut resp.body_mut().as_reader(), &mut file)
        .map_err(|e| format!("download write: {e}"))?;
    Ok(n)
}

/// Qt's update comparison: an update exists when the release's CLI is newer
/// than the installed one, or the CLI matches and the release's GUI is newer.
/// An unknown installed version ("" - the CLI not installed or not yet
/// probed) never reports an update rather than a bogus one.
pub fn has_update(cur_cli: &str, rel: &Release) -> bool {
    !cur_cli.is_empty()
        && !rel.gui.is_empty()
        && (crate::util::is_newer(cur_cli, &rel.cli)
            || (rel.cli == cur_cli
                && crate::util::is_newer(crate::GUI_VERSION, &rel.gui)))
}

// ------------------------------------------------------------- the update swap
//
// The same "verified updates activate" rule the PyQt app followed: the new
// build is staged beside the running one, booted once offscreen (--smoke)
// and checksum-verified; only then may it replace the running copy, and the
// swap records a marker so a version that fails to start is rolled back
// automatically on the next launch.

/// The running AppImage's own path (set by the runtime), when it is one.
pub fn appimage_target() -> Option<std::path::PathBuf> {
    let t = std::env::var_os("APPIMAGE")?;
    let p = std::path::PathBuf::from(t);
    p.is_file().then_some(p)
}

fn http_get_text(url: &str, timeout_s: u64) -> Result<String, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_s)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut resp = agent
        .get(url)
        .header("User-Agent", "Proton-Drive-Linux-Cli")
        .call()
        .map_err(|e| format!("fetch failed: {e}"))?;
    if resp.status().as_u16() != 200 {
        return Err(format!("fetch: HTTP {}", resp.status()));
    }
    resp.body_mut()
        .read_to_string()
        .map_err(|e| format!("fetch body: {e}"))
}

/// The checksum line of SHA256SUMS for `name` ("<hash>  <name>"; the name
/// may carry a leading "*" for binary mode, and releases up to 3.15.0
/// listed it as "./name" the way sha256sum prints path arguments - match
/// either form).
fn sum_for(text: &str, name: &str) -> Option<String> {
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let file = parts[1].trim_start_matches('*').trim_start_matches("./");
            if file == name {
                return Some(parts[0].to_string());
            }
        }
    }
    None
}

fn sha256_file(path: &std::path::Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Probe: can we create files in this directory? (A read-only install dir
/// falls back to /tmp; the swap then copies across.)
fn dir_writable(dir: &std::path::Path) -> bool {
    let probe = dir.join(format!(".proton-write-test-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)
    {
        Ok(f) => {
            drop(f);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Download the AppImage for this machine and verify its checksum.
pub fn download_update(rel: &Release, arch: &str) -> Result<std::path::PathBuf, String> {
    let name = if !rel.gui.is_empty() {
        format!("ProtonDrive-CLI-{}-GUI-{}-{arch}.AppImage", rel.cli, rel.gui)
    } else {
        // legacy release tagged v<cli> without a GUI suffix
        format!("ProtonDrive-CLI-{}-{arch}.AppImage", rel.cli)
    };
    let url = rel
        .asset_url(&name)
        .ok_or_else(|| format!("no build for {arch} in release {}", rel.tag))?;
    let sums = rel
        .asset_url("SHA256SUMS")
        .ok_or_else(|| format!("no checksums in release {}", rel.tag))?;

    // Download beside the running AppImage, not into /tmp: /tmp is usually a
    // tmpfs and the final swap is a rename(), which cannot cross filesystems.
    let base = appimage_target()
        .filter(|t| t.parent().is_some_and(dir_writable))
        .and_then(|t| t.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(std::env::temp_dir);
    let dest = base.join(format!("proton-update-{name}"));

    if let Err(e) = download(url, &dest) {
        let _ = std::fs::remove_file(&dest);
        return Err(e);
    }
    let want = match http_get_text(sums, 30) {
        Ok(text) => sum_for(&text, &name),
        Err(_) => None,
    };
    let Some(want) = want else {
        let _ = std::fs::remove_file(&dest);
        return Err("checksum entry not found".to_string());
    };
    match sha256_file(&dest) {
        Ok(got) if got == want => {}
        Ok(_) => {
            let _ = std::fs::remove_file(&dest);
            return Err("download corrupted (checksum mismatch)".to_string());
        }
        Err(e) => {
            let _ = std::fs::remove_file(&dest);
            return Err(e);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }
    Ok(dest)
}

/// Boot the freshly downloaded AppImage once, offscreen, before it may
/// replace the running version. `PROTON_SYNC_SMOKE` keeps the dry run from
/// touching the live sync service; a build that cannot start never takes
/// your working copy's place.
pub fn verify_staged_build(img: &std::path::Path) -> bool {
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new(img)
        .arg("--smoke")
        .env("PROTON_SYNC_SMOKE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    // No wait-timeout in std: poll try_wait for at most 120s.
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    return false;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(_) => return false,
        }
    }
}

/// The swap's sidecar files: `<app>.new` (staging) and `<app>.old` (backup).
fn sidecar(target: &std::path::Path, ext: &str) -> std::path::PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(ext);
    std::path::PathBuf::from(s)
}

/// Recover from an update whose new version never came up.
///
/// The swap records a marker before it happens; the first launch of the new
/// version marks itself attempted and, once its window is up, confirms
/// health (`update_confirm_health`). If the same marker is still
/// unconfirmed on the next launch, the previous version never started and
/// is restored from `<app>.old`.
///
/// Returns (`state`, `relaunch`) where state is "pending" (this run is the
/// freshly updated version), "rolled-back" (old version restored; the
/// caller should relaunch it), or "" (nothing to do). Also sweeps a
/// staging file left behind by an interrupted download.
pub fn update_recovery_check(
    paths: &crate::paths::Paths,
) -> (&'static str, Option<std::path::PathBuf>) {
    use serde_json::Value;
    // Read APPIMAGE directly (no is_file gate): the "swap died" case below
    // is exactly when the target file is missing.
    if let Some(target) = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from) {
        let new = sidecar(&target, ".new");
        let old = sidecar(&target, ".old");
        if !target.is_file() {
            // The swap died between its two renames: the launcher's file is
            // gone. The staged build already passed its start-up check, so
            // prefer it; otherwise put the previous version back.
            if new.is_file() {
                let _ = std::fs::rename(&new, &target);
            } else if old.is_file() {
                let _ = std::fs::rename(&old, &target);
            }
        } else {
            let _ = std::fs::remove_file(new); // interrupted download
        }
    }
    let Ok(text) = std::fs::read_to_string(&paths.update_mark_file) else {
        return ("", None);
    };
    let Ok(mut mark) = serde_json::from_str::<Value>(&text) else {
        return ("", None);
    };
    let old = mark.get("old").and_then(Value::as_str).map(String::from);
    let new = mark.get("new").and_then(Value::as_str).map(String::from);
    if mark.get("attempted").and_then(Value::as_bool).unwrap_or(false) {
        // Second launch without a health ack: the new version never ran.
        let _ = std::fs::remove_file(&paths.update_mark_file);
        if let (Some(old), Some(new)) = (old, new) {
            let (old, new) = (std::path::PathBuf::from(old), std::path::PathBuf::from(new));
            if old.is_file() && new.is_file() && std::fs::rename(&old, &new).is_ok() {
                return ("rolled-back", Some(new));
            }
        }
        return ("", None);
    }
    mark["attempted"] = Value::Bool(true);
    let _ = std::fs::write(
        &paths.update_mark_file,
        serde_json::to_vec(&mark).unwrap_or_default(),
    );
    ("pending", None)
}

/// The freshly updated version is up and its window was built: keep it.
/// Dropping the marker and the .old backup is the health acknowledgement.
pub fn update_confirm_health(paths: &crate::paths::Paths) {
    use serde_json::Value;
    let Ok(text) = std::fs::read_to_string(&paths.update_mark_file) else {
        return;
    };
    let _ = std::fs::remove_file(&paths.update_mark_file);
    if let Ok(v) = serde_json::from_str::<Value>(&text) {
        if let Some(old) = v.get("old").and_then(Value::as_str) {
            let _ = std::fs::remove_file(old);
        }
    }
}

/// Swap the running AppImage for the freshly downloaded one, then return
/// its path (the caller relaunches it once the single-instance lock is
/// free - a new copy started now would just report "Already running").
pub fn install_update_and_restart(
    dl: &std::path::Path,
    tag: &str,
) -> Result<std::path::PathBuf, String> {
    let target = appimage_target().ok_or("not running as an AppImage")?;
    let stage = sidecar(&target, ".new");
    let old = sidecar(&target, ".old");
    // Same directory: one atomic rename; a /tmp staging falls back to copy.
    if std::fs::rename(dl, &stage).is_err() {
        std::fs::copy(dl, &stage).map_err(|e| format!("could not stage the update ({e})"))?;
        let _ = std::fs::remove_file(dl);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o755));
    }
    if !verify_staged_build(&stage) {
        let _ = std::fs::remove_file(&stage);
        return Err(
            "the new version failed its start-up check - you keep the current one".to_string(),
        );
    }
    let mark = serde_json::json!({
        "old": old.to_string_lossy(),
        "new": target.to_string_lossy(),
        "tag": tag,
        "started": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });
    let mark_file = crate::paths::resolve().update_mark_file;
    let _ = std::fs::write(&mark_file, serde_json::to_vec(&mark).unwrap_or_default());
    if let Err(e) = std::fs::rename(&target, &old) {
        let _ = std::fs::remove_file(&stage);
        return Err(format!("could not replace the AppImage ({e})"));
    }
    if let Err(e) = std::fs::rename(&stage, &target) {
        // Recovery handles the missing target on the next launch (.old).
        let _ = std::fs::remove_file(&stage);
        return Err(format!("could not replace the AppImage ({e})"));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_tag() {
        let (cli, gui) = parse_tag("v0.9.0-gui3.14.0").unwrap();
        assert_eq!(cli, "0.9.0");
        assert_eq!(gui, "3.14.0");
        assert!(parse_tag("v0.9.0").is_none());
        assert!(parse_tag("nonsense").is_none());
    }

    #[test]
    fn parses_latest_release_json() {
        let v = serde_json::json!({
            "tag_name": "v0.9.0-gui3.13.2",
            "published_at": "2026-10-18T09:30:00Z",
            "assets": [
                {"name": "ProtonDrive-CLI-0.9.0-GUI-3.13.2-x86_64.AppImage",
                 "browser_download_url": "https://x/a.AppImage", "size": 100},
                {"name": "SHA256SUMS",
                 "browser_download_url": "https://x/SHA256SUMS", "size": 10}
            ]
        });
        let r = parse_release(&v).unwrap();
        assert_eq!(r.tag, "v0.9.0-gui3.13.2");
        assert_eq!(r.gui, "3.13.2");
        assert_eq!(r.published, "2026-10-18");
        assert_eq!(r.assets.len(), 2);
        assert_eq!(
            r.asset_url("SHA256SUMS"),
            Some("https://x/SHA256SUMS")
        );
        assert_eq!(r.asset_url("nope"), None);
    }

    #[test]
    fn finds_checksum_line() {
        let text = "aaaa  ProtonDrive-CLI-0.9.0-GUI-3.14.0-x86_64.AppImage\n\
                    bbbb *ProtonDrive-CLI-0.9.0-GUI-3.14.0-aarch64.AppImage\n\
                    cccc  ./ProtonDrive-CLI-0.9.0-GUI-3.13.2-x86_64.AppImage\n";
        assert_eq!(
            sum_for(text, "ProtonDrive-CLI-0.9.0-GUI-3.14.0-x86_64.AppImage").as_deref(),
            Some("aaaa")
        );
        // "*" prefix (binary marker) is tolerated
        assert_eq!(
            sum_for(text, "ProtonDrive-CLI-0.9.0-GUI-3.14.0-aarch64.AppImage").as_deref(),
            Some("bbbb")
        );
        // "./" prefix - how every release up to 3.15.0 listed its files
        assert_eq!(
            sum_for(text, "ProtonDrive-CLI-0.9.0-GUI-3.13.2-x86_64.AppImage").as_deref(),
            Some("cccc")
        );
        assert_eq!(sum_for(text, "missing"), None);
    }

    #[test]
    fn staged_paths_are_plain_appends() {
        let t = std::path::PathBuf::from("/opt/bin/Proton.AppImage");
        assert_eq!(sidecar(&t, ".new"), std::path::PathBuf::from("/opt/bin/Proton.AppImage.new"));
        assert_eq!(sidecar(&t, ".old"), std::path::PathBuf::from("/opt/bin/Proton.AppImage.old"));
    }

    fn rel(cli: &str, gui: &str) -> Release {
        Release {
            tag: format!("v{cli}-gui{gui}"),
            cli: cli.to_string(),
            gui: gui.to_string(),
            published: "2026-10-18".to_string(),
            assets: vec![],
        }
    }

    #[test]
    fn update_comparison_is_qt_semantics() {
        // newer CLI always wins
        assert!(has_update("0.9.0", &rel("0.10.0", "3.14.0")));
        // same CLI, newer GUI (3.99.0 stays ahead of GUI_VERSION for any bump)
        assert!(has_update("0.9.0", &rel("0.9.0", "3.99.0")));
        // exactly what is installed: no update (dev GUI newer than release)
        assert!(!has_update("0.9.0", &rel("0.9.0", "3.13.2")));
        // older release CLI than installed: no update even with a newer GUI tag
        assert!(!has_update("0.10.0", &rel("0.9.0", "3.99.0")));
        // unknown installed version: never a bogus "update available"
        assert!(!has_update("", &rel("0.10.0", "3.14.0")));
        // legacy GUI-less release: no update (GUI comparison impossible)
        assert!(!has_update("0.9.0", &rel("0.9.0", "")));
    }
}
