# Full product audit and interface-size program

**Status:** Approved; G0 complete, G1 shipping 3.20.44  
**Date:** 2026-11-08  
**Scope:** every Rust file in the workspace, every UI surface, every CLI/engine
command, the release pipeline, and the legal notices.  
**Participants:** director, Design Council, Engineering Council, Trust Council,
and the audit secretariat.

## 1. Why this program exists

Two things are being asked for:

1. Users find the three interface-size choices too limited.
2. The whole product needs an evidence-based audit so commands, state files,
   layout, security, licensing and release steps do not overlap, fight for the
   same space, or leave dead code behind.

This is not a redesign and not a big-bang rewrite. It is a three-week audit
with small, signed releases. Each milestone ends with a council review and a
director gate.

## 2. Councils and responsibilities

| Council | Members | Owns |
|---|---|---|
| Director | You | Scope, priorities, risk acceptance, final sign-off |
| Design Council | UI/UX, accessibility, brand/legal reviewer | CLARITY tokens, layout/space, copy, contrast, keyboard/focus, legal wording |
| Engineering Council | GUI, engine/backend, architecture, release engineer | Command contracts, state ownership, dead code, performance, packaging |
| Trust Council | Security, legal counsel, QA | Threat model, dependency/license review, signing, CI gates, release evidence |
| Audit Secretariat | The agent team | Inventory, file-coverage ledger, finding register, evidence, scheduling |

Every finding has an owner, a severity, a file and line, evidence, a fix, and a
test. No finding closes without evidence.

## 3. Interface size (approved)

The old control offered three choices: `Compact 85%`, `Default 100%`,
`Large 110%`. The approved replacement:

- **Five quick presets:** Compact 85%, Small 93%, Default 100%, Large 110%,
  Extra large 120%.
- **Custom slider:** **50% to 200% in 5% steps**, with the percentage shown
  numerically and a **Reset to 100%** action.
- **Live preview**, still governed by Save/Discard; the value persists as a
  percent string (`ui_scale=112`), with legacy `compact`/`default`/`large`
  values migrated automatically.
- **Layout guard:** every page, tab, dialog and empty/error state is tested
  at 720×540, 1024×720, 1280×800, 1440×900 and 1920×1080, at 50%, 100%,
  150% and 200%. A layout that overlaps or clips at a supported combination
  is a P1 finding and must be fixed before the milestone closes.
- **Accessibility:** the slider is keyboard operable, has a visible value,
  and does not depend on colour or a narrow hit target.

Alternatives considered:

| Option | Trade-off | Verdict |
|---|---|---|
| Seven fixed presets only | Simple, but still a menu of guesses | Not enough |
| Slider only | Most flexible, but slow to find a known size | Too hidden |
| Presets + custom slider | Quick common choices plus full range | **Recommended** |

## 4. Audit method

The workspace has 60 Rust files and about 33,000 lines excluding build output.
"Every line" is enforced as follows:

1. **All lines through automation:** stable `cargo fmt --check`,
   `cargo test --workspace --all-targets`, release clippy with `-D warnings`,
   dependency/license scans, `cargo machete`/`cargo udeps` where installable,
   shell syntax checks and the release gate.
2. **Every file read by a council:** a coverage ledger lists each file,
   reviewer, date and finding IDs. No file is marked covered without a read.
3. **High-risk files double-read:** authentication/keyring, update signing,
   delete/trash paths, shell/terminal invocation, path handling, release
   workflow, and anything using `unsafe` or raw subprocesses get a second
   independent reviewer.
4. **Behaviour matrix:** every user action maps to the CLI command, engine
   action, state file and lock it touches. One action must have one owner.
5. **UI matrix:** every page, tab, dialog, menu and tray state at the sizes and
   scales above; screenshots are stored under `docs/audit/layout/`.

Evidence lives in:

- `docs/audit/COVERAGE.md` - file and UI-surface coverage.
- `docs/audit/FINDINGS.md` - one row per finding.
- `docs/audit/layout/` - screenshot matrix.
- `docs/audit/evidence/` - command output, threat model, license report.

## 5. Severity and release rules

| Severity | Meaning | Release rule |
|---|---|---|
| P0 | Data loss, credential exposure, broken update trust, security hole | Stop; fix before any release |
| P1 | User-visible breakage, incorrect state, layout overlap at a supported size | Fix in the current milestone |
| P2 | Consistency, accessibility, performance, maintainability | Schedule in the program |
| P3 | Nice-to-have or future idea | Backlog, director decides |

Releases are incremental: one signed release per milestone, not one huge
release at the end. A milestone cannot close with an open P0 or P1.

## 6. Timeline

