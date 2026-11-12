//! Release checking (GitHub `latest` + ETag cache). The download/swap part
//! of the update flow lands with its own milestone; checking is independent
//! of the identity fetch and must stay that way.

use crate::paths::{Paths, RELEASE_API};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A build is ~60 MB; anything past this is not an AppImage we will execute.
const MAX_UPDATE_BYTES: u64 = 256 * 1024 * 1024;
/// SHA256SUMS is a few hundred bytes; cap a hostile response before reading.
const MAX_SUMS_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Asset {
    pub name: String,
    pub url: String,
    /// The release API's own `sha256:<hex>` digest, when GitHub supplies it.
    pub digest: Option<String>,
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

    /// The release API's raw per-asset digest. Callers must pass it through
    /// [`normalize_sha256`] before trusting it.
    fn asset_digest_raw(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|a| a.name == name)
            .and_then(|a| a.digest.as_deref())
    }
}

/// The project's primary Ed25519 update-manifest public key (raw 32-byte hex).
/// The private key lives in the GitHub `UPDATE_SIGNING_KEY` secret and an
/// offline backup; it is never in this repository.
const UPDATE_PUBLIC_KEY_HEX: &str = "49a5c084f6e0673bb7ad3c102db4186ff6048529c6533b334ebfc868db0e51bb";
/// Successor key. Its private half is offline; it signs the key-revocation
/// feed so a compromised primary key cannot suppress its own revocation.
const UPDATE_SUCCESSOR_PUBLIC_KEY_HEX: &str =
    "f0ed7f4fa4fac94e0bf72931ee0ab16fd4882867c167572c988bec85986444c1";
const KEY_REVOCATIONS_URL: &str =
    "https://raw.githubusercontent.com/zhafribs/Proton-Drive-Linux-Cli/main/security/key-revocations.json";
const KEY_REVOCATIONS_SIG_URL: &str =
    "https://raw.githubusercontent.com/zhafribs/Proton-Drive-Linux-Cli/main/security/key-revocations.json.sig";

fn hex_bytes<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn verify_manifest_with(key_hex: &str, manifest: &[u8], signature: &[u8]) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};
    let Some(key) = hex_bytes::<32>(key_hex) else {
        return false;
    };
    let Ok(verifying) = VerifyingKey::from_bytes(&key) else {
        return false;
    };
    let Ok(bytes) = <[u8; 64]>::try_from(signature) else {
        return false;
    };
    verifying
        .verify_strict(manifest, &Signature::from_bytes(&bytes))
        .is_ok()
}

/// Keys allowed to sign update manifests. Rotation adds a new key here in a
/// release signed by the old one; the old key stays until the migration
/// window ends.
const UPDATE_PUBLIC_KEYS: &[&str] = &[UPDATE_PUBLIC_KEY_HEX, UPDATE_SUCCESSOR_PUBLIC_KEY_HEX];
/// Keys revoked by a later release. A revoked key is skipped even when its
/// signature would otherwise verify.
const REVOKED_UPDATE_KEYS: &[&str] = &[];

fn verify_manifest_with_keys(
    keys: &[&str],
    revoked: &[&str],
    manifest: &[u8],
    signature: &[u8],
) -> bool {
    keys.iter()
        .filter(|key| !revoked.contains(key))
        .any(|key| verify_manifest_with(key, manifest, signature))
}

/// An update is installable only when `SHA256SUMS.sig` is a valid signature by
/// a non-revoked project key over `SHA256SUMS`. The digest in that signed
/// manifest is the authority; the GitHub API digest is only a consistency
/// check.
fn manifest_signing_key(manifest: &[u8], signature: &[u8]) -> Option<&'static str> {
    UPDATE_PUBLIC_KEYS
        .iter()
        .filter(|key| !REVOKED_UPDATE_KEYS.contains(key))
        .find(|key| verify_manifest_with(key, manifest, signature))
        .copied()
}

#[cfg(test)]
fn verify_manifest_signature(manifest: &[u8], signature: &[u8]) -> bool {
    manifest_signing_key(manifest, signature).is_some()
}

fn key_is_revoked(key: &str, feed: &[u8]) -> Result<bool, String> {
    #[derive(serde::Deserialize)]
    struct Feed {
        #[serde(default)]
        revoked_keys: Vec<String>,
    }
    let feed: Feed = serde_json::from_slice(feed)
        .map_err(|e| format!("key revocation feed is not valid JSON: {e}"))?;
    Ok(feed.revoked_keys.iter().any(|k| k == key))
}

/// Check the manifest signature and the successor-signed key-revocation feed.
/// The feed is required, so a compromised primary key cannot suppress its own
/// revocation; a missing feed fails closed.
fn check_manifest_trust(manifest: &[u8], signature: &[u8]) -> Result<(), String> {
    let key = manifest_signing_key(manifest, signature)
        .ok_or_else(|| "update manifest is unsigned or its signature is invalid".to_string())?;
    let feed = http_get_text_retry(KEY_REVOCATIONS_URL, 30)
        .map_err(|e| format!("key revocation feed fetch failed: {e}"))?;
    let sig = http_get_bytes_retry(KEY_REVOCATIONS_SIG_URL, 30, 1024)
        .map_err(|e| format!("key revocation signature fetch failed: {e}"))?;
    check_manifest_trust_with(key, UPDATE_SUCCESSOR_PUBLIC_KEY_HEX, feed.as_bytes(), &sig)
}

/// The pure trust decision: the feed must be signed by the successor key and
/// must not list the key that signed the manifest.
fn check_manifest_trust_with(
    key: &str,
    successor_key: &str,
    feed: &[u8],
    feed_signature: &[u8],
) -> Result<(), String> {
    if !verify_manifest_with(successor_key, feed, feed_signature) {
        return Err("key revocation feed has an invalid successor signature".into());
    }
    if key_is_revoked(key, feed)? {
        return Err("the update manifest is signed by a revoked key".into());
    }
    Ok(())
}

/// Releases at or after this GUI version must carry a signed revocation feed.
/// Older releases predate the feature and are accepted without one.
const REVOCATIONS_MIN_GUI: &str = "3.27.0";

