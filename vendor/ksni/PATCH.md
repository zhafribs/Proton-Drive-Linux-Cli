# Local ksni patch

`ksni` 0.3.6 is vendored here for one addition: `Handle::update_icon` /
`blocking::Handle::update_icon`. It refreshes the StatusNotifierItem
properties and icon but skips `Service::update_menu`, so an animated tray icon
does not rebuild the D-Bus menu on every frame.

Upstream: https://github.com/iovxw/ksni (Unlicense). Re-apply the three hunks
in `src/lib.rs`, `src/service.rs` and `src/blocking.rs` when upgrading.
