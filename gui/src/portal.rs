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

enum DialogResult {
    Done(Vec<PathBuf>),
    Cancel,
    Unavailable,
}

fn run_dialog(exe: &str, args: Vec<String>) -> DialogResult {
    let Ok(out) = Command::new(exe).args(args).output() else {
        return DialogResult::Unavailable;
    };
    if !out.status.success() {
        return DialogResult::Cancel;
    }
    DialogResult::Done(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect(),
    )
}

/// A file-dialog outcome. `Cancelled` means the user closed the picker;
/// `Unavailable` means no portal, zenity or kdialog could answer, which the
/// caller must report instead of silently doing nothing.
pub enum PickResult<T> {
    Done(T),
    Cancelled,
    Unavailable,
}

fn dialog_result(result: DialogResult) -> PickResult<Vec<PathBuf>> {
    match result {
        DialogResult::Done(paths) if !paths.is_empty() => PickResult::Done(paths),
        DialogResult::Done(_) | DialogResult::Cancel => PickResult::Cancelled,
        DialogResult::Unavailable => PickResult::Unavailable,
    }
}

fn first_path(result: DialogResult) -> PickResult<PathBuf> {
    match result {
        DialogResult::Done(paths) => match paths.into_iter().next() {
            Some(path) => PickResult::Done(path),
            None => PickResult::Cancelled,
        },
        DialogResult::Cancel => PickResult::Cancelled,
        DialogResult::Unavailable => PickResult::Unavailable,
    }
}

/// Pick several files ("Upload files...").
pub fn open_files() -> PickResult<Vec<PathBuf>> {
    match portal(
        "OpenFile",
        "Upload files",
        vec![("multiple", Value::from(true))],
    ) {
        Portal::Done(p) if !p.is_empty() => PickResult::Done(p),
        Portal::Done(_) | Portal::Cancel => PickResult::Cancelled,
        Portal::Unavailable => {
            let zenity = run_dialog(
                "zenity",
                vec![
                    "--file-selection".into(),
                    "--multiple".into(),
                    "--separator=\n".into(),
                ],
            );
            match zenity {
                DialogResult::Unavailable => dialog_result(run_dialog(
                    "kdialog",
                    vec![
                        "--getopenfilename".into(),
                        ".".into(),
                        "--multiple".into(),
                        "--separate-output".into(),
                    ],
                )),
                other => dialog_result(other),
            }
        }
    }
}

/// Pick a folder ("Upload folder..." / "Save to folder").
pub fn open_dir(title: &str) -> PickResult<PathBuf> {
    match portal("OpenFile", title, vec![("directory", Value::from(true))]) {
        Portal::Done(p) => match p.into_iter().next() {
            Some(path) => PickResult::Done(path),
            None => PickResult::Cancelled,
        },
        Portal::Cancel => PickResult::Cancelled,
        Portal::Unavailable => {
            let zenity = run_dialog(
                "zenity",
                vec!["--file-selection".into(), "--directory".into()],
            );
            match zenity {
                DialogResult::Unavailable => first_path(run_dialog(
                    "kdialog",
                    vec!["--getexistingdirectory".into(), ".".into()],
                )),
                other => first_path(other),
            }
        }
    }
}

/// Save-as path for a single download.
pub fn save_file(name: &str) -> PickResult<PathBuf> {
    match portal(
        "SaveFile",
        "Save file",
        vec![("current_name", Value::from(name.to_owned()))],
    ) {
        Portal::Done(p) => match p.into_iter().next() {
            Some(path) => PickResult::Done(path),
            None => PickResult::Cancelled,
        },
        Portal::Cancel => PickResult::Cancelled,
        Portal::Unavailable => {
            let zenity = run_dialog(
                "zenity",
                vec![
                    "--file-selection".into(),
                    "--save".into(),
                    "--confirm-overwrite".into(),
                    format!("--filename={name}"),
                ],
            );
            match zenity {
                DialogResult::Unavailable => first_path(run_dialog(
                    "kdialog",
                    vec!["--getsavefilename".into(), format!("Downloads/{name}")],
                )),
                other => first_path(other),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_is_reported_separately_from_cancel() {
        assert!(matches!(
            dialog_result(DialogResult::Unavailable),
            PickResult::Unavailable
        ));
        assert!(matches!(
            dialog_result(DialogResult::Cancel),
            PickResult::Cancelled
        ));
        assert!(matches!(
            dialog_result(DialogResult::Done(Vec::new())),
            PickResult::Cancelled
        ));
        assert!(matches!(
            first_path(DialogResult::Unavailable),
            PickResult::Unavailable
        ));
        assert!(matches!(
            first_path(DialogResult::Done(Vec::new())),
            PickResult::Cancelled
        ));
    }

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