fn is_revoked(tag: &str, feed: &[u8]) -> Result<bool, String> {
    #[derive(serde::Deserialize)]
    struct Feed {
        #[serde(default)]
        revoked_tags: Vec<String>,
    }
    let feed: Feed = serde_json::from_slice(feed)
        .map_err(|e| format!("release revocation feed is not valid JSON: {e}"))?;
    Ok(feed.revoked_tags.iter().any(|t| t == tag))
}

fn check_revocations(rel: &Release) -> Result<(), String> {
    let required =
        rel.gui == REVOCATIONS_MIN_GUI || crate::util::is_newer(REVOCATIONS_MIN_GUI, &rel.gui);
    let Some(url) = release_asset_url(rel, "revocations.json") else {
        return if required {
            Err(format!("release {} has no revocation feed URL", rel.tag))
        } else {
            Ok(())
        };
    };
    let sig_url = release_asset_url(rel, "revocations.json.sig").unwrap_or_default();
    let feed = match http_get_text_retry(&url, 30) {
        Ok(text) => text,
        Err(_) if !required => return Ok(()),
        Err(e) => {
            return Err(format!(
                "release {} is missing its signed revocation feed: {e}",
                rel.tag
            ));
        }
    };
    let signature = http_get_bytes_retry(&sig_url, 30, 1024)
        .map_err(|e| format!("release revocation signature fetch failed: {e}"))?;
    if !verify_manifest_with_keys(
        UPDATE_PUBLIC_KEYS,
        REVOKED_UPDATE_KEYS,
        feed.as_bytes(),
        &signature,
    ) {
        return Err("release revocation feed has an invalid signature".into());
    }
    if is_revoked(&rel.tag, feed.as_bytes())? {
        return Err(format!("release {} has been revoked", rel.tag));
    }
    Ok(())
}

/// Accept only a 64-character SHA-256; strip the API's `sha256:` prefix and
/// normalize case. A malformed API digest must be rejected, not ignored.
fn normalize_sha256(raw: &str) -> Option<String> {
    let hex = raw.strip_prefix("sha256:").unwrap_or(raw).to_ascii_lowercase();
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hex)
}

/// The updater only talks to this repository's releases and GitHub's asset
/// CDN. Hosts are matched exactly, so `github.com.evil` or userinfo tricks do
/// not pass. The digest is still the integrity check; this is defence in depth.
fn trusted_download_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let Some((host, path)) = rest.split_once('/') else {
        return false;
    };
    match host {
        "github.com" => path.starts_with("zhafribs/Proton-Drive-Linux-Cli/releases/download/"),
        "release-assets.githubusercontent.com" | "objects.githubusercontent.com" => true,
        _ => false,
    }
}

fn host_of(url: &str) -> Option<&str> {
    url.strip_prefix("https://")?.split('/').next()
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
    let digest = v
        .get("digest")
        .and_then(Value::as_str)
        .and_then(|d| d.strip_prefix("sha256:"))
        .map(str::to_string);
    Some(Asset {
        name: v.get("name")?.as_str()?.to_string(),
        url: v.get("browser_download_url")?.as_str()?.to_string(),
        digest,
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
    Some(Release {
        tag,
        cli,
        gui,
        published,
        assets,
    })
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

    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
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

    if status == 403 || status == 429 {
        // The unauthenticated API quota is 60 requests/hour. GitHub's
        // `releases/latest` redirect is not rate-limited, so fall back to it
        // and let the signed manifest carry the integrity data.
        if let Ok(rel) = check_via_redirect() {
            check_revocations(&rel)?;
            return Ok((rel, false));
        }
        // Say what happened instead of a bare 403.
        let minutes = resp
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<i64>().ok())
            .map(|reset| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                (reset - now).max(0) / 60 + 1
            });
        return Err(match minutes {
            Some(m) => format!(
                "release check: GitHub is rate-limiting update checks - try again in ~{m} min"
            ),
            None => "release check: GitHub is rate-limiting update checks - try again later".into(),
        });
    }

    if status == 304 {
        if let Some(rel) = cached.get("release").and_then(parse_release) {
            check_revocations(&rel)?;
            return Ok((rel, true));
        }
        // fall through to a full refetch when the cache is unusable
        let resp = agent
            .get(RELEASE_API)
            .header("User-Agent", "Proton-Drive-Linux-Cli")
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| format!("release check failed: {e}"))?;
        let (rel, cached) = read_fresh(resp, &paths.release_cache_file)?;
        check_revocations(&rel)?;
        return Ok((rel, cached));
    }
    let (rel, cached) = read_fresh(resp, &paths.release_cache_file)?;
    check_revocations(&rel)?;
    Ok((rel, cached))
}

/// The latest release tag without the API: `releases/latest` redirects to the
/// tag page. Used only when the API is rate-limited. The result has no asset
/// list; [`release_asset_url`] builds the expected release URLs from the tag.
fn check_via_redirect() -> Result<Release, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(0)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let resp = agent
        .get(crate::paths::RELEASE_PAGE)
        .header("User-Agent", "Proton-Drive-Linux-Cli")
        .call()
        .map_err(|e| format!("release redirect failed: {e}"))?;
    if !(300..400).contains(&resp.status().as_u16()) {
        return Err(format!("release redirect: HTTP {}", resp.status()));
    }
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "release redirect: no location".to_string())?;
    let tag = tag_from_release_location(location)
        .ok_or_else(|| "release redirect: no tag".to_string())?;
    let (cli, gui) =
        parse_tag(tag).ok_or_else(|| "release redirect: invalid tag shape".to_string())?;
    Ok(Release {
        tag: tag.to_string(),
        cli,
        gui,
        published: String::new(),
        assets: Vec::new(),
    })
}

/// `/tag/<tag>` from a `releases/latest` redirect target, rejecting anything
/// that is not a plain single path segment.
fn tag_from_release_location(location: &str) -> Option<&str> {
    let (_, tag) = location.split_once("/tag/")?;
    let tag = tag.split(['?', '#']).next()?;
    if tag.is_empty() || tag.contains('/') || tag.contains(' ') || tag.contains('\\') {
        return None;
    }
    Some(tag)
}

