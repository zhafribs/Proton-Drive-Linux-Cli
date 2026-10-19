//! Locating the official Proton Drive CLI and the few things we ask it for:
//! the version banner, the signed-in identity, and the sign-in/sign-out
//! helpers. Transfers always stay the CLI's job - this app never touches
//! Proton's API itself.

use crate::exec;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

/// Same search order the PyQt app used: bundled-in-AppImage, repo checkout,
/// PATH, the manual-install location.
pub fn resolve_cli() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("APPDIR") {
        cands.push(PathBuf::from(dir).join("usr/bin/proton-drive"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("../usr/bin/proton-drive"));
            if let Ok(ver) = std::env::var("VERSION") {
                if let Ok(plat) = std::env::var("PLATFORM") {
                    cands.push(
                        dir.join(format!("../download/proton-drive-{ver}-{plat}")),
                    );
                }
            }
            cands.push(dir.join("proton-drive"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        cands.push(home.join(".local/bin/proton-drive"));
    }
    if let Some(p) = which("proton-drive") {
        cands.push(PathBuf::from(p));
    }
    cands.into_iter().find(|p| p.is_file())
}

fn which(name: &str) -> Option<String> {
    std::env::var("PATH").ok().and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned())
    })
}

/// First line of `proton-drive --version` ("" on failure).
pub fn cli_version(cli: &PathBuf) -> String {
    let out = exec::run(
        &cli.to_string_lossy(),
        &["--version"],
        Duration::from_secs(60),
    );
    match out {
        Ok(o) => o
            .stdout
            .lines()
            .chain(o.stderr.lines())
            .next()
            .unwrap_or("")
            .trim()
            .to_string(),
        Err(_) => String::new(),
    }
}