| Gate | Dates | Deliverable | Release |
|---|---|---|---|
| G0 Baseline | Mon Nov 9 | Inventory, tooling, baseline, risk register; director approves scope | None |
| G1 Interface size | Tue Nov 10 - Wed Nov 11 | Presets + custom slider, migration, layout matrix, a11y | 3.20.44 |
| G2 UI/frontend audit | Thu Nov 12 - Tue Nov 17 | Every page/dialog/menu, space matrix, dead UI, motion, focus | 3.20.45 |
| G3 Backend/command contract | Wed Nov 18 - Fri Nov 20 | Action -> CLI -> engine -> state/lock matrix, races, dead paths | 3.20.46 |
| G4 Security/legal/release | Mon Nov 23 - Tue Nov 24 | Threat model, deps/licenses, signing, CI, legal notices | 3.20.47 |
| G5 Consolidation/final | Wed Nov 25 - Thu Nov 26 | Dead code/deps removed, docs/screenshots, full regression, sign-off | 3.20.48 |

A soft freeze applies during the program: bug fixes and audit findings only;
no unrelated feature work. Interface size is the one planned feature.

## 7. Milestone checklist

### G0 - Baseline (Nov 9)

- [x] Freeze `v0.9.0-gui3.20.43` and record its signed assets.
- [x] Inventory 60 Rust files (33,244 lines), UI surfaces, CLI commands,
      state files and locks in `COVERAGE.md`.
- [x] Baseline tests (269 passed, 4 ignored), release clippy, shell syntax and
      the release-gate fixtures; `cargo fmt --check` is not usable yet because
      the tree is not rustfmt-clean (F-004).
- [x] Document the missing audit tools and schedule them for G4 (F-007).
- [x] Create `COVERAGE.md`, `FINDINGS.md`, severity rules and owners.
- [x] Director approved the scope, dates and risk register.

### G1 - Interface size (Nov 10-11)

- [x] Director approved five presets and a custom 50-200% slider.
- [x] Replace the three-value enum with a percent-backed scale and migration.
- [x] Build the Appearance control: presets, slider, numeric value, reset.
- [x] Verify live preview and Save/Discard with keyboard navigation.
- [x] Run the layout matrix at 50/100/150/200% and at the supported window
      sizes; evidence in `docs/audit/layout/`.
- [x] Design, engineering and accessibility review; shipped `v0.9.0-gui3.20.44`
      with checksum, Ed25519 signature, smoke test and GLIBC_2.28 floor verified.

### G2 - UI/frontend audit (Nov 12-17)

- [ ] Every page: Home, Files, Photos, Activity, Settings tabs, About.
- [ ] Every dialog, context menu, tray menu and empty/error state.
- [ ] Space budget: no clipping, no text wrap inside chips/buttons, equal
      cards, scroll behaviour, focus rings, keyboard reachability.
- [ ] Contrast and motion audit; reduced-motion coverage.
- [ ] Dead widgets, duplicated labels and unused UI code removed.
- [ ] Fix all P0/P1; release 3.20.45.

### G3 - Backend/command contract (Nov 18-20)

- [ ] Matrix every GUI/menu/tray action -> CLI argv -> engine action.
- [ ] Map every state file, lock, cursor and service transition to one owner.
- [ ] Duplicate sync/refresh/start/stop commands and race windows removed.
- [ ] Error, retry and stale-session paths reviewed; no silent failure.
- [ ] Dead flags, unused modules and duplicated helpers removed.
- [ ] Fix all P0/P1; release 3.20.46.

### G4 - Security/legal/release (Nov 23-24)

- [ ] Threat model: keyring/session, subprocess quoting, path traversal,
      symlinks, deletions, update signature, CI secrets, permissions.
- [ ] Dependency advisory and license audit; notices regenerated.
- [ ] Release retention, signing and gate verified end to end.
- [ ] Legal wording reviewed by counsel; trademark/unofficial status clear.
- [ ] Fix all P0/P1; release 3.20.47.

### G5 - Consolidation and sign-off (Nov 25-26)

- [ ] Remove confirmed dead code, dead flags, dead dependencies and stale docs.
- [ ] Re-run full coverage ledger; no unread Rust files.
- [ ] Final layout screenshot matrix and README screenshots.
- [ ] Full regression: tests, clippy, gate, AppImage smoke, signatures.
- [ ] Director signs the final audit report; release 3.20.48.

## 8. Director decisions (approved 2026-11-08)

1. **Interface size:** approved five presets plus a custom **50-200%** slider.
2. **Audit depth:** approved the file-coverage model (every file read,
   high-risk files double-read) as the definition of "every line".
3. **Soft freeze:** approved bug-fix-only until G5 consolidation/sign-off.
4. **Release cadence:** approved one signed release per milestone.
5. **Director gates:** the G0-G5 dates in section 6 stand; a gate closes only
   when the director accepts its evidence.

## 9. What this plan does not promise

- It does not promise that no bug can ever exist; it promises evidence,
  coverage, named owners and a release gate.
- It does not add new features beyond interface size.
- It does not replace legal advice; counsel reviews the notices.
- It does not remove the AppImage/systemd/FUSE portability limits; those are
  product constraints documented in `docs/COMPATIBILITY.md`.