/// A release asset URL from the API list, or the canonical release-download URL
/// when the release came from the rate-limit fallback.
fn release_asset_url(rel: &Release, name: &str) -> Option<String> {
    if let Some(url) = rel.asset_url(name) {
        return Some(url.to_string());
    }
    if rel.tag.is_empty()
        || rel.tag.contains('/')
        || rel.tag.contains(' ')
        || rel.tag.contains('\\')
    {
        return None;
    }
    Some(format!(
        "https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/download/{}/{name}",
        rel.tag
    ))
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
    let _ = std::fs::write(
        cache_file,
        serde_json::to_vec_pretty(&cache).unwrap_or_default(),
    );
    Ok((rel, false))
}

/// GET following at most a few redirects, validating every hop. GitHub serves
/// release assets from `release-assets.githubusercontent.com`; anything else
/// is refused before its bytes are read.
fn get_following(
    agent: &ureq::Agent,
    url: &str,
) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut current = url.to_string();
    for _ in 0..4 {
        if !trusted_download_url(&current) {
            return Err(format!("untrusted download URL: {current}"));
        }
        let resp = agent
            .get(&current)
            .header("User-Agent", "Proton-Drive-Linux-Cli")
            .call()
            .map_err(|e| format!("download failed: {e}"))?;
        let status = resp.status().as_u16();
        if (300..400).contains(&status) {
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("redirect without a location (HTTP {status})"))?;
            current = if location.starts_with("https://") {
                location.to_string()
            } else if location.starts_with('/') {
                format!("https://{}{}", host_of(&current).unwrap_or(""), location)
            } else {
                return Err(format!("unexpected redirect target: {location}"));
            };
            continue;
        }
        return Ok(resp);
    }
    Err("too many redirects".to_string())
}

/// Copy at most `limit` bytes; one byte over is an error, not a truncated file.
fn copy_capped<R: Read, W: std::io::Write>(
    reader: &mut R,
    writer: &mut W,
    limit: u64,
) -> std::io::Result<u64> {
    let n = std::io::copy(&mut reader.take(limit + 1), writer)?;
    if n > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "download too large",
        ));
    }
    Ok(n)
}

/// Download an asset to a new file; returns bytes written.
pub fn download(url: &str, dest: &std::path::Path) -> Result<u64, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(1800)))
        .max_redirects(0)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut resp = get_following(&agent, url)?;
    if resp.status().as_u16() != 200 {
        return Err(format!("download: HTTP {}", resp.status()));
    }
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(dest)
            .map_err(|e| e.to_string())?
    };
    #[cfg(not(unix))]
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(dest)
        .map_err(|e| e.to_string())?;
    let mut reader = resp.body_mut().as_reader();
    copy_capped(&mut reader, &mut file, MAX_UPDATE_BYTES).map_err(|e| match e.kind() {
        std::io::ErrorKind::InvalidData => "update is too large".to_string(),
        _ => format!("download write: {e}"),
    })
}

/// Qt's update comparison: an update exists when the release's CLI is newer
/// than the installed one, or the CLI matches and the release's GUI is newer.
/// An unknown installed version ("" - the CLI not installed or not yet
/// probed) never reports an update rather than a bogus one.
pub fn has_update(cur_cli: &str, rel: &Release) -> bool {
    !cur_cli.is_empty()
        && !rel.gui.is_empty()
        && (crate::util::is_newer(cur_cli, &rel.cli)
            || (rel.cli == cur_cli && crate::util::is_newer(crate::GUI_VERSION, &rel.gui)))
}

// ------------------------------------------------------------- the update swap
//
// The same "verified updates activate" rule the PyQt app followed: the new
// build is staged beside the running one, booted once offscreen (--smoke)
// and checksum-verified; only then may it replace the running copy, and the
// swap records a marker so a version that fails to start is rolled back
// automatically on the next launch.

/// The `APPIMAGE` path whether or not that file still exists: an update can
/// install the new build under a new name, and then the running app's own path
/// is stale (see [`install_target`]).
fn appimage_path() -> Option<std::path::PathBuf> {
    std::env::var_os("APPIMAGE").map(std::path::PathBuf::from)
}

/// Where a downloaded update should be installed.
///
/// Normally the running AppImage is replaced in place, so the user keeps their
/// file name. If that file is gone (a previous update installed a different
/// name), the build is placed beside it instead of failing the whole update
/// with "not running as an AppImage".
/// A directory is safe for an update only if we can create files there and
/// other users cannot replace our sidecars. `~/Applications` (0755) passes;
/// `/tmp` (1777) does not.
fn safe_install_dir(dir: &std::path::Path) -> bool {
    if !dir_writable(dir) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(meta) = std::fs::symlink_metadata(dir) else {
            return false;
        };
        if !meta.is_dir() || meta.mode() & 0o022 != 0 {
            return false;
        }
    }
    true
}

fn install_target_with(
    recorded: Option<&std::path::Path>,
    asset: &str,
    writable: bool,
) -> Option<std::path::PathBuf> {
    let asset_path = std::path::Path::new(asset);
    if asset.is_empty()
        || asset == "."
        || asset == ".."
        || asset_path.components().count() != 1
    {
        return None;
    }
    let recorded = recorded?;
    if recorded.is_file() && writable {
        return Some(recorded.to_path_buf());
    }
    let dir = recorded.parent()?;
    writable.then(|| dir.join(asset))
}

fn install_target(recorded: Option<&std::path::Path>, asset: &str) -> Option<std::path::PathBuf> {
    let recorded = recorded?;
    let dir = recorded.parent()?;
    install_target_with(Some(recorded), asset, safe_install_dir(dir))
}

fn http_get_text(url: &str, timeout_s: u64) -> Result<String, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_s)))
        .max_redirects(0)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut resp = get_following(&agent, url)?;
    if resp.status().as_u16() != 200 {
        return Err(format!("fetch: HTTP {}", resp.status()));
    }
    let mut text = String::new();
    let mut reader = resp.body_mut().as_reader().take(MAX_SUMS_BYTES + 1);
    reader
        .read_to_string(&mut text)
        .map_err(|e| format!("fetch body: {e}"))?;
    if text.len() as u64 > MAX_SUMS_BYTES {
        return Err("checksum response is too large".to_string());
    }
    Ok(text)
}