#[derive(Default, Clone, Debug)]
pub struct Identity {
    pub version: String,
    pub email: Option<String>,
    pub logged_in: bool,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct FsList {
    #[serde(default, rename = "ownedBy")]
    owned_by: Option<OwnedBy>,
}
#[derive(Deserialize)]
struct OwnedBy {
    #[serde(default)]
    email: Option<String>,
}

/// Version + who is signed in (`fs list` of the root returns `ownedBy`).
/// Independent of the engine: the update check must not wait on this.
pub fn fetch_identity(cli: &PathBuf) -> Identity {
    let mut out = Identity { version: "?".into(), ..Default::default() };
    let v = cli_version(cli);
    out.version = if v.is_empty() { "?".into() } else { v };
    match exec::run(
        &cli.to_string_lossy(),
        &["fs", "list", "/my-files", "--json"],
        Duration::from_secs(180),
    ) {
        Ok(o) if o.ok() => {
            // The root listing is an array; take the first entry with an email
            // (mirrors the PyQt loop over `json.loads(r.stdout)`).
            if let Ok(entries) = serde_json::from_str::<Vec<FsList>>(&o.stdout) {
                out.email = entries.into_iter().find_map(|e| e.owned_by.and_then(|ob| ob.email));
                out.logged_in = out.email.is_some();
            }
        }
        Ok(_) => out.logged_in = false, // not signed in yet
        Err(e) => out.error = Some(e),
    }
    out
}

pub fn logout(cli: &PathBuf) {
    let _ = exec::run(
        &cli.to_string_lossy(),
        &["auth", "logout"],
        Duration::from_secs(120),
    );
}

/// Proton's trash markers as the PyQt app filtered them: `.Trash-<id>`.
fn is_trash_marker(name: &str) -> bool {
    name.strip_prefix(".Trash-")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

/// Folders one level below `rel_path` (the selective-sync picker) as
/// `(name, rel)`, case-insensitive sort - `fetch_remote_folders` parity.
pub fn fetch_remote_folders(remote_root: &str, rel_path: &str) -> Vec<(String, String)> {
    let Some(cli) = resolve_cli() else { return Vec::new() };
    let rpath = if rel_path.is_empty() {
        remote_root.to_string()
    } else {
        format!("{remote_root}/{rel_path}")
    };
    let out = exec::run(
        &cli.to_string_lossy(),
        &["fs", "list", &rpath, "--json"],
        Duration::from_secs(120),
    );
    let Ok(o) = out else { return Vec::new() };
    if !o.ok() || o.stdout.trim().is_empty() {
        return Vec::new();
    }
    parse_folder_list(&o.stdout, rel_path)
}

fn parse_folder_list(json: &str, rel_path: &str) -> Vec<(String, String)> {
    let Ok(raw) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for e in &raw {
        if e.get("type").and_then(Value::as_str) != Some("folder") {
            continue;
        }
        let name_obj = e.get("name");
        let ok = name_obj
            .and_then(|n| n.get("ok"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let name = name_obj
            .and_then(|n| n.get("value"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !ok || name.is_empty() || is_trash_marker(name) {
            continue;
        }
        let rel = if rel_path.is_empty() {
            name.to_string()
        } else {
            format!("{rel_path}/{name}")
        };
        out.push((name.to_string(), rel));
    }
    out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    out
}

/// One row of the Cloud browser (also the on-disk cache shape - key names
/// match the PyQt-written cloud-cache.json so the two stay interchangeable).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RemoteEntry {
    pub name: String,
    pub rel: String,
    pub dir: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mtime: String,
    #[serde(default)]
    pub doc: bool,
}

/// The CLI's remote path for a relative one (PyQt `_cloud_rpath`): the root
/// plus each component with "/" escaped.
pub fn rpath(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        return root.to_string();
    }
    let mut parts: Vec<String> = vec![root.to_string()];
    for c in rel.split('/') {
        parts.push(c.replace('/', r"\/"));
    }
    parts.join("/")
}

fn parse_entries(json: &str, rel_path: &str, trash: bool) -> Result<Vec<RemoteEntry>, String> {
    let raw: Vec<Value> = serde_json::from_str(if json.trim().is_empty() { "[]" } else { json })
        .map_err(|e| format!("bad JSON from CLI: {e}"))?;
    let mut out: Vec<RemoteEntry> = Vec::new();
    for e in &raw {
        let Some(typ) = e.get("type").and_then(Value::as_str) else {
            continue; // pseudo-sections (/, /trash, ...)
        };
        if typ != "folder" && typ != "file" {
            continue;
        }
        let name_obj = e.get("name");
        let ok = name_obj
            .and_then(|n| n.get("ok"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let name = name_obj
            .and_then(|n| n.get("value"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !ok || name.is_empty() || is_trash_marker(name) {
            continue;
        }
        let is_dir = e.get("type").and_then(Value::as_str) == Some("folder");
        let media = e.get("mediaType").and_then(Value::as_str).unwrap_or("");
        let size = e
            .get("activeRevision")
            .and_then(|r| r.get("claimedSize"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let ts = if trash {
            e.get("trashTime").or(e.get("modificationTime"))
        } else {
            e.get("modificationTime")
        };
        let mtime: String = ts
            .and_then(Value::as_str)
            .unwrap_or("")
            .replace('T', " ")
            .chars()
            .take(16)
            .collect();
        let rel = if trash {
            name.to_string()
        } else if rel_path.is_empty() {
            name.to_string()
        } else {
            format!("{rel_path}/{name}")
        };
        out.push(RemoteEntry {
            name: name.to_string(),
            rel,
            dir: is_dir,
            size,
            mtime,
            doc: !is_dir && media.starts_with("application/vnd.proton"),
        });
    }
    // PyQt: sorted by (folders first, then name case-insensitively).
    out.sort_by(|a, b| {
        b.dir.cmp(&a.dir) // folders (dir=true) sort first
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

fn cli_err(o: &exec::Output) -> String {
    let src = if o.stderr.trim().is_empty() { &o.stdout } else { &o.stderr };
    let s: String = src.trim().chars().take(200).collect();
    if s.is_empty() { "list failed".to_string() } else { s }
}

/// Everything inside `rel` (folders first) for the Cloud browser - the PyQt
/// `fetch_remote_entries`: Err on CLI failure so the page can show an honest
/// status line.
pub fn fetch_remote_entries(remote_root: &str, rel: &str) -> Result<Vec<RemoteEntry>, String> {
    let Some(cli) = resolve_cli() else {
        return Err("proton-drive CLI not found".into())
    };
    let rpath = rpath(remote_root, rel);
    let o = exec::run(
        &cli.to_string_lossy(),
        &["fs", "list", &rpath, "--json"],
        Duration::from_secs(120),
    )?;
    if !o.ok() {
        return Err(cli_err(&o));
    }
    parse_entries(&o.stdout, rel, false)
}

/// Items sitting in the Proton trash (flat view) - PyQt `fetch_trash_entries`.
pub fn fetch_trash_entries() -> Result<Vec<RemoteEntry>, String> {
    let Some(cli) = resolve_cli() else {
        return Err("proton-drive CLI not found".into())
    };
    let o = exec::run(
        &cli.to_string_lossy(),
        &["fs", "list", "/trash", "--json"],
        Duration::from_secs(120),
    )?;
    if !o.ok() {
        return Err(cli_err(&o));
    }
    parse_entries(&o.stdout, "", true)
}

use serde_json::Value;

/// Terminal emulators, in the PyQt app's order -> (exe, extra args).
pub fn find_terminal() -> Option<(String, Vec<String>)> {
    for (name, args) in [
        ("konsole", vec!["-e"]),
        ("kitty", vec![]),
        ("alacritty", vec!["-e"]),
        ("xfce4-terminal", vec!["-e"]),
        ("gnome-terminal", vec!["--"]),
        ("xterm", vec!["-e"]),
    ] {
        if let Some(exe) = which(name) {
            return Some((exe, args.into_iter().map(str::to_string).collect()));
        }
    }
    None
}

/// The command string PyQt ran inside the terminal, unchanged:
/// `<cli> auth login; echo; echo "You can close this window now."`
pub fn login_script(cli: &PathBuf) -> String {
    let q = shell_quote(&cli.to_string_lossy());
    format!("{q} auth login; echo; echo \"You can close this window now.\"")
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_script_quotes_path_with_spaces() {
        let p = PathBuf::from("/opt/My Apps/proton-drive");
        let s = login_script(&p);
        assert!(s.starts_with("'/opt/My Apps/proton-drive' auth login"));
        assert!(s.ends_with("You can close this window now.\""));
    }

    #[test]
    fn folder_list_filters_and_sorts() {
        let json = r#"[
            {"type":"section","name":{"value":"/"}},
            {"type":"folder","name":{"value":"Zoo"}},
            {"type":"folder","name":{"value":"apple"}},
            {"type":"folder","name":{"value":".Trash-123"}},
            {"type":"folder","name":{"value":"bad","ok":false}},
            {"type":"file","name":{"value":"notes.md"}},
            {"type":"folder"}
        ]"#;
        let out = parse_folder_list(json, "");
        // sorted case-insensitively: apple < Zoo; trash/broken/file skipped
        let names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["apple", "Zoo"]);
        assert_eq!(out[0].1, "apple");

        let nested = parse_folder_list(json, "personel");
        assert_eq!(nested[0].1, "personel/apple");
    }

    #[test]
    fn trash_markers_only_numeric_ids() {
        assert!(is_trash_marker(".Trash-42"));
        assert!(!is_trash_marker(".Trash-x"));
        assert!(!is_trash_marker(".Trash-"));
        assert!(!is_trash_marker("notes.Trash-1"));
    }

    #[test]
    fn identity_scans_root_listing_array() {
        // Trimmed real `fs list /my-files --json` output: an ARRAY where the
        // email lives on each entry's ownedBy (a bare-object parse silently
        // blanked the identity card).
        let json = r#"[
{"uid":"a","name":{"ok":true,"value":"UUM-Bungalow"},"ownedBy":{"email":"zhafribs@gmail.com"},"type":"folder"},
{"uid":"b","name":{"ok":true,"value":"docs"},"type":"folder"}
]"#;
        let entries: Vec<FsList> = serde_json::from_str(json).unwrap();
        let email = entries.into_iter().find_map(|e| e.owned_by.and_then(|ob| ob.email));
        assert_eq!(email.as_deref(), Some("zhafribs@gmail.com"));
    }

    #[test]
    fn remote_entries_parse_filter_sort() {
        // Trimmed `fs list <folder> --json`: pseudo-sections (no "type"),
        // trash markers, not-ok names, Proton docs, claimedSize, mtime.
        let json = r#"[
            {"type":"section","name":{"value":"/"}},
            {"type":"folder","name":{"value":"Zoo"},"modificationTime":"2026-10-01T10:00:00.000Z"},
            {"type":"file","name":{"value":"a.txt"},"mediaType":"text/plain","activeRevision":{"claimedSize":2048},"modificationTime":"2026-10-02T11:30:00.000Z"},
            {"type":"file","name":{"value":"doc.pdf"},"mediaType":"application/vnd.proton.doc"},
            {"type":"folder","name":{"value":".Trash-9"}},
            {"type":"file","name":{"value":"bad","ok":false}}
        ]"#;
        let out = parse_entries(json, "docs", false).unwrap();
        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Zoo", "a.txt", "doc.pdf"]); // folders first
        assert_eq!(out[1].rel, "docs/a.txt");
        assert_eq!(out[1].size, 2048);
        assert_eq!(out[1].mtime, "2026-10-02 11:30"); // T->space, 16 chars
        assert!(!out[1].doc);
        assert!(out[2].doc); // application/vnd.proton.* is a document
        assert!(out[0].mtime.starts_with("2026-10-01"));
    }

    #[test]
    fn trash_entries_flat_with_trashtime() {
        let json = r#"[
            {"type":"file","name":{"value":"gone.txt"},"trashTime":"2026-10-05T08:00:00.000Z","modificationTime":"2026-09-01T08:00:00.000Z"},
            {"type":"section","name":{"value":"/trash"}}
        ]"#;
        let out = parse_entries(json, "", true).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rel, "gone.txt"); // flat: rel = name
        assert_eq!(out[0].mtime, "2026-10-05 08:00"); // trashTime wins
    }

    #[test]
    fn rpath_escapes_components() {
        assert_eq!(rpath("/my-files", ""), "/my-files");
        assert_eq!(rpath("/my-files", "docs/a"), "/my-files/docs/a");
        // parse-level sanity: empty stdout is an empty listing, not an error
        assert!(parse_entries("", "x", false).unwrap().is_empty());
    }
}
