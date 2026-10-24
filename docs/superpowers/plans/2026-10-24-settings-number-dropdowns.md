# Settings Number Dropdowns Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the four numeric settings rows in the GUI (Scan workers, Transfer concurrency, Parallel connections, Retries per folder) from `DragValue` spinners into `egui::ComboBox` dropdowns, then publish them inside the official 3.16.16 release.

**Architecture:** All four rows are built by one private helper, `number_combo`, in `gui/src/ui/settings.rs`. Its clamping, display, option-list, and adaptive-enabled logic live in pure functions so they are unit-testable; the two rows bound to `PROTON_SYNC_WORKERS` share the same range constant and the same `workers_enabled` predicate, so they can never disagree. No engine, config-key, or save-path changes.

**Tech Stack:** Rust, egui 0.37 (`gui/`), existing test module at the bottom of `gui/src/ui/settings.rs`.

**Spec:** `docs/superpowers/specs/2026-10-24-settings-number-dropdowns-design.md`

## Global Constraints

- `GUI_VERSION` in `gui/src/main.rs` stays `3.16.16` — it has not been released yet; do not bump it.
- Ranges, verbatim: workers/parallel connections `1..=16` (`PROTON_SYNC_WORKERS`), transfer concurrency `1..=10` (`PROTON_SYNC_TRANSFERS`), retries per folder `1..=10` (`PROTON_SYNC_RETRIES`).
- Option lists contain **every integer** in range, so any value already present in `~/.config/proton-sync.conf` still displays.
- The workers dropdowns are disabled while `app.draft.auto_workers` is true (engine uses `limit = max(1, WORKER_LIMIT if AUTO_WORKERS else SCAN_WORKERS)`), using the same `workers_enabled` predicate in both cards.
- Hover copy is preserved verbatim from today's `on_hover_text` strings; no tooltip for retries (today's spinner has none).
- No changes to `scripts/proton-sync.py`, config keys, defaults, or the save path (`save_settings` / `performance_values`).
- Work on branch `feat/fast-start-sync` in this worktree; do **not** delete the branch when merging (the worktree is attached to it).

## Review Focus

