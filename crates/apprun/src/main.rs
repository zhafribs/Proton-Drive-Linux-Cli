//! AppImage entry point - the Rust replacement for the old `src/AppRun`
//! shell script.
//!
//! Dispatch (`argv[1]`):
//!   (none) / `__gui` / `--smoke`  -> this project's Rust GUI
//!   `__sync-engine`               -> this project's Rust sync engine (`--loop`)
//!   anything else                 -> the unmodified upstream Proton CLI
//!
//! The payload always lives next to this binary inside the AppDir:
//!   `AppRun`, `usr/bin/proton-drive`, `usr/bin/proton-drive-gui`,
//!   `usr/bin/proton-engine`.

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Engine,
    Gui,
    Cli,
}

/// Dispatch on the raw OS string. A non-UTF-8 first argument is a CLI
/// operand, not "no argument": converting with `to_str()` used to send it to
/// the GUI, whose `std::env::args()` then aborts on the same bytes.
#[cfg(unix)]
fn route(first: Option<&std::ffi::OsStr>) -> Route {
    use std::ffi::OsStr;
    match first {
        Some(s) if s == OsStr::new("__sync-engine") => Route::Engine,
        Some(s)
            if s == OsStr::new("__gui")
                || s == OsStr::new("--smoke")
                || s == OsStr::new("--diagnose") =>
        {
            Route::Gui
        }
        None => Route::Gui,
        Some(s) if s.is_empty() => Route::Gui,
        Some(_) => Route::Cli,
    }
}

#[cfg(unix)]
fn exit_code(err: &std::io::Error) -> i32 {
    match err.kind() {
        std::io::ErrorKind::NotFound => 127,
        _ => 126, // found but not executable / bad format / busy
    }
}

#[cfg(unix)]
fn main() {
    use std::env;
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, exit};

    let root = match env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        Some(p) => p,
        None => {
            eprintln!("proton-drive: cannot resolve the AppImage payload");
            exit(127);
        }
    };
    let cli = root.join("usr/bin/proton-drive");
    let gui = root.join("usr/bin/proton-drive-gui");
    let engine = root.join("usr/bin/proton-engine");

    if !cli.is_file() {
        eprintln!("proton-drive: payload missing from AppImage ({})", cli.display());
        exit(127);
    }

    let mut args = env::args_os().skip(1);
    let first = args.next();
    let route = route(first.as_deref());

    match route {
        Route::Engine => {
            if !engine.is_file() {
                eprintln!("proton-engine: missing from AppImage ({})", engine.display());
                exit(127);
            }
            let err = Command::new(&engine)
                .arg("--loop")
                .args(args)
                .env("PROTON_DRIVE_BIN", &cli)
                .exec();
            eprintln!("proton-engine: {err}");
            exit(exit_code(&err));
        }
        Route::Gui => {
            if !gui.is_file() {
                eprintln!("GUI missing from AppImage ({})", gui.display());
                exit(127);
            }
            let mut cmd = Command::new(&gui);
            if let Some(a) = first.as_ref() {
                cmd.arg(a);
            }
            let err = cmd.args(args).exec();
            eprintln!("proton-drive-gui: {err}");
            exit(exit_code(&err));
        }
        Route::Cli => {
            let mut cmd = Command::new(&cli);
            if let Some(a) = first.as_ref() {
                cmd.arg(a);
            }
            let err = cmd.args(args).exec();
            eprintln!("proton-drive: {err}");
            exit(exit_code(&err));
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("proton-drive-apprun is Linux-only");
    std::process::exit(1);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn non_utf8_operands_go_to_the_cli() {
        let raw = OsStr::from_bytes(b"name-\xff");
        assert_eq!(route(Some(raw)), Route::Cli);
        assert_eq!(route(Some(OsStr::new("__sync-engine"))), Route::Engine);
        assert_eq!(route(Some(OsStr::new("__gui"))), Route::Gui);
        assert_eq!(route(Some(OsStr::new("--smoke"))), Route::Gui);
        assert_eq!(route(Some(OsStr::new("--diagnose"))), Route::Gui);
        assert_eq!(route(Some(OsStr::new(""))), Route::Gui);
        assert_eq!(route(None), Route::Gui);
    }
}
