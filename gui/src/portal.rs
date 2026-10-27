//! File dialogs through the XDG desktop portal
//! (org.freedesktop.portal.FileChooser) - the native pickers the PyQt app
//! got from QFileDialog, with no GTK/Qt dependency. Fallbacks when no
//! portal is running: zenity, then kdialog. Blocking: worker threads only.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::Value;

/// Percent-decode a `file://` URI into a filesystem path.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8(out).ok()?))
}

enum Portal {
    Done(Vec<PathBuf>),
    Cancel,
    Unavailable,
}

/// One portal round-trip: subscribe to the Request's Response signal first
/// (the request path is derivable from our unique bus name + handle_token,
/// so no response can be missed), then call the method and block for the
/// user's answer.
fn portal(
    member: &str,
    title: &str,
    extra: Vec<(&'static str, Value<'static>)>,
) -> Portal {
    let Ok(conn) = Connection::session() else { return Portal::Unavailable };
    static N: AtomicU64 = AtomicU64::new(0);
    let token = format!("pd_{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed));
    let Some(unique) = conn.unique_name() else { return Portal::Unavailable };
    let owner = unique.as_str().trim_start_matches(':').replace('.', "_");
    let req_path =
        format!("/org/freedesktop/portal/desktop/request/{owner}/{token}");

    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    for (k, v) in extra {
        options.insert(k, v);
    }

    let Ok(proxy) = Proxy::new(
        &conn,
        "org.freedesktop.portal.desktop",
        req_path.as_str(),
        "org.freedesktop.portal.Request",
    ) else {
        return Portal::Unavailable;
    };
    let Ok(mut signals) = proxy.receive_signal("Response") else {
        return Portal::Unavailable;
    };

    let msg = match conn.call_method(
        Some("org.freedesktop.portal.desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.portal.FileChooser"),
        member,
        &("", title, &options),
    ) {
        Ok(m) => m,
        Err(_) => return Portal::Unavailable,
    };
    let Ok(_handle) = msg.body().deserialize::<zbus::zvariant::OwnedObjectPath>() else {
        return Portal::Unavailable;
    };

    // Exactly one Response signal is expected for the request; block on it.
    let Some(resp) = signals.next() else {
        return Portal::Unavailable;
    };
    let body = resp.body();
    let Ok((code, results)) = body.deserialize::<(u32, HashMap<String, Value>)>() else {
        return Portal::Unavailable;
    };
    if code != 0 {
        return Portal::Cancel; // the user cancelled
    }
    let Some(uris) = results.get("uris") else { return Portal::Cancel };
    let inner = match uris {
        Value::Value(b) => &**b,
        other => other,
    };
    let Value::Array(arr) = inner else { return Portal::Cancel };
    let paths: Vec<PathBuf> = arr
        .iter()
        .filter_map(|v| match v {
            Value::Str(s) => uri_to_path(s.as_str()),
            _ => None,
        })
        .collect();
    if paths.is_empty() {
        Portal::Cancel
    } else {
        Portal::Done(paths)
    }
}

fn run_dialog(exe: &str, args: Vec<String>) -> Vec<PathBuf> {
    let Ok(out) = Command::new(exe).args(args).output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new(); // cancel / closed
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Pick several files ("Upload files..."). Empty = cancelled.
pub fn open_files() -> Vec<PathBuf> {
    match portal(
        "OpenFile",
        "Upload files",
        vec![("multiple", Value::from(true))],
    ) {
        Portal::Done(p) => p,
        Portal::Cancel => Vec::new(),
        Portal::Unavailable => run_dialog(
            "zenity",
            vec![
                "--file-selection".into(),
                "--multiple".into(),
                "--separator=\n".into(),
            ],
        )
        .if_empty_then(|| {
            run_dialog(
                "kdialog",
                vec!["--getopenfilename".into(), ".".into(), "--multiple".into(), "--separate-output".into()],
            )
        }),
    }
}

/// Pick a folder ("Upload folder..." / "Save to folder"). None = cancelled.
pub fn open_dir(title: &str) -> Option<PathBuf> {
    match portal("OpenFile", title, vec![("directory", Value::from(true))]) {
        Portal::Done(p) => p.into_iter().next(),
        Portal::Cancel => None,
        Portal::Unavailable => run_dialog("zenity", vec!["--file-selection".into(), "--directory".into()])
            .into_iter()
            .next()
            .or_else(|| {
                run_dialog("kdialog", vec!["--getexistingdirectory".into(), ".".into()])
                    .into_iter()
                    .next()
            }),
    }
}

/// Save-as path for a single download. None = cancelled.
pub fn save_file(name: &str) -> Option<PathBuf> {
    match portal(
        "SaveFile",
        "Save file",
        vec![("current_name", Value::from(name.to_owned()))],
    ) {
        Portal::Done(p) => p.into_iter().next(),
        Portal::Cancel => None,
        Portal::Unavailable => run_dialog(
            "zenity",
            vec!["--file-selection".into(), "--save".into(), "--confirm-overwrite".into(), format!("--filename={name}")],
        )
        .into_iter()
        .next()
        .or_else(|| {
            run_dialog(
                "kdialog",
                vec!["--getsavefilename".into(), format!("Downloads/{name}")],
            )
            .into_iter()
            .next()
        }),
    }
}

trait IfEmptyThen: Sized {
    fn if_empty_then(self, f: impl FnOnce() -> Self) -> Self;
}

impl<T> IfEmptyThen for Vec<T> {
    fn if_empty_then(self, f: impl FnOnce() -> Self) -> Self {
        if self.is_empty() { f() } else { self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_decoding() {
        assert_eq!(
            uri_to_path("file:///home/me/My%20File.txt").unwrap(),
            PathBuf::from("/home/me/My File.txt")
        );
        assert_eq!(
            uri_to_path("file:///a/b").unwrap(),
            PathBuf::from("/a/b")
        );
        assert!(uri_to_path("https://example.com/x").is_none());
        assert!(uri_to_path("file:///bad%zz").is_none());
    }
}
