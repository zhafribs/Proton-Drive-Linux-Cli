# Local ksni patch

`ksni` 0.3.6 is vendored here for one addition: `Handle::update_icon` /
`blocking::Handle::update_icon`. It refreshes the StatusNotifierItem
properties and icon but skips `Service::update_menu`, so an animated tray icon
does not rebuild the D-Bus menu on every frame.

Upstream: https://github.com/iovxw/ksni (Unlicense). Re-apply the three hunks
in `src/lib.rs`, `src/service.rs` and `src/blocking.rs` when upgrading.

Audit 2026-11-08: removed the optional `futures-lite` dependency. It was only
listed by the unused `async-io` feature and had no source reference; the
default `tokio` build is unchanged. `cargo-machete` is clean after this.
