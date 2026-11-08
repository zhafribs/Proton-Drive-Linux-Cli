# Finding register

**Program:** Full product audit, 2026-11-08  
**Rule:** no milestone closes with an open P0 or P1. Every finding names a
file, the evidence, an owner, a fix and a verification step.

| Severity | Meaning | Release rule |
|---|---|---|
| P0 | Data loss, credential exposure, broken update trust, security hole | Stop; fix before any release |
| P1 | User-visible breakage, incorrect state, layout overlap at a supported size | Fix in the current milestone |
| P2 | Consistency, accessibility, performance, maintainability | Schedule in the program |
| P3 | Nice-to-have or future idea | Backlog; director decides |

## Open findings

| ID | Sev | Area | File | Evidence | Owner | Target | Status |
|---|---|---|---|---|---|---|---|
| F-004 | P2 | Tooling | workspace | `cargo fmt --check` is not part of CI and the current tree is not rustfmt-clean; adopting the default formatter would rewrite ~33k lines. | Engineering | G5 | Open: decide between one formatting release and a documented "no fmt gate" decision. |
| F-005 | P3 | Accessibility | `gui/src/ui/settings.rs` | 50% renders text at half size; it is the approved lower bound, but very small on a 1080p display. | Design | G5 | Open: collect feedback before changing the approved range. |
| F-006 | P3 | Layout | `gui/src/ui/settings.rs` | At the 720x400 logical minimum, the Appearance card needs vertical scrolling to reach the slider. No overlap or clipping occurs. | Design | G2 | Open: consider a more compact Appearance card if the audit finds horizontal room. |
| F-007 | P2 | Tooling | workspace | `cargo-audit`, `cargo-deny`, `cargo-machete` and `cargo-udeps` are not installed locally; CI has the license check and clippy only. | Trust | G4 | Open: install or record the supported substitute before the dependency audit. |
| F-008 | P2 | UX overlap | `gui/src/ui/settings.rs` | The same `PROTON_SYNC_WORKERS` value is edited by **Scan workers** (Performance) and **Parallel connections** (Fine-tuning), with different labels and help text. | Design/Engineering | G2 | Open: keep one control and link the other card to it. |
| F-009 | P2 | Security | `crates/core/src/conf.rs` | `conf::write` creates `proton-sync.conf` with the default umask (usually 0644). The **Extra environment variables** card accepts proxy credentials, so the file can be readable by other local users. | Trust | G4 | Open: create the file with mode 0600 on Unix and add a test. |
| F-010 | P3 | Consistency | `gui/src/ui/settings.rs` | The **Small** preset is 93%, but the slider steps in 5% increments, so 93% cannot be reached by dragging and the next drag jumps to 95%. | Design | G2 | Open: decide between 95% and a 1% slider step. |
| F-011 | P2 | Accessibility | `gui/src/ui/settings.rs` | In the selective-sync tree the folder name is a plain label; only the small checkbox glyph and the expand arrow are clickable. | Design | G2 | Open: make the row name toggle the checkbox and keep the arrow separate. |
| F-012 | P3 | Migration | `gui/src/prefs.rs` | Configs written before 3.20.44 stored the window size in egui points. At a non-100% scale those values are now read as physical units, so the first launch after upgrading can open a smaller window; the scale-aware floor and auto-grow correct it. | Engineering | G5 | Open: document the one-time change; no data loss. |

## Closed findings

| ID | Sev | Area | File | Finding | Fix | Verified |
|---|---|---|---|---|---|---|
| F-001 | P1 | Layout | `gui/src/ui/settings.rs` | At narrow widths the preset row and the slider shared one horizontal control row, pushing the percentage, Reset and part of the card past the right edge. | Presets and the slider now use separate rows inside a vertical control; the slider has a fixed 160 pt rail and a trailing fill. | 720x400 and 720x600 screenshots; Appearance card ends inside the page with the value and Reset visible. |
| F-002 | P1 | State | `gui/src/prefs.rs`, `gui/src/ui/mod.rs` | The window size was saved in egui points. Restarting at a larger interface scale restored a smaller physical window, and at 200% the layout fell below its minimum. | Window sizes are stored as physical pixels at the desktop scale (egui points × zoom). Loading clamps the saved size to a scale-aware floor. | `prefs::tests`; 200% launch from a 1024x720 seed grows to 1440x800. |
| F-003 | P1 | Layout | `gui/src/ui/mod.rs`, `gui/src/main.rs` | The viewport minimum was a fixed 720x540 physical points, so 200% left only 360 logical points and the three-column pages collided and clipped. | The minimum is 720x400 egui points, capped to the monitor; changing scale sends a scaled `MinInnerSize` and grows the window to the logical minimum. | 200% screenshots at 1440x800, 1440x900 and 1440x1000; all pages in `docs/audit/layout/`. |
| F-013 | P1 | Interaction | `gui/src/ui/settings.rs` | Dragging the new interface-size slider applied the zoom on every frame. The zoom changed the slider's geometry under the pointer, feeding back into the value; one short drag from 100% could jump to 50% or 200%. | The percentage still previews live, but the zoom is applied on release (or immediately for a click, wheel or arrow key). Added `scale_applies` and a unit test for the drag-stopped transition. | Virtual evdev pointer drag: 20 px right from 100% showed 120% while the UI stayed at 100% scale; on release the UI moved to 120%; a 40 px drag landed at 140%, not 200%. |

## Gate status

| Gate | Open P0 | Open P1 | Status |
|---|---:|---:|---|
| G0 Baseline | 0 | 0 | Evidence collection in progress |
| G1 Interface size | 0 | 0 | Shipped `v0.9.0-gui3.20.44`; patch `v0.9.0-gui3.20.45` fixes F-013; both verified |
| G2 UI/frontend | - | - | Not started |
| G3 Backend/contract | - | - | Not started |
| G4 Security/legal/release | - | - | Not started |
| G5 Consolidation | - | - | Not started |