1. **A stale config value outside the range** (`PROTON_SYNC_WORKERS=99`, `PROTON_SYNC_RETRIES=0`) must clamp to the range bound instead of rendering a blank or panicking — covered by `clamps_values_to_range` in Task 1.
2. **Range drift** (someone later narrows transfers to 1–6 or workers to 1–8) breaks parity with the engine's `SCAN_WORKERS = max(1, min(16, ...))` and the 1–10 transfer cap — covered by `ranges_match_spec` in Task 1.
3. **Dropdown opens on the first entry instead of the saved value** (user's `12` workers shows `1`) — covered by `display_shows_saved_value` in Task 1; the helper uses `number_display` for `selected_text`.
4. **One of the two worker rows stays editable while adaptive scans is on**, letting a user set a ceiling the engine will ignore — covered by `workers_enabled_follows_adaptive` in Task 1 plus the Task 2 grep step showing both call sites pass `workers_enabled(app.draft.auto_workers)`.
5. **An integer missing from the options list** leaves a saved value with no matching entry to click — covered by `options_cover_every_integer` in Task 1.

---

### Task 1: Pure helpers and their tests

**Files:**
- Modify: `gui/src/ui/settings.rs` (constants + helper fns above `fn performance_card`, tests in the existing `mod tests` at the file bottom)
- Create: `docs/superpowers/specs/2026-10-24-settings-number-dropdowns-design.md` (already written; commit it with this task)

**Interfaces:**
- Consumes: `app.draft.workers: i32`, `app.draft.transfers: i32`, `app.draft.retries: i32`, `app.draft.auto_workers: bool` (all already on `Draft`).
- Produces (Task 2 relies on these exact names):

```rust
const WORKERS_RANGE: std::ops::RangeInclusive<i32> = 1..=16;
const TRANSFERS_RANGE: std::ops::RangeInclusive<i32> = 1..=10;
const RETRIES_RANGE: std::ops::RangeInclusive<i32> = 1..=10;

fn clamp_to_range(value: i32, range: std::ops::RangeInclusive<i32>) -> i32;
fn number_display(value: i32, range: std::ops::RangeInclusive<i32>) -> String;
fn number_options(range: std::ops::RangeInclusive<i32>) -> Vec<i32>;
fn workers_enabled(auto_workers: bool) -> bool;
```

- [x] **Step 1: Write the failing tests**

Append to the existing `mod tests` in `gui/src/ui/settings.rs`:

```rust
#[test]
fn clamps_values_to_range() {
    assert_eq!(clamp_to_range(99, 1..=16), 16);
    assert_eq!(clamp_to_range(0, 1..=16), 1);
    assert_eq!(clamp_to_range(12, 1..=16), 12);
    assert_eq!(clamp_to_range(99, 1..=10), 10);
    assert_eq!(clamp_to_range(0, 1..=10), 1);
}

#[test]
fn ranges_match_spec() {
    assert_eq!(*WORKERS_RANGE.start(), 1);
    assert_eq!(*WORKERS_RANGE.end(), 16);
    assert_eq!(*TRANSFERS_RANGE.start(), 1);
    assert_eq!(*TRANSFERS_RANGE.end(), 10);
    assert_eq!(*RETRIES_RANGE.start(), 1);
    assert_eq!(*RETRIES_RANGE.end(), 10);
}

#[test]
fn display_shows_saved_value() {
    assert_eq!(number_display(12, WORKERS_RANGE), "12");
    assert_eq!(number_display(4, TRANSFERS_RANGE), "4");
    assert_eq!(number_display(99, TRANSFERS_RANGE), "10");
    assert_eq!(number_display(0, RETRIES_RANGE), "1");
}

#[test]
fn options_cover_every_integer() {
    assert_eq!(number_options(WORKERS_RANGE), (1..=16).collect::<Vec<i32>>());
    assert_eq!(number_options(TRANSFERS_RANGE), (1..=10).collect::<Vec<i32>>());
    assert_eq!(number_options(RETRIES_RANGE), (1..=10).collect::<Vec<i32>>());
}

#[test]
fn workers_enabled_follows_adaptive() {
    assert!(!workers_enabled(true));
    assert!(workers_enabled(false));
}
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test --manifest-path gui/Cargo.toml settings::tests`
Expected: compilation error — `cannot find function clamp_to_range` / `cannot find value WORKERS_RANGE` (Rust tests fail at compile time when the symbol is missing).

- [x] **Step 3: Implement the constants and four functions**

Add just above `fn performance_card` in `gui/src/ui/settings.rs`. Bodies are fixed by the signatures and tests:

```rust
const WORKERS_RANGE: std::ops::RangeInclusive<i32> = 1..=16;
const TRANSFERS_RANGE: std::ops::RangeInclusive<i32> = 1..=10;
const RETRIES_RANGE: std::ops::RangeInclusive<i32> = 1..=10;

fn clamp_to_range(value: i32, range: std::ops::RangeInclusive<i32>) -> i32 {
    value.clamp(*range.start(), *range.end())
}

fn number_display(value: i32, range: std::ops::RangeInclusive<i32>) -> String {
    clamp_to_range(value, range).to_string()
}

fn number_options(range: std::ops::RangeInclusive<i32>) -> Vec<i32> {
    range.collect()
}

fn workers_enabled(auto_workers: bool) -> bool {
    !auto_workers
}
```

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path gui/Cargo.toml`
Expected: `test result: ok` — 49 existing tests plus the 5 new ones (54 total), 0 failures.

- [x] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-10-24-settings-number-dropdowns-design.md gui/src/ui/settings.rs
git commit -m "test: add settings dropdown helpers for numeric controls"
```

---

### Task 2: `number_combo` and the four rows

**Files:**
- Modify: `gui/src/ui/settings.rs` — new `number_combo` next to the Task 1 helpers; `performance_card` rows at ~lines 559–570; `fine_tuning_card` rows at ~lines 800–810.

**Interfaces:**
- Consumes: `WORKERS_RANGE`, `TRANSFERS_RANGE`, `RETRIES_RANGE`, `number_display`, `number_options`, `workers_enabled` (Task 1).
- Produces: `fn number_combo(ui: &mut egui::Ui, id_salt: &str, value: &mut i32, range: std::ops::RangeInclusive<i32>, enabled: bool, hover: &str)` — no later task.

- [x] **Step 1: Implement `number_combo`**

Add after the Task 1 helpers. Behaviour the signature and hover rule fix: clamp `*value` in place (so the saved draft equals what the user saw), `selected_text(number_display(*value, range.clone()))`, one `selectable_value` per entry from `number_options(range)`, the whole combo wrapped in `ui.add_enabled(enabled, ...)`, and `.on_hover_text(hover)` only when `hover` is non-empty (retries has no tooltip today).

- [x] **Step 2: Replace the Performance card rows**

Replace the current `ui.horizontal` block in `performance_card` with:

```rust
ui.horizontal(|ui| {
    ui.label("Scan workers");
    number_combo(
        ui,
        "scan-workers",
        &mut app.draft.workers,
        WORKERS_RANGE,
        workers_enabled(app.draft.auto_workers),
        "Raw ceiling used when adaptive scans are off.",
    );
    ui.add_space(24.0);
    ui.label("Transfer concurrency");
    number_combo(
        ui,
        "transfer-concurrency",
        &mut app.draft.transfers,
        TRANSFERS_RANGE,
        true,
        "Independent parent folders transferred at the same time.",
    );
});
```

- [x] **Step 3: Replace the Fine-tuning rows**

Replace the current `ui.horizontal` block in `fine_tuning_card` with:

```rust
ui.horizontal(|ui| {
    ui.label("Parallel connections");
    number_combo(
        ui,
        "parallel-connections",
        &mut app.draft.workers,
        WORKERS_RANGE,
        workers_enabled(app.draft.auto_workers),
        "Ceiling for parallel CLI calls. The engine uses fewer \
         automatically when memory is tight.",
    );
    ui.add_space(24.0);
    ui.label("Retries per folder");
    number_combo(ui, "retries-per-folder", &mut app.draft.retries, RETRIES_RANGE, true, "");
});
```

- [x] **Step 4: Confirm no spinners and one shared predicate remain**

Run: `grep -n 'DragValue\|workers_enabled' gui/src/ui/settings.rs`
Expected: no `DragValue` in `performance_card`/`fine_tuning_card` (the `interval`, `theme`, `ui_scale` combos are untouched `ComboBox`es); exactly two `workers_enabled(app.draft.auto_workers)` call sites, one per card.

- [x] **Step 5: Run the tests**

Run: `cargo test --manifest-path gui/Cargo.toml`
Expected: 54 passed, 0 failed.

- [x] **Step 6: Build and smoke the AppImage**

Run: `PATH=/tmp/opencode/bin:$PATH ./build.sh && ./dist/ProtonDrive-CLI-0.9.0-GUI-3.16.16-x86_64.AppImage --smoke`
Expected: build succeeds; `smoke ok: gui 3.16.16`. (The `/tmp/opencode/bin` shim supplies the pinned `jeepney` wheel because this machine's Python has no `pip`; it is outside the repo and changes nothing in the build.)
Manual check: open Settings — all four rows render as dropdowns showing the current values (`12`, `4`, `12`, `3`), with the workers row greyed while "Adapt parallel scans to available memory" is ticked.

- [x] **Step 7: Commit**

```bash
git add gui/src/ui/settings.rs
git commit -m "feat: use dropdowns for settings numeric controls"
```

---

### Task 3: Publish official 3.16.16

**Files:**
- None in the repo beyond pushing what Tasks 1–2 committed.

**Interfaces:**
- Consumes: the pushed `feat/fast-start-sync` branch; PR #1 (base `main`); CI job `.github/workflows/release.yml` (push to `main` with a new `GUI_VERSION` publishes tag `v0.9.0-gui3.16.16`).
- Produces: GitHub release `v0.9.0-gui3.16.16` with the x86_64 AppImage asset installed at `~/Downloads/AppImage/ProtonDrive-CLI-0.9.0-GUI-3.16.16-x86_64.AppImage`.

- [x] **Step 1: Push the branch**

Run: `git push origin feat/fast-start-sync`
Expected: `feat/fast-start-sync -> feat/fast-start-sync`; `gh pr view 1 --json mergeable` still `MERGEABLE`.

- [x] **Step 2: Merge PR #1**

Run: `gh pr merge 1 --merge --subject "feat: fast-start sync with background audit"`
Expected: PR state `MERGED`. Do **not** pass `--delete-branch` (the worktree is attached to the branch).

- [x] **Step 3: Wait for the release workflow**

Run: `gh run list --workflow release.yml --limit 3` then `gh run watch <id> --exit-status`
Expected: the `main` run succeeds and publishes tag `v0.9.0-gui3.16.16`.

- [x] **Step 4: Verify the release**

Run: `gh release view v0.9.0-gui3.16.16 --json tagName,assets`
Expected: one `ProtonDrive-CLI-0.9.0-GUI-3.16.16-x86_64.AppImage` asset.

- [x] **Step 5: Install the official artifact over the local build**

```bash
cd ~/Downloads/AppImage
gh release download v0.9.0-gui3.16.16 --clobber --pattern 'ProtonDrive-CLI-0.9.0-GUI-3.16.16-x86_64.AppImage'
sha256sum ProtonDrive-CLI-0.9.0-GUI-3.16.15-x86_64.AppImage \
          ProtonDrive-CLI-0.9.0-GUI-3.16.16-x86_64.AppImage > SHA256SUMS
./ProtonDrive-CLI-0.9.0-GUI-3.16.16-x86_64.AppImage --smoke
systemctl --user restart proton-sync.service
systemctl --user is-active proton-sync.service
```
Expected: smoke prints `gui 3.16.16`; service `active`; the service unit and desktop/autostart entries already point at this exact path (updated when the local build was installed), so only the file content changes. Relaunch the GUI so it runs the official binary.

- [x] **Step 6: Report**

Confirm to the user: official release URL, asset checksum, service active, GUI running the official build.
