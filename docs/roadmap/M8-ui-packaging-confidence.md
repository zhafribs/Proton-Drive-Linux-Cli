# Milestone M8 — UI & Packaging Confidence

**Status:** complete — released as `v0.9.0-gui3.29.0`
**Theme:** prove the interface and the packages, not just the sync engine.

## Scope

- **Automated page audit:** `ui::audit` renders every page, the main dialogs and
  the status variants through egui_kittest at 720×400 and 50/100/200%.
  AccessKit fails any input widget without a screen-reader name.
- **Keyboard-only test:** Tab focus must reach the Settings nav row and Enter
  must switch the page; no mouse event is sent.
- **Screenshot matrix:** 15 PNGs in `docs/audit/m8/`, refreshed by
  `scripts/ui-check.sh`; CI uploads them as an artifact.
- **aarch64 AppImage:** built from Proton's official linux-arm64 CLI with the
  same glibc 2.28 floor, pinned runtime and release signing.
- **Debian packages:** amd64 and arm64 `.deb` files install the tested
  AppImage, launcher, desktop entry and icons.
- **Release verification:** assets, notes and `scripts/verify-release.sh`
  cover both architectures and both packages.

## Not covered

- **Flatpak.** A sandboxed build needs a runtime and a `flatpak-builder` CI
  job; adding an unbuilt manifest would document, not prove, it.
- **Onboarding and localization.** The first-run welcome and sign-in flow
  already exist; a guided checklist and translated strings are M9 work.
- **Running the aarch64 binary under emulation.** CI verifies its ELF
  architecture, bundled contents and glibc floor, but does not execute it on
  an arm host.

## Release verification

- Tag `v0.9.0-gui3.29.0`; evidence in
  `docs/audit/evidence/release-3.29.0.txt`.