/// The checksum asset can lag a just-published release or hit a transient CDN
/// error; retry a few times instead of failing the whole update silently.
fn http_get_text_retry(url: &str, timeout_s: u64) -> Result<String, String> {
    let mut last = String::new();
    for attempt in 0..3 {
        match http_get_text(url, timeout_s) {
            Ok(text) => return Ok(text),
            Err(e) => {
                last = e;
                if attempt < 2 {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }
    Err(last)
}

fn http_get_bytes(
    url: &str,
    timeout_s: u64,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_s)))
        .max_redirects(0)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut resp = get_following(&agent, url)?;
    if resp.status().as_u16() != 200 {
        return Err(format!("fetch: HTTP {}", resp.status()));
    }
    let mut bytes = Vec::new();
    let mut reader = resp.body_mut().as_reader().take(limit + 1);
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| format!("fetch body: {e}"))?;
    if bytes.len() as u64 > limit {
        return Err("response is too large".to_string());
    }
    Ok(bytes)
}

fn http_get_bytes_retry(
    url: &str,
    timeout_s: u64,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for attempt in 0..3 {
        match http_get_bytes(url, timeout_s, limit) {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                last = e;
                if attempt < 2 {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }
    Err(last)
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

#[derive(Debug)]
struct DownloadPlan {
    name: String,
    url: String,
    sums_url: String,
    sig_url: String,
    api_digest: Option<String>,
    dir: PathBuf,
    dest: PathBuf,
}

/// Resolve everything that can fail offline (URL trust, digest shape, install
/// target, private directory) before touching the network.
fn download_plan(
    rel: &Release,
    arch: &str,
    target: Option<&Path>,
) -> Result<DownloadPlan, String> {
    let name = if !rel.gui.is_empty() {
        format!(
            "ProtonDrive-CLI-{}-GUI-{}-{arch}.AppImage",
            rel.cli, rel.gui
        )
    } else {
        // legacy release tagged v<cli> without a GUI suffix
        format!("ProtonDrive-CLI-{}-{arch}.AppImage", rel.cli)
    };
    let url = release_asset_url(rel, &name)
        .ok_or_else(|| format!("no build for {arch} in release {}", rel.tag))?;
    if !trusted_download_url(&url) {
        return Err(format!("release asset is not on a trusted host: {url}"));
    }
    let api_digest = match rel.asset_digest_raw(&name) {
        Some(raw) => Some(normalize_sha256(raw).ok_or_else(|| {
            format!("release asset digest for {name} is not a SHA-256")
        })?),
        None => None,
    };
    // Signature verification is mandatory: the manifest and its detached
    // signature are fetched for every update, even when GitHub supplies a
    // digest. The signed manifest is the authority.
    let sums_url = release_asset_url(rel, "SHA256SUMS")
        .ok_or_else(|| format!("no checksum manifest in release {}", rel.tag))?;
    if !trusted_download_url(&sums_url) {
        return Err(format!("checksum manifest is not on a trusted host: {sums_url}"));
    }
    let sig_url = release_asset_url(rel, "SHA256SUMS.sig")
        .ok_or_else(|| format!("no signature for the checksum manifest in {}", rel.tag))?;
    if !trusted_download_url(&sig_url) {
        return Err(format!("manifest signature is not on a trusted host: {sig_url}"));
    }
    let target = install_target(target, &name)
        .ok_or_else(|| "no writable install directory for the update".to_string())?;
    let base = target
        .parent()
        .ok_or_else(|| "install target has no parent directory".to_string())?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = base.join(format!(".proton-update-{}-{}", std::process::id(), nanos));
    let dest = dir.join(format!("proton-update-{name}"));
    Ok(DownloadPlan {
        name,
        url,
        sums_url,
        sig_url,
        api_digest,
        dir,
        dest,
    })
}

/// Download the AppImage for this machine, verify the signed checksum
/// manifest, then verify the AppImage against that manifest.
pub fn download_update(rel: &Release, arch: &str) -> Result<std::path::PathBuf, String> {
    let plan = download_plan(rel, arch, appimage_path().as_deref())?;

    // Signature first: a checksum is only trustworthy when the manifest that
    // carries it is signed by the project key. The GitHub API digest is a
    // consistency check, never the authority.
    let manifest = http_get_text_retry(&plan.sums_url, 60)
        .map_err(|e| format!("checksum fetch failed: {e}"))?;
    let signature = http_get_bytes_retry(&plan.sig_url, 60, 1024)
        .map_err(|e| format!("manifest signature fetch failed: {e}"))?;
    check_manifest_trust(manifest.as_bytes(), &signature)?;
    let want = sum_for(&manifest, &plan.name)
        .and_then(|sum| normalize_sha256(&sum))
        .ok_or_else(|| {
            format!(
                "no valid checksum entry for {} in the signed manifest",
                plan.name
            )
        })?;
    if let Some(api) = &plan.api_digest
        && api != &want
    {
        return Err("release digest does not match the signed manifest".to_string());
    }

    let _ = std::fs::remove_dir_all(&plan.dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&plan.dir)
            .map_err(|e| format!("cannot create the update directory: {e}"))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir(&plan.dir)
        .map_err(|e| format!("cannot create the update directory: {e}"))?;

    let result = (|| -> Result<(), String> {
        download(&plan.url, &plan.dest)?;
        match sha256_file(&plan.dest) {
            Ok(got) if got == want => {}
            Ok(_) => return Err("download corrupted (checksum mismatch)".to_string()),
            Err(e) => return Err(e),
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &plan.dest,
                std::fs::Permissions::from_mode(0o755),
            );
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&plan.dir);
        return Err(e);
    }
    Ok(plan.dest)
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

/// Validate a marker against the running AppImage. `old` must be exactly
/// `<app>.old`; a tampered marker can never point at another file.
fn validated_marker(
    mark: &Value,
    target: &Path,
) -> Option<(PathBuf, PathBuf)> {
    let old = PathBuf::from(mark.get("old")?.as_str()?);
    let new = PathBuf::from(mark.get("new")?.as_str()?);
    if !new.is_absolute() || !old.is_absolute() || new != target {
        return None;
    }
    if old != sidecar(target, ".old") {
        return None;
    }
    Some((old, new))
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
/// caller should relaunch it), or "" (nothing to do).
pub fn update_recovery_check(
    paths: &crate::paths::Paths,
) -> (&'static str, Option<std::path::PathBuf>) {
    update_recovery_check_at(&paths.update_mark_file, appimage_path().as_deref())
}

fn update_recovery_check_at(
    mark_file: &Path,
    target: Option<&Path>,
) -> (&'static str, Option<PathBuf>) {
    use serde_json::Value;
    // The APPIMAGE env is authoritative even when its file is missing - that
    // is exactly the interrupted-swap case recovery exists for.
    let Some(target) = target else {
        return ("", None);
    };
    let Ok(text) = std::fs::read_to_string(mark_file) else {
        return ("", None);
    };
    let Ok(mut mark) = serde_json::from_str::<Value>(&text) else {
        let _ = std::fs::remove_file(mark_file);
        return ("", None);
    };
    let Some((old, _)) = validated_marker(&mark, target) else {
        // A marker we did not write (or a corrupted one) never authorises a
        // rename or delete. Remove it; leave every file alone.
        let _ = std::fs::remove_file(mark_file);
        return ("", None);
    };
    let staged = sidecar(target, ".new");
    let attempted = mark
        .get("attempted")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if !target.is_file() {
        if attempted {
            if old.is_file() {
                let _ = std::fs::rename(&old, target);
                let _ = std::fs::remove_file(mark_file);
                return ("rolled-back", Some(target.to_path_buf()));
            }
        } else if staged.is_file() {
            let _ = std::fs::rename(&staged, target);
        } else if old.is_file() {
            let _ = std::fs::rename(&old, target);
        }
    }
    if target.is_file() {
        let _ = std::fs::remove_file(&staged); // interrupted download
    }

    if attempted {
        // Second launch without a health ack: the new version never ran.
        let _ = std::fs::remove_file(mark_file);
        if old.is_file() && std::fs::rename(&old, target).is_ok() {
            return ("rolled-back", Some(target.to_path_buf()));
        }
        return ("", None);
    }
    mark["attempted"] = Value::Bool(true);
    if std::fs::write(mark_file, serde_json::to_vec(&mark).unwrap_or_default()).is_err() {
        return ("", None);
    }
    ("pending", None)
}

/// The freshly updated version is up and its window was built: keep it.
/// Dropping the marker and the .old backup is the health acknowledgement.
pub fn update_confirm_health(paths: &crate::paths::Paths) {
    update_confirm_health_at(&paths.update_mark_file, appimage_path().as_deref());
}

fn update_confirm_health_at(mark_file: &Path, target: Option<&Path>) {
    use serde_json::Value;
    let Some(target) = target else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(mark_file) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        let _ = std::fs::remove_file(mark_file);
        return;
    };
    let Some((old, _)) = validated_marker(&value, target) else {
        let _ = std::fs::remove_file(mark_file);
        return;
    };
    let _ = std::fs::remove_file(mark_file);
    let _ = std::fs::remove_file(old);
}

/// Copy `src` into a brand-new `dst`, refusing a planted symlink.
fn copy_new(src: &Path, dst: &Path) -> std::io::Result<()> {
    let mut input = std::fs::File::open(src)?;
    #[cfg(unix)]
    let mut output = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(dst)?
    };
    #[cfg(not(unix))]
    let mut output = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(dst)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

/// Swap the running AppImage for the freshly downloaded one, then return
/// its path (the caller relaunches it once the single-instance lock is
/// free - a new copy started now would just report "Already running").
pub fn install_update_and_restart(
    dl: &std::path::Path,
    tag: &str,
) -> Result<std::path::PathBuf, String> {
    // `download_update` names the file `proton-update-<asset>`; the asset name
    // is what the new build is called when it cannot replace an existing file.
    let asset = dl
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("proton-update-"))
        .ok_or_else(|| "unexpected download name".to_string())?;
    let recorded = appimage_path();
    let target = install_target(recorded.as_deref(), asset)
        .ok_or_else(|| "not running as an AppImage".to_string())?;
    let in_place = target.is_file();
    let stage = sidecar(&target, ".new");
    let old = sidecar(&target, ".old");
    let private_dir = dl.parent().map(Path::to_path_buf);
    let _ = std::fs::remove_file(&stage);

    if std::fs::rename(dl, &stage).is_err() {
        copy_new(dl, &stage)
            .map_err(|e| format!("could not stage the update ({e})"))?;
        let _ = std::fs::remove_file(dl);
    }
    if let Some(dir) = private_dir
        && dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(".proton-update-"))
    {
        let _ = std::fs::remove_dir_all(dir);
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
    if !in_place {
        // Nothing is being replaced, so there is nothing to roll back: just
        // put the verified build where the launcher can start it.
        return std::fs::rename(&stage, &target)
            .map(|()| target)
            .map_err(|e| {
                let _ = std::fs::remove_file(&stage);
                format!("could not place the update ({e})")
            });
    }

    let state_dir = crate::paths::resolve().state_dir;
    if let Err(e) = proton_core::state_snapshot::create(&state_dir, "update-swap") {
        eprintln!("update snapshot: {e}");
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
    // Keep a copy of the running build before touching the target. A crash
    // between the backup and the rename leaves the original launchable, and
    // the rename itself atomically replaces the target (no missing-file
    // window).
    if let Err(e) = std::fs::copy(&target, &old) {
        let _ = std::fs::remove_file(&stage);
        return Err(format!("could not back up the AppImage ({e})"));
    }
    // The marker must exist before the target is replaced: if it cannot be
    // written, abort while the running file is still intact.
    if let Err(e) = std::fs::write(&mark_file, serde_json::to_vec(&mark).unwrap_or_default()) {
        let _ = std::fs::remove_file(&stage);
        let _ = std::fs::remove_file(&old);
        return Err(format!("could not record the update for recovery ({e})"));
    }
    if let Err(e) = std::fs::rename(&stage, &target) {
        let _ = std::fs::remove_file(&stage);
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::remove_file(&mark_file);
        return Err(format!("could not replace the AppImage ({e})"));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An update must still install when the running AppImage's file is gone
    /// (a previous update installed a different name); before 3.16.93 that
    /// failed with "not running as an AppImage".
    #[test]
    fn install_target_follows_a_vanished_appimage() {
        let dir = std::env::temp_dir().join(format!("pd-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let current = dir.join("ProtonDrive-CLI-0.9.0-GUI-3.13.0-x86_64.AppImage");
        std::fs::write(&current, b"x").unwrap();

        // An existing AppImage is replaced in place, keeping its name.
        assert_eq!(
            install_target(Some(&current), "new.AppImage"),
            Some(current.clone())
        );
        // A vanished one gets the new build beside it, under the asset name.
        let gone = dir.join("ProtonDrive-CLI-0.9.0-GUI-3.12.0-x86_64.AppImage");
        assert_eq!(
            install_target(Some(&gone), "new.AppImage"),
            Some(dir.join("new.AppImage"))
        );
        // Not an AppImage at all: nothing to install into.
        assert_eq!(install_target(None, "new.AppImage"), None);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_release_tag() {
        let (cli, gui) = parse_tag("v0.9.0-gui3.14.0").unwrap();
        assert_eq!(cli, "0.9.0");
        assert_eq!(gui, "3.14.0");
        assert!(parse_tag("v0.9.0").is_none());
        assert!(parse_tag("nonsense").is_none());
    }

    #[test]
    fn parses_latest_redirect_location() {
        assert_eq!(
            tag_from_release_location(
                "https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/tag/v0.9.0-gui3.20.22"
            ),
            Some("v0.9.0-gui3.20.22")
        );
        assert_eq!(
            tag_from_release_location("/zhafribs/Proton-Drive-Linux-Cli/releases/tag/v0.9.0-gui3.20.22?x=1#top"),
            Some("v0.9.0-gui3.20.22")
        );
        assert_eq!(tag_from_release_location("https://github.com/releases/latest"), None);
        assert_eq!(tag_from_release_location("https://github.com/releases/tag/"), None);
        assert_eq!(
            tag_from_release_location("https://github.com/releases/tag/a/b"),
            None
        );
    }

    #[test]
    fn a_redirect_release_synthesizes_canonical_asset_urls() {
        let mut r = rel("0.9.0", "3.20.22");
        r.assets.clear();
        let app = "ProtonDrive-CLI-0.9.0-GUI-3.20.22-x86_64.AppImage";
        assert_eq!(
            release_asset_url(&r, app).as_deref(),
            Some(
                "https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/download/v0.9.0-gui3.20.22/ProtonDrive-CLI-0.9.0-GUI-3.20.22-x86_64.AppImage"
            )
        );
        assert!(release_asset_url(&r, "SHA256SUMS")
            .unwrap()
            .ends_with("/v0.9.0-gui3.20.22/SHA256SUMS"));
        assert!(release_asset_url(&r, "SHA256SUMS.sig")
            .unwrap()
            .ends_with("/v0.9.0-gui3.20.22/SHA256SUMS.sig"));

        // The API's real asset list still wins when present.
        r.assets.push(Asset {
            name: "SHA256SUMS".into(),
            url: "https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/download/v1/SHA256SUMS"
                .into(),
            digest: None,
        });
        assert_eq!(
            release_asset_url(&r, "SHA256SUMS").as_deref(),
            Some("https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/download/v1/SHA256SUMS")
        );

        // A hostile tag never becomes a URL.
        r.assets.clear();
        r.tag = "../evil".into();
        assert_eq!(release_asset_url(&r, "SHA256SUMS"), None);
    }

    #[test]
    fn parses_latest_release_json() {
        let v = serde_json::json!({
            "tag_name": "v0.9.0-gui3.13.2",
            "published_at": "2026-10-18T09:30:00Z",
            "assets": [
                {"name": "ProtonDrive-CLI-0.9.0-GUI-3.13.2-x86_64.AppImage",
                 "browser_download_url": "https://x/a.AppImage", "size": 100,
                 "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
                {"name": "SHA256SUMS",
                 "browser_download_url": "https://x/SHA256SUMS", "size": 10,
                 "digest": null}
            ]
        });
        let r = parse_release(&v).unwrap();
        assert_eq!(r.tag, "v0.9.0-gui3.13.2");
        assert_eq!(r.gui, "3.13.2");
        assert_eq!(r.published, "2026-10-18");
        assert_eq!(r.assets.len(), 2);
        assert_eq!(r.asset_url("SHA256SUMS"), Some("https://x/SHA256SUMS"));
        assert_eq!(r.asset_url("nope"), None);
        assert_eq!(
            r.asset_digest_raw("ProtonDrive-CLI-0.9.0-GUI-3.13.2-x86_64.AppImage")
                .and_then(normalize_sha256),
            Some("a".repeat(64))
        );
        assert_eq!(r.asset_digest_raw("SHA256SUMS"), None);
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
        assert_eq!(
            sidecar(&t, ".new"),
            std::path::PathBuf::from("/opt/bin/Proton.AppImage.new")
        );
        assert_eq!(
            sidecar(&t, ".old"),
            std::path::PathBuf::from("/opt/bin/Proton.AppImage.old")
        );
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

    #[test]
    fn update_is_found_from_a_real_tag() {
        // The tag shapes that have shipped, including the suffixed ones that
        // used to parse as 3.16.0 and hide every update.
        let (cli, gui) = parse_tag("v0.9.0-gui3.99.0").unwrap();
        assert!(has_update("0.9.0", &rel(&cli, &gui)));
        let (cli, gui) = parse_tag("v0.9.0-gui3.99.0-rust").unwrap();
        assert!(has_update("0.9.0", &rel(&cli, &gui)));
        // a release equal to the installed build is not an update
        assert!(!has_update("0.9.0", &rel("0.9.0", crate::GUI_VERSION)));
    }

    #[test]
    fn download_hosts_are_exact() {
        assert!(trusted_download_url(
            "https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/download/v1/x.AppImage"
        ));
        assert!(trusted_download_url(
            "https://release-assets.githubusercontent.com/github-production-release-asset/x"
        ));
        assert!(trusted_download_url(
            "https://objects.githubusercontent.com/github-production-release-asset/x"
        ));
        for bad in [
            "http://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/download/v1/x",
            "https://github.com.evil/zhafribs/Proton-Drive-Linux-Cli/releases/download/v1/x",
            "https://github.com@evil/zhafribs/Proton-Drive-Linux-Cli/releases/download/v1/x",
            "https://github.com:444/zhafribs/Proton-Drive-Linux-Cli/releases/download/v1/x",
            "https://github.com/other/repo/releases/download/v1/x",
            "https://evil/github-production-release-asset/x",
            "",
        ] {
            assert!(!trusted_download_url(bad), "accepted {bad}");
        }
    }

    #[test]
    fn digests_are_64_lowercase_hex() {
        let valid = "A".repeat(64);
        assert_eq!(
            normalize_sha256(&format!("sha256:{valid}")),
            Some("a".repeat(64))
        );
        assert_eq!(normalize_sha256(&valid), Some("a".repeat(64)));
        assert_eq!(normalize_sha256(&"a".repeat(63)), None);
        assert_eq!(normalize_sha256(&"g".repeat(64)), None);
        assert_eq!(normalize_sha256(""), None);
    }

    #[test]
    fn download_plan_rejects_untrusted_hosts_and_bad_digests() {
        let target = std::env::temp_dir().join(format!("pd-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&target);
        std::fs::create_dir_all(&target).unwrap();
        let app = target.join("old.AppImage");
        std::fs::write(&app, b"x").unwrap();

        let mut r = rel("0.9.0", "3.20.9");
        r.assets.push(Asset {
            name: "ProtonDrive-CLI-0.9.0-GUI-3.20.9-x86_64.AppImage".into(),
            url: "https://evil/x.AppImage".into(),
            digest: Some("a".repeat(64)),
        });
        let e = download_plan(&r, "x86_64", Some(&app)).unwrap_err();
        assert!(e.contains("trusted host"), "{e}");

        r.assets[0].url =
            "https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/download/v1/x.AppImage"
                .into();
        r.assets[0].digest = Some("not-a-digest".into());
        let e = download_plan(&r, "x86_64", Some(&app)).unwrap_err();
        assert!(e.contains("not a SHA-256"), "{e}");
        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn install_targets_require_a_writable_target_and_plain_asset() {
        let dir = std::env::temp_dir().join(format!("pd-target-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let app = dir.join("app.AppImage");
        std::fs::write(&app, b"x").unwrap();
        assert_eq!(
            install_target_with(Some(&app), "new.AppImage", true),
            Some(app.clone())
        );
        assert_eq!(install_target_with(Some(&app), "new.AppImage", false), None);
        assert_eq!(
            install_target_with(Some(&app), "../evil", true),
            None
        );
        assert_eq!(
            install_target_with(Some(&dir.join("gone.AppImage")), "new.AppImage", true),
            Some(dir.join("new.AppImage"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_cap_rejects_one_byte_over() {
        let mut input = std::io::Cursor::new(vec![0u8; 10]);
        let mut output = Vec::new();
        assert_eq!(copy_capped(&mut input, &mut output, 10).unwrap(), 10);
        let mut input = std::io::Cursor::new(vec![0u8; 11]);
        let mut output = Vec::new();
        assert!(copy_capped(&mut input, &mut output, 10).is_err());
    }

    #[test]
    fn marker_rolls_back_only_its_own_sidecar() {
        let dir = std::env::temp_dir().join(format!("pd-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("app.AppImage");
        let old = sidecar(&target, ".old");
        let mark_file = dir.join("update-pending.json");
        std::fs::write(&target, b"new").unwrap();
        std::fs::write(&old, b"old").unwrap();
        std::fs::write(
            &mark_file,
            serde_json::json!({"old": old, "new": target, "attempted": false}).to_string(),
        )
        .unwrap();

        assert_eq!(
            update_recovery_check_at(&mark_file, Some(&target)).0,
            "pending"
        );
        let (state, relaunch) = update_recovery_check_at(&mark_file, Some(&target));
        assert_eq!(state, "rolled-back");
        assert_eq!(relaunch.as_deref(), Some(target.as_path()));
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(!old.exists());
        assert!(!mark_file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_marker_never_touches_another_file() {
        let dir = std::env::temp_dir().join(format!("pd-marker-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("app.AppImage");
        let victim = dir.join("victim.txt");
        let mark_file = dir.join("update-pending.json");
        std::fs::write(&target, b"new").unwrap();
        std::fs::write(&victim, b"keep").unwrap();
        std::fs::write(
            &mark_file,
            serde_json::json!({"old": victim, "new": target, "attempted": true}).to_string(),
        )
        .unwrap();

        assert_eq!(update_recovery_check_at(&mark_file, Some(&target)).0, "");
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert!(!mark_file.exists());

        std::fs::write(
            &mark_file,
            serde_json::json!({"old": sidecar(&target, ".old"), "new": target, "attempted": true})
                .to_string(),
        )
        .unwrap();
        update_confirm_health_at(&mark_file, Some(&target));
        assert!(victim.exists());
        assert!(!mark_file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signed_manifest_verifies_and_tampering_fails() {
        use ed25519_dalek::{Signer, SigningKey};
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let key_hex: String = signing
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let manifest = b"aaa  file.AppImage\n";
        let signature = signing.sign(manifest);
        assert!(verify_manifest_with(
            &key_hex,
            manifest,
            &signature.to_bytes()
        ));
        assert!(!verify_manifest_with(
            &key_hex,
            b"aaa  other.AppImage\n",
            &signature.to_bytes()
        ));
        assert!(!verify_manifest_with("not-hex", manifest, &signature.to_bytes()));
        assert!(!verify_manifest_signature(manifest, &[0u8; 63]));
        assert!(!verify_manifest_signature(manifest, &[0u8; 65]));

        // Rotation: a signature from any listed, non-revoked key is accepted;
        // revoking that key rejects it even though the bytes still verify.
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let other_hex: String = other
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let other_sig = other.sign(manifest);
        let keys = [key_hex.as_str(), other_hex.as_str()];
        assert!(verify_manifest_with_keys(
            &keys,
            &[],
            manifest,
            &other_sig.to_bytes()
        ));
        assert!(!verify_manifest_with_keys(
            &keys,
            &[other_hex.as_str()],
            manifest,
            &other_sig.to_bytes()
        ));
        assert!(!verify_manifest_with_keys(
            &[key_hex.as_str()],
            &[],
            manifest,
            &other_sig.to_bytes()
        ));
    }

    #[test]
    fn key_revocation_feed_lists_revoked_keys() {
        let feed = br#"{"version":1,"revoked_keys":["abc"]}"#;
        assert!(key_is_revoked("abc", feed).unwrap());
        assert!(!key_is_revoked("def", feed).unwrap());
        let empty = br#"{"version":1,"revoked_keys":[]}"#;
        assert!(!key_is_revoked("abc", empty).unwrap());
        assert!(key_is_revoked("abc", b"not json").is_err());
    }

    #[test]
    fn key_revocation_feed_requires_the_successor_signature() {
        use ed25519_dalek::{Signer, SigningKey};
        let successor = SigningKey::from_bytes(&[21u8; 32]);
        let successor_hex: String = successor
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let attacker = SigningKey::from_bytes(&[22u8; 32]);
        let feed = br#"{"version":1,"revoked_keys":["compromised"]}"#;
        let good = successor.sign(feed);
        assert!(
            check_manifest_trust_with("healthy", &successor_hex, feed, &good.to_bytes()).is_ok()
        );
        let revoked =
            check_manifest_trust_with("compromised", &successor_hex, feed, &good.to_bytes())
                .unwrap_err();
        assert!(revoked.contains("revoked key"));
        let bad = attacker.sign(feed);
        assert!(
            check_manifest_trust_with("healthy", &successor_hex, feed, &bad.to_bytes()).is_err()
        );
        let malformed = b"not json";
        let malformed_sig = successor.sign(malformed);
        assert!(check_manifest_trust_with(
            "healthy",
            &successor_hex,
            malformed,
            &malformed_sig.to_bytes()
        )
        .is_err());
    }

    #[test]
    fn revocation_feed_rejects_a_listed_tag() {
        let feed = br#"{"version":1,"revoked_tags":["v0.9.0-gui3.24.0"]}"#;
        assert!(is_revoked("v0.9.0-gui3.24.0", feed).unwrap());
        assert!(!is_revoked("v0.9.0-gui3.25.0", feed).unwrap());
        let empty = br#"{"version":1,"revoked_tags":[]}"#;
        assert!(!is_revoked("v0.9.0-gui3.24.0", empty).unwrap());
        assert!(is_revoked("x", b"not json").is_err());
    }

    #[test]
    fn project_key_verifies_an_openssl_signature() {
        // Signature over b"proton-update-self-test" made with the offline
        // Ed25519 key; proves the OpenSSL signer and the Rust verifier agree.
        let signature = hex_bytes::<64>(
            "4cfea0b3dcf5f08b2a4ee8ecdbd8244933903e266dfcf739688644b05624196b\
             f07f1f645f7070eea9fa741ce6a7d3b63eebde6ed3907d4b57183da4731af009",
        )
        .unwrap();
        assert!(verify_manifest_signature(
            b"proton-update-self-test",
            &signature
        ));
        assert!(!verify_manifest_signature(b"tampered", &signature));
    }

    #[test]
    fn download_plan_works_from_a_rate_limit_redirect() {
        let dir = std::env::temp_dir().join(format!("pd-plan-redirect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let app = dir.join("old.AppImage");
        std::fs::write(&app, b"x").unwrap();
        let mut r = rel("0.9.0", "3.20.22");
        r.assets.clear();
        let plan = download_plan(&r, "x86_64", Some(&app)).unwrap();
        assert!(plan.url.ends_with(
            "/v0.9.0-gui3.20.22/ProtonDrive-CLI-0.9.0-GUI-3.20.22-x86_64.AppImage"
        ));
        assert!(plan.sums_url.ends_with("/v0.9.0-gui3.20.22/SHA256SUMS"));
        assert!(plan.sig_url.ends_with("/v0.9.0-gui3.20.22/SHA256SUMS.sig"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_plan_requires_a_signed_manifest() {
        let dir = std::env::temp_dir().join(format!("pd-plan-signed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let app = dir.join("old.AppImage");
        std::fs::write(&app, b"x").unwrap();
        let name = "ProtonDrive-CLI-0.9.0-GUI-3.20.9-x86_64.AppImage";
        let mut r = rel("0.9.0", "3.20.9");
        r.assets = vec![
            Asset {
                name: name.to_string(),
                url: format!("https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/download/v1/{name}"),
                digest: None,
            },
            Asset {
                name: "SHA256SUMS".into(),
                url: "https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/download/v1/SHA256SUMS".into(),
                digest: None,
            },
            Asset {
                name: "SHA256SUMS.sig".into(),
                url: "https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/download/v1/SHA256SUMS.sig".into(),
                digest: None,
            },
        ];
        let plan = download_plan(&r, "x86_64", Some(&app)).unwrap();
        assert!(plan.sig_url.ends_with("SHA256SUMS.sig"));
        assert!(plan.sums_url.ends_with("SHA256SUMS"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "network: GitHub releases redirect"]
    fn live_redirect_finds_the_latest_tag() {
        let r = check_via_redirect().unwrap();
        assert!(parse_tag(&r.tag).is_some(), "unexpected tag {}", r.tag);
        assert!(!r.tag.contains('/'));
    }

    #[test]
    #[ignore = "network: GitHub release check"]
    fn live_check_still_works_when_the_api_is_rate_limited() {
        let root =
            std::env::temp_dir().join(format!("pd-live-check-{}", std::process::id()));
        let state = root.join("state");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&state).unwrap();
        let paths = Paths {
            conf_file: root.join("proton-sync.conf"),
            unit_file: root.join("proton-sync.service"),
            state_dir: state.clone(),
            health_file: state.join("health.json"),
            wake_file: state.join("wake"),
            lock_file: state.join("gui.lock"),
            snapshot_file: state.join("remote-snapshot.json"),
            cloud_cache_file: state.join("cloud-cache.json"),
            release_cache_file: state.join("release-cache.json"),
            update_mark_file: state.join("update-pending.json"),
            autostart_file: root.join("autostart.desktop"),
            menu_desktop_file: root.join("menu.desktop"),
            menu_icon_file: root.join("icon.svg"),
            prefs_file: root.join("gui.conf"),
            appimage: None,
        };
        let (rel, cached) = check(&paths).unwrap();
        assert!(!cached);
        assert!(parse_tag(&rel.tag).is_some(), "unexpected tag {}", rel.tag);
        // The fallback has no API asset list, but plans from canonical URLs.
        assert!(release_asset_url(&rel, "SHA256SUMS")
            .unwrap()
            .ends_with("SHA256SUMS"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
