# Local ksni patch

`ksni` 0.3.6 is vendored here for one safety fix: `Service::new` no longer
asserts on `zbus::Error::NameTaken`. A same-session peer can own the
predictable StatusNotifierItem bus name first; the old assert aborted the
whole GUI (release builds use `panic = "abort"`). The error now makes the tray
unavailable, which the GUI already handles.

The earlier `update_icon` animation patch was removed in the 2026-11-08 audit
because nothing calls it; the crate matches upstream except the NameTaken fix
and the dropped optional `futures-lite` feature dependency.

Upstream: https://github.com/iovxw/ksni (Unlicense). Re-apply the one hunk in
`src/service.rs` when upgrading.

Audit 2026-11-08: removed the optional `futures-lite` dependency. It was only
listed by the unused `async-io` feature and had no source reference; the
default `tokio` build is unchanged. `cargo-machete` is clean after this.

Audit 2026-11-08: removed the `assert_ne!(NameTaken)` in `Service::new`.
A same-session peer that owns the predictable
`org.kde.StatusNotifierItem-<pid>-<counter>` name made the assert abort the
whole GUI (release builds use `panic = "abort"`). `NameTaken` now returns
`Error::Dbus` and the tray degrades to unavailable, as it already does when
no host is present.
