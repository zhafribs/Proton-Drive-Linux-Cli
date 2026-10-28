// Zig's linker (via cargo-zigbuild) still passes a deprecated `-Wl,-O1`,
// which rustc's `linker_messages` lint surfaces as a false-positive warning.
// Upstream: rust-cross/cargo-zigbuild#487, rust-lang/rust#158192.
#![allow(linker_messages)]

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
    let dispatch = first.as_deref().and_then(|s| s.to_str()).unwrap_or("");

    match dispatch {
        "__sync-engine" => {
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
            exit(127);
        }
        "__gui" | "--smoke" | "" => {
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
            exit(127);
        }
        _ => {
            let mut cmd = Command::new(&cli);
            if let Some(a) = first.as_ref() {
                cmd.arg(a);
            }
            let err = cmd.args(args).exec();
            eprintln!("proton-drive: {err}");
            exit(127);
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("proton-drive-apprun is Linux-only");
    std::process::exit(1);
}
