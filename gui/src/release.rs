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
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    /// ("0.9.0", "3.14.0") from a `v0.9.0-gui3.14.0` tag.
    pub cli: String,
    pub gui: String,
    pub assets: Vec<Asset>,
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
        size: v.get("size").and_then(Value::as_u64).unwrap_or(0),
    })
}

fn parse_release(v: &Value) -> Option<Release> {
    let tag = v.get("tag_name")?.as_str()?.to_string();
    let (cli, gui) = parse_tag(&tag)?;
    let assets = v
        .get("assets")?
        .as_array()?
        .iter()
        .filter_map(asset_from)
        .collect();
    Some(Release { tag, cli, gui, assets })
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
        .timeout_global(Some(Duration::from_secs(600)))
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
        assert_eq!(r.assets.len(), 2);
        assert_eq!(r.assets[0].size, 100);
    }
}
