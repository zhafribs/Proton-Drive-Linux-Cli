# Milestone M10 — First Run, Language and Flatpak

**Status:** in progress — target `v0.9.0-gui3.31.0`
**Theme:** make the first launch explain itself, give Malay speakers a real
main screen, and ship a sandboxed package that tells the truth about its
limits.

## Scope

- **Onboarding wizard:** welcome → sign in → folder, shown once, skippable.
- **Flatpak bundle:** x86_64 single-file bundle built from the tested
  AppImage, with a wrapper, desktop entry, icons, metainfo and sandbox
  permissions for home sync, keyring, network, tray and notifications.
- **Flatpak-aware UI:** the sign-in dialog shows the `flatpak run` command,
  updates are software-center owned, and host autostart/menu toggles are
  hidden.
- **Bahasa Melayu (beta):** language preference, first-run choice, and
  translation of setup, sidebar, status and Home. Other pages stay English
  and the UI says so.
- **Release plumbing:** Flatpak asset, SHA256SUMS, notes, CI build and
  install/smoke check, verifier awareness.

## Not covered

- **Full translation of every page.** `docs/LOCALIZATION.md` lists the exact
  covered surface.
- **aarch64 Flatpak and Flathub submission.** The runtime bundle is x86_64 and
  self-hosted.
- **Sandbox autostart.** Flatpak's background portal is not integrated; sync
  starts when the window opens.

## Release verification

- Tag `v0.9.0-gui3.31.0`; evidence in
  `docs/audit/evidence/release-3.31.0.txt`.
