# Settings numeric controls as dropdowns — short design (2026-10-24)

## Intent

The four numeric settings in the GUI are `DragValue` spinners (drag or click the
arrows). The user wants to pick from a list instead. This ships **inside the
official 3.16.16 release**: the change lands on `feat/fast-start-sync`, PR #1 is
merged, and the push to `main` triggers CI to publish `v0.9.0-gui3.16.16`.

## Scope — the four rows

| Card | Label | Config key | Range | Today |
| --- | --- | --- | --- | --- |
| Performance | Scan workers | `PROTON_SYNC_WORKERS` | 1–16 | `DragValue`, disabled when adaptive scans is on |
| Performance | Transfer concurrency | `PROTON_SYNC_TRANSFERS` | 1–10 | `DragValue` |
| Fine-tuning | Parallel connections | `PROTON_SYNC_WORKERS` | 1–16 | `DragValue` |
| Fine-tuning | Retries per folder | `PROTON_SYNC_RETRIES` | 1–10 | `DragValue` |

"Scan workers" and "Parallel connections" are the **same** setting written twice.

## Approaches

- **A — dropdowns in Fine-tuning only.** Smallest diff; leaves the same setting
  looking like a dropdown in one card and a spinner in the other.
- **B — shared workers control used by both cards.** One widget, consistent look;
  Fine-tuning only.
- **C — dropdowns everywhere (chosen).** All four rows become `egui::ComboBox`,
  with one shared helper for the workers setting so the two rows can never
  disagree. Transfer concurrency becomes a dropdown too, for uniformity across
  the card.

## Decisions

- **Value lists are every integer in range** (1…16, 1…10) so a value already in
  the user's config always displays; selected text is just the number.
- **One private helper in `settings.rs`** — `number_combo(ui, id_salt, &mut i32,
  range, enabled, hover)` — builds the `ComboBox`, clamps out-of-range values on
  render (stale config like `99` shows the top entry), and keeps the existing
  hover text. Only `settings.rs` needs it, so it does not go in `widgets.rs`.
- **Adaptive behaviour now greys out both workers rows:** Scan workers was
  already disabled while `auto_workers` is on, and Parallel connections in
  Fine-tuning is now disabled too — the engine ignores that value when
  `PROTON_SYNC_AUTO_WORKERS=1`, and both rows write `PROTON_SYNC_WORKERS`, so
  they must look the same. Built with `ui.add_enabled_ui` (egui 0.37's
  `ComboBox` has no `impl Widget`, so it cannot be passed to `ui.add_enabled`);
  the greyed rows keep their tooltip via `on_disabled_hover_text`.
- **No engine or config change:** keys, ranges, defaults (`WORKERS=12`,
  `TRANSFERS=4`, `RETRIES=3`), the save path, and `parse_transfers` stay as they
  are. Existing combo ids (`interval`, `theme`, `ui_scale`) stay untouched.
- **`GUI_VERSION` stays 3.16.16** — it has not been released yet, so no bump.

## Non-goals

- No change to sync semantics, transfers, retries, or adaptive scan logic.
- No new settings, no reorganising cards, no restyling.

## Verification

- `cargo test --manifest-path gui/Cargo.toml` (49 existing tests + new helper
  tests: in-range mapping, out-of-range clamping for workers/transfers/retries).
  (`rustfmt` is not installed in this toolchain, so no `cargo fmt --check`.)
- `./build.sh` → `./dist/*.AppImage --smoke` reports `gui 3.16.16`, and the four
  rows render as dropdowns with the current value selected.
- Merge PR #1 → CI publishes `v0.9.0-gui3.16.16`; confirm the release and its
  AppImage asset appear on GitHub.
