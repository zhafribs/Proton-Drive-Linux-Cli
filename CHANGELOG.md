## 3.23.0 — 2026-11-09

### Update trust

- **Multiple trusted update keys.** The manifest verifier accepts a signature
  from any key in `UPDATE_PUBLIC_KEYS`, so a successor key can be added in one
  release and used for the next.
- **Key revocation list.** `REVOKED_UPDATE_KEYS` rejects a key even when its
  signature is cryptographically valid. This protects users who install the
  revoking build.
- **Rotation procedure documented.** `docs/UPDATE-TRUST.md` covers successor
  key generation, the migration window, revocation and the remaining
  limitation (no remote revocation feed).

## 3.22.0 — 2026-11-09

### Portable sync

- **Works without a systemd user session.** When `systemctl --user` is not
  available, the GUI owns the engine: it starts it, keeps it under a monitor,
  restarts it after a crash, and stops it on pause/quit. The engine's own
  `sync.lock` still prevents a second copy.
- **Adoption after a GUI restart.** The supervisor writes `supervisor.pid`, so
  a new GUI instance recognises the running engine instead of spawning a
  duplicate; pause and restart still reach it.
- **Logs and diagnostics.** Fallback engine output goes to
  `~/.local/state/proton-sync/supervisor.log`; the Activity journal view reads
  that file when journalctl is unavailable.
- **Pause timers without systemd.** Pause 15 min / 1 h resumes from an
  in-process timer while the GUI is running. A paused marker still prevents a
  startup from resuming.
- **No misleading unit warnings.** In supervisor mode the app skips the
  systemd unit write and does not report an old unit as a failure.

The systemd unit remains the default and is unchanged when a user session is
available.

## 3.21.1 — 2026-11-09

- **Recovery refreshes when you open the page.** A deletion or conflict created
  while the app was running is visible without pressing Refresh.
- **Recovery check ignores conflicts you already chose to keep both.** The
  diagnostics count now matches the Conflict Center, and local-trash usage is
  included in the report.
- **Deleting a state snapshot asks for confirmation.** Restore points are
  metadata-only, but an accidental click no longer removes one.

## 3.21.0 — 2026-11-08

### Recovery Center

- **Local trash has a manifest and a view.** Every local deletion records its
  original path, time, size and kind; the Recovery page restores it without
  overwriting anything, or permanently deletes it after confirmation. Legacy
  entries written before the manifest are still found on disk.
- **Conflict Center.** Engine-created conflict copies are grouped with their
  canonical file. You can keep the current file (the copy moves to local
  trash), use the conflict copy (the current file is backed up to local trash
  first), keep both (the conflict is hidden until one changes), or open the
  folder.
- **Checksum-verified state snapshots.** A snapshot is taken before a root
  change, an approved mass delete and an update swap, and can be created
  manually. Restore verifies SHA-256, preserves the current state, refuses a
  snapshot from different sync roots, and wakes the engine to re-plan.
- **Recovery check and drill.** A read-only check reports sync history,
  snapshot, trash and conflict health in About → Diagnostics. The new engine
  test corrupts `state.json`, leaves an interrupted download stage behind,
  restores the snapshot and proves no local file is deleted.
- **Fingerprint and no-replace rules.** Conflict actions re-check both files;
  local trash restore uses hard-link/copy fallbacks that never replace a
  destination.

## 3.20.52 — 2026-11-08

- **Uploads use the folder that was open when the picker opened.** A late
  listing or navigation can no longer retarget the upload.
- **The selection bar keeps the status/error line visible** and adds a
  keyboard-reachable **More actions** menu (Open, Rename, Duplicate, Details,
  Move to...) plus **F2** rename. Ctrl+A is listed in the shortcut reference.
- **Native picker failures are reported.** A missing portal/zenity/kdialog is
  now an error toast instead of looking like the user cancelled.
- **Sidebar tree labels elide to the available width** and show the full name
  on hover, including at the 176 px pane minimum.
- **Changing a sync root warns before saving** that history is archived and
  cloud caches are cleared.
- **A second launch during quit waits for the lock** and starts normally if
  the previous instance has already exited.
- **50% now shows a readability warning** in Appearance.

## 3.20.51 — 2026-11-08

- **A post-download edit is no longer stamped as synced.** The fingerprint of
  the bytes actually placed is carried into `state_update`; a later rescan
  cannot hide an edit that happened between the download and the state write.
- **Pause and Resume are serialized.** One service-control lane and a desired
  state bit make the last click win, so a quick pause→resume cannot leave the
  engine in the opposite state.
- **Deletion approvals are SHA-256 and root-bound.** The one-shot approval now
  covers the exact side/path set and the local/remote roots; an approval for a
  different root or a stale path set fails closed.
- **The keyring lookup prefers the exact CLI item.** A single renamed
  auth-session is tolerated, but two ambiguous Proton sessions no longer make
  quota/event calls use the wrong account.

## 3.20.50 — 2026-11-08

- **Non-UTF-8 command operands reach the CLI.** AppRun dispatches on raw
  `OsStr` values, so a filename that is not valid Unicode is forwarded to the
  official CLI instead of being mistaken for “no argument” and aborting the
  GUI. Exec failures now use 126 (found but not executable) instead of 127.
- **A tray name collision cannot kill the app.** If a same-session peer owns
  the predictable StatusNotifierItem bus name first, the vendored ksni now
  returns `NameTaken` as a tray-unavailable error instead of asserting; the
  GUI keeps running without a tray.

## 3.20.49 — 2026-11-08

- **A download can no longer overwrite an edit made after the scan.** The
  planner's local fingerprint is re-checked immediately before the atomic
  place, with a backup/restore for the replace window. New destinations use
  no-replace hard links; local moves refuse an appeared destination.
- **Deletes are propagated only when the surviving side is unchanged.** A
  cloud edit paired with a local delete (or the reverse) is restored or
  uploaded instead of destroyed. Folder trashes count every descendant in the
  approval set, and the wipe guard now protects libraries of any size.
- **Unreadable or symlinked folders no longer look like deletions.** Local
  scans fail closed on read errors, links, special files and malformed remote
  listings. Changing the local or remote sync root archives the old state
  history before starting fresh.
- **The CLI lock fails closed.** Cloud mutations and downloads use the shared
  lock, and a busy lock returns an error instead of running unprotected during
  sign-in. Signing out clears the old account's state and cloud caches.
- **The fast-edit path is gone.** Event-driven updates use the normal scoped
  listing and planner, so an edit can never be stamped as synced by the
  shortcut.
- **Journal timestamps work on older systemd.** `+0800` offsets and precise
  fractions are accepted, conflict copy names keep their parentheses, and
  notifications match the leading verb rather than a file name.
- **Partial failures show as errors.** The engine writes `error` when a cycle
  had failed transfers, instead of claiming everything is up to date.
- **Trash rows have stable identities.** Trash entries use the node UID for
  selection/actions, and switching views clears stale rows and selections.
- **Download temp directories are private** (0700, unique), cleanup runs on
  every path, collisions get a new name, and folder downloads move
  recursively.
- **Smaller audit fixes:** theme detection prefers an explicit GTK theme on
  non-GNOME desktops, the tray's “Open Proton Drive” always shows the window,
  update recovery runs only after the single-instance lock, and starting in
  the tray no longer spends the first-close notification.

## 3.20.48 — 2026-11-08

- **Security dependencies updated.** `rustls-webpki` and `webpki-roots` are
  current, clearing the certificate-verification advisories found by
  `cargo audit`; the remaining report has no vulnerabilities.
- **Sync settings are private.** `proton-sync.conf` and the GUI preferences
  are written with mode `0600` on Unix, so an optional proxy credential in the
  environment block is not readable by other local users.
- **The release pipeline pins its actions.** Checkout, cache and artifact
  upload use commit SHAs instead of movable version tags.
- **Threat model and licence review are recorded** under `docs/audit/` for
  this milestone.

## 3.20.47 — 2026-11-08

- **A hung CLI command can no longer freeze sync.** Engine calls now have a
  deadline; a command that overruns it is killed and reported instead of
  holding the single CLI slot forever.
- **The CLI lock is genuinely re-entrant.** The first locked call sets the
  thread depth, so a nested CLI call cannot deadlock on the app-wide lock.
- **State files have one owner.** The engine resolves the state directory in
  one place, and the service unit pins `XDG_STATE_HOME` and `XDG_CONFIG_HOME`
  so a GUI launched with custom XDG dirs and the background engine agree on
  health, wake files and `cli.lock`.
- **Sign-in cannot lose the keyring session.** `auth login` never runs without
  the lock, and the identity probe waits for a slow sign-in instead of running
  unprotected after 60 seconds.
- **A transient CLI failure is not a sign-out.** Busy locks, locked keyrings
  and network errors no longer send the user to the welcome screen.
- **Pause always stops the engine.** A failed autoresume timer no longer
  leaves sync running while the UI says paused.
- **Service updates are atomic and executable-only.** The unit is written to a
  temp file and renamed, and only executable AppImages can become `ExecStart`.
- **Safer file handling.** The GUI and engine resolve the CLI the same way
  (including `PROTON_DRIVE_BIN`), the folder counter skips symlinks, and the
  photo-hold marker cannot follow a planted symlink.
- **App-menu files honor `XDG_DATA_HOME`.**

## 3.20.46 — 2026-11-08

- **Keyboard navigation reaches every page and control.** Sidebar rows and
  long pages scroll the focused control into view, Space works like Enter on
  nav rows and custom buttons, page titles and section labels are headings,
  and the account menu names the signed-in account.
- **Form controls are visible.** Checkbox, input and button borders now meet
  the 3:1 non-text contrast bar in both themes instead of using the decorative
  card hairline.
- **Right-click selects the row under the pointer.** Rename, Duplicate, Move
  and Trash now act on the file you right-clicked instead of the previous
  selection. Shift-range selection follows the visible sorted list, and
  **Select all** ignores files hidden by the search.
- **Selecting photos no longer hides the grid.** The hero and metric cards
  collapse while a selection is active, and the tile-size calculation can no
  longer bounce between one and two columns on narrow cards.
- **One owner for the worker limit.** The duplicate “Parallel connections”
  control is gone; **Scan workers** in Performance is the only control for
  `PROTON_SYNC_WORKERS`.
- **The Small preset is 95%.** Every preset now lands on the 5% slider grid,
  so choosing Small and then dragging does not snap to a different value.
- **Selective-sync rows are easier to toggle.** Clicking a folder name toggles
  its checkbox, not just the small glyph.
- **Saving while paused stays paused.** A settings save updates the service
  unit but no longer restarts an engine the user paused.

## 3.20.45 — 2026-11-08

- **The interface-size slider no longer jumps to the ends.** The percentage
  still updates while you drag, but the zoom is applied when you release the
  slider (or immediately for a click, wheel or arrow key). Applying zoom
  mid-drag moved the slider under the pointer and could throw the value to
  50% or 200% in one gesture.

## 3.20.44 — 2026-11-08

- **Interface size is now fully adjustable.** Appearance offers five presets
  (Compact 85%, Small 93%, Default 100%, Large 110%, Extra large 120%) and a
  custom **50%-200%** slider in 5% steps, with a live preview, a visible
  percentage, and **Reset** to 100%. The saved value is a percentage; old
  `compact`/`default`/`large` settings migrate automatically.
- **Large sizes stay usable.** Choosing a larger interface grows the window to
  the smallest layout that still fits every page, and the minimum window size
  follows the current scale. Window sizes are stored physically, so changing
  the scale and restarting no longer shrinks the window.
- **The Appearance controls no longer run off the card.** Presets and the
  slider sit on separate rows, so the percentage and Reset stay visible at
  narrow widths.

## 3.20.43 — 2026-11-08

- **Home shows your cloud storage.** A new **Cloud storage** card reports
  used and free space from your account quota, with a usage bar. The engine
  reads the quota through the CLI's existing session; the official CLI has no
  quota command. The third summary card returns to **Last check**, and Sync
  details add **Cloud walk** so the age of the remote view is explicit. The
  card fills in when the background sync service runs.

## 3.20.42 — 2026-11-07

- **About tags stay intact at the minimum window width.** Rust, egui,
  AppImage and MIT no longer break inside a word; the tag row wraps between
  tags instead.

## 3.20.41 — 2026-11-07

- **About shows a clean Proton CLI version.** Values such as
  `cli-drive@0.9.0` and `proton-drive 0.9.0` now display as `0.9.0`.
- **The What it does and Good to know cards match heights side by side.**
  Both cards use the taller measured body, so adding a line to either one
  grows the other instead of leaving mismatched panels. Narrow windows still
  stack them naturally.

## 3.20.40 — 2026-11-07

- **About is rebuilt as a product page.** Identity hero with live update
  status and project tags, three version/platform cards, What it does, Good
  to know, Updates, Details with copyable diagnostics, Credits, and a
  collapsible legal notice.
- **Dialogs take keyboard focus.** The safe/default action receives focus when
  a dialog opens, and the page behind it is disabled while it is up, so Tab
  stays inside the dialog instead of wandering through the background.

## 3.20.39 — 2026-11-07

- **The Photos action reads "Download all".** It still copies only photos
  missing from this computer; the shorter label keeps the title and three
  actions on one line at the minimum window width.

## 3.20.38 — 2026-11-07

- **Photos' hero copy lines up with the status pill.** The title and context
  now start at the same height as the pill instead of sitting lower in the
  orb's vertical centre. The 415.7 MB library value stays on one line at the
  minimum window width.

## 3.20.37 — 2026-11-07

- **Photos fits the smallest window.** The title and its three actions stay
  on one row, the status pill no longer collides with the hero title, and
  the three metric cards keep equal heights with large values (such as
  "415.7 MB") on a single line. The small info glyph was removed from the
  Photos header.

## 3.20.36 — 2026-11-07

- **Photos now matches the Home layout.** The library card is replaced by the
  same status hero as Home: orb, "Your photos are up to date", one context
  line, and the status pill. Three metric cards show In the cloud, Library
  size and On this computer. The separate "Two-way sync with Proton Photos."
  line is gone; technical paths and last-pass details stay in hover text.

## 3.20.35 — 2026-11-07

- **Photos has a proper library header.** The old three-line technical summary
  is now one card: photos in the cloud, library size, copies on this computer,
  sync status and the last update. Actions sit on the title row, with a
  stacked layout on narrow windows; the remote/local paths and last-pass
  details moved to hover.
- Signed-out, signing-in, paused, photo-sync-off, syncing and error states now
  get a status pill and one clear hint instead of a run-on sentence. Summary
  counts reset when the engine summary disappears, and the relative time
  updates live.

## 3.20.34 — 2026-11-07

- **Releases are no longer pruned.** The release workflow used to delete
  everything beyond the seven newest GitHub releases; it now keeps every
  published release and its tag. Older releases removed by the previous
  cleanup cannot be restored automatically.

## 3.20.33 — 2026-11-07

- **The first screen shows where files will sync.** While signed out, Home
  shows the configured folder (hover for the full path) with a Change link
  that opens the folder setting. No wizard and no extra confirmation step.
- **Keyboard shortcuts, with a list.** Ctrl+F jumps to Files and focuses the
  search box, Ctrl+Q quits, and F1 opens a grouped shortcut list (also linked
  from About) from any page. Delete, Enter and Space stay row-scoped to Files.

## 3.20.32 — 2026-11-07

- **“Animate status changes” now reaches the rest of the app.** Toasts,
  spinners and egui's widget transitions stop animating when it is off;
  `PROTON_UI_MOTION=0` now applies the same freeze to those surfaces, not just
  the Home orb.
- **Files is honest while signed out.** Upload and New folder are hidden,
  account actions are disabled, and an inline sign-in banner explains why.
  The remote listing is skipped (cached names stay visible), and an empty
  signed-out browser offers Sign in instead of claiming the folder is empty.
- **Accessibility pass on Files and the selective-sync tree.** Toolbar
  buttons, sort headers, breadcrumbs and folder-tree rows now expose
  accessible names; sort direction and mixed checkbox state are announced;
  headers, breadcrumbs and checkboxes work with Enter/Space and show a focus
  ring.
- **Contrast fixes.** Destructive ghost buttons use the AA danger colour and
  keep their surface fill when focused; activity icons and the download glyph
  now use theme-aware success, info and warning colours, with WCAG checks.
- The Photos page marks its local folder ready only after creation succeeds,
  and re-checks it after Save, rather than on every frame.

## 3.20.31 — 2026-11-06

- **Activity icons are drawn geometry now.** The up/down arrows, the removed
  × and the conflict mark are painted in a fixed 18 px box on the title line,
  so they no longer inherit a font baseline. The compact Home rows and the
  full Activity rows use the same alignment.
- **Home’s Recent activity stays summary-only.** Each row is icon + file name
  + time; the action and full path remain on the Activity page and in the
  hover text.
- **“Match my system” follows the desktop, not the wallpaper.** Detection now
  reads the XDG portal first, then desktop settings: gsettings/dconf for
  GNOME, Cinnamon and MATE (the active desktop's schema is asked first),
  `kdeglobals` for KDE/Plasma, and XFCE's XSettings. `GTK_THEME` and GTK's
  `settings.ini` follow as fallbacks. The Appearance tab shows
  **System theme: Dark/Light** while System is selected, with a hover note
  that a dark wallpaper is not a dark theme.
- **Theme and interface size preview live.** Selecting a theme or scale applies
  it immediately; **Save** persists it with the rest of the page, and
  **Discard** restores the saved baseline.
- **Sign-in terminal detection is broader.** `$TERMINAL` is honoured first;
  ptyxis, GNOME Console (`kgx`), foot, WezTerm, Terminator, urxvt and st are
  also detected. XFCE's terminal now receives `-x` so the sign-in command
  actually runs.
- AppImage portability is documented in `docs/COMPATIBILITY.md`: the release is
  glibc-2.28 pinned, x86_64, uses the standard type-2 runtime, and supports
  FUSE2/FUSE3. `APPIMAGE_EXTRACT_AND_RUN=1` runs the GUI without FUSE, but not
  the systemd-managed background sync. Systemd remains required for background
  sync; non-systemd init systems are the largest remaining gap.

## 3.20.30 — 2026-11-06

- **Home’s Recent activity is a summary again.** Each row shows the direction
  icon, file name and time only. The action/path subtitle stays on the full
  Activity page, where there is room for it.
- **The direction icons now line up with the title.** They are painted in a
  fixed 18 px box at the row’s center instead of being centered against a
  two-line text block.
- Hovering a row still reveals the full path.

## 3.20.29 — 2026-11-06

- **Project Clarity, phase 4: one Settings save model.** Every tab now stages
  changes and applies them only when you press **Save all changes**; **Discard
  all changes** restores the last saved state. There is no longer a mix of
  “save” tabs and “apply immediately” tabs.
- **Save covers everything.** Theme, interface size, motion, notifications,
  automatic update checks, window behavior, start-at-login, open-at-login,
  sync, photos, performance and advanced settings are all applied on Save.
- **Start-at-login is safer.** The background systemd probe no longer
  overwrites a value you have staged, and Save only changes start-at-login when
  you actually changed that checkbox. A theme or notification change no longer
  restarts syncing.
- **The Save bar is present on every tab**, and the tab hover text no longer
  claims some tabs apply immediately.
- Saving now writes all app-owned keys, including Photos and performance
  settings, instead of silently dropping them.
- Performance toggles now actually drive the engine: the fast remote snapshot
  key is the one the scanner reads, and adaptive workers cap the scan/transfer
  pool by available memory.
- Enabling automatic update checks arms the next check immediately; disabling
  it stops the schedule.
- Discard keeps the current window size and sidebar width; those are layout
  state, not settings.
- The folder card now uses the drawn folder icon instead of an emoji.
- No user data or sync-path changes in this phase; the performance toggles now
  reach the engine as described above.

## 3.20.28 — 2026-11-06

- **Project Clarity, phase 3: Photos.** The page header now carries the real
  actions — **Sync photos**, **Download missing**, **Open folder** — with the
  same hierarchy as Home and Files. The old action card is gone.
- **Selection has its own bar.** “N selected” with Download, Delete, Clear
  selection and Open folder; all busy/sync guards remain.
- **Syncing is visible.** While a photo pass is running, a compact banner says
  “Syncing photos…” or “Waiting for the file sync to finish…” instead of leaving
  a disabled button with no explanation.
- **Signed-out photo actions are honest.** Sync/Download/Delete are unavailable
  until you sign in on Home; the page no longer offers actions that can only
  fail.
- The grid, date grouping, thumbnails, skipped/failed cards, selection toggle,
  per-tile actions and photo deletion confirmation are unchanged.

## 3.20.27 — 2026-11-06

- **Project Clarity, phase 2: Files.** The browser now has a proper page
  header: **Files** or **Trash**, with Upload/New folder or Empty trash in the
  top-right, and a second row with refresh, back/forward, the breadcrumb, and a
  right-aligned search field.
- **The file list is quieter.** A 12 px card, the same solid surfaces as Home,
  and a drawn folder empty state instead of an emoji. Column sorting, context
  menus, drag/drop, keyboard selection and every existing action remain.
- **Selection now gets its own bar.** “N selected” shows the count, with
  Download, Move to trash, Restore, Delete permanently and Clear as
  appropriate. With nothing selected, the bar shows the sync status and item
  count.
- **Trash is first-class.** Its header offers Empty trash; selected trash rows
  can be restored or permanently deleted.
- No engine, sync, or data-path changes in this phase.

## 3.20.26 — 2026-11-06

- **Project Clarity, phase 1: the new app shell.** Neutral solid surfaces
  replace the translucent aurora/glass look. Proton violet is now the accent,
  not a background wash; cards have a crisp border and a quiet shadow.
- **The sidebar is simpler.** The wordmark gains a Linux/community subtitle,
  the two stat chips move to Home, and the footer shows the short sync status
  plus when it was last updated. The selected row is a clean pill.
- **Home is rebuilt.** A page header with the primary actions, a calm status
  hero, three honest metrics (files, on-disk size, last check), a compact
  recent-activity card, and a Sync details card. Recent activity has a
  **View all activity** link.
- **Activity is a real destination** in the sidebar, with the full moved-files
  list and technical log. Home shows a four-row preview.
- **Motion is now honest.** The decorative background animation is gone; the
  Appearance setting is renamed **Animate status changes** and controls the
  status disc instead of an invisible backdrop.
- Files, Photos, and Settings keep every control. Their pages inherit the new
  surface, border, and typography tokens; their deeper layout rebuilds are the
  next phases.
- The design constitution and prototype live in `docs/design/`.

## 3.20.25 — 2026-11-06

- **Removed the always-visible explainer paragraphs from Settings and Photos.**
  The page now opens with just the title, tabs, and controls instead of
  instructional copy.
- **The information is still there on hover.** Each Settings tab explains
  itself when you hover it; Save and Discard show their scope on hover; the
  Photos selection hint is now a small info icon next to the title.
- **The Animate the background hint is a hover tooltip** instead of a
  permanent line under the checkbox.
- No controls or features were removed.

## 3.20.24 — 2026-11-06

- **A skipped-file card now disappears as soon as the file is gone.** The
  Photos page checks that the named file still exists before showing it, and
  re-checks immediately when the folder changes. Deleting or renaming the file
  no longer leaves a stale “1 file skipped” card until the next hourly photo
  pass.
- **The skipped-file card has an Open folder action**, so the exact file is
  easy to find. The summary line likewise counts only skipped/failed files that
  still exist.
- The engine already clears the skip on the next cycle; a regression test now
  verifies that the cycle summary becomes empty once the file is deleted.

## 3.20.23 — 2026-11-06

- **A large photo import no longer stops at one bad file.** Files are checked
  by their actual bytes, not just their extension. A web page saved as
  `something.jpg` is skipped and named in Photos instead of being sent to the
  CLI, where it failed the whole upload batch.
- **Partial batches are reconciled and the rest keep going.** After a failed
  batch the engine checks which files actually reached the cloud, counts them,
  and retries only the genuinely missing ones. A failure in one chunk no longer
  blocks later chunks.
- **Retries cannot make duplicates.** Sync uploads now use Proton's `skip`
  conflict strategy instead of `rename`, so retrying a partially successful
  batch never creates `photo (1).jpg` copies.
- **Restored photos are no longer moved back to the trash.** The engine now
  records each local file's change time. An old state entry with no change time
  (or a file restored/edited since it was synced) is uploaded, never used to
  authorise a local delete. Normal cloud deletions still propagate for
  untouched files.
- **The Photos page tells the truth after a partial sync.** It shows the real
  uploaded count, names files that could not upload, and gives non-image files
  their own neutral “file skipped” card with the filename and reason. The
  message is no longer “Photo sync failed” for one odd file.
- **Failed local reads are safe.** A file that cannot be stat'ed is treated as
  a 0-byte placeholder for the plan instead of silently disappearing and
  causing its cloud copy to be trashed.

## 3.20.22 — 2026-11-06

- **Update checks keep working when GitHub rate-limits the API.** GitHub's
  unauthenticated API allows 60 requests per hour, and when the quota ran out
  the App & updates tab simply said it could not check. The app now falls back
  to GitHub's `releases/latest` redirect and the signed checksum manifest, so
  finding and downloading an update no longer depends on the API quota.
- **The update card tells you what happened.** A failed check now shows the
  real reason instead of a generic message, and the “up to date” line no longer
  leaves a blank date when the fallback supplied no publish date.
- **Installations stay signature-checked.** The fallback builds the same
  canonical asset URLs; every download is still verified against the Ed25519
  signature over `SHA256SUMS` before it can replace the running AppImage.

## 3.20.21 — 2026-11-06

- **Locally added photos now upload even while the photo timeline is held.**
  A previous safety hold paused the entire photo pass when the CLI returned an
  empty timeline, so a photo copied into `~/ProtonPhotos` never reached the
  cloud. The hold now keeps deletions and downloads paused, but uploads local
  photos: all of them when the cloud timeline is empty, or only names the
  engine has not seen when it is merely short. Successful uploads are
  remembered as pending so an empty timeline cannot upload duplicates every
  cycle.
- The hold summary now says how many photos were uploaded and that nothing was
  deleted, instead of only saying “Nothing was changed”.

## 3.20.20 — 2026-11-06

- **The Photos page is calmer and easier to scan.** The header shows the local
  folder as `~/ProtonPhotos` (full path on hover), the actions live in one
  card, and selecting photos swaps that card into a clear “N selected” mode
  instead of adding more buttons.
- **Selection is discoverable.** A quiet “Click a photo to select it for
  download or delete” line appears until the first selection.
- **Sync photos is honest.** It is visibly disabled while a sync or download is
  running, with a hover explanation, instead of looking clickable and doing
  nothing. Delete selected now respects the same busy/sync guard.
- **Errors get a proper callout** with a **Reload** action.
- Every previous feature remains: Sync photos, Download missing, Download
  selected, Delete selected, Clear selection, Open folder, per-tile
  Download/Re-download, status/attention, and the day-grouped grid.

## 3.20.19 — 2026-11-06

- **Settings is now six clear tabs instead of one long wall of cards.**
  **General**, **Sync**, **Notifications**, **Appearance**, **Advanced** and
  **App** each open with a one-line description. Every existing control is
  still there; Advanced is simply out of the way until you need it.
- **One readable column.** The old 1/2/3-column packing is replaced by a
  centered column capped at 820 px, so labels and controls no longer compete
  for attention across the whole window.
- **Save appears only where it matters.** Sync/performance tabs keep
  **Save changes** and **Discard**; tabs whose controls apply immediately say
  so instead of offering a button that does not do what it looks like.
- Tabs expose their selected state to AccessKit and are keyboard-activatable.

## 3.20.18 — 2026-11-06

- **Dialogs block the page underneath.** A dimmed, input-absorbing layer sits
  behind every dialog and dialog windows are foreground, so pointer clicks and
  drags cannot reach the file list or controls while a confirmation is open.
- **Toasts are dismissible and pause on hover.** Escape dismisses the front
  error when no dialog is using the key, and hovering an info toast stops its
  four-second timer until the pointer leaves.
- **The aurora can be switched off.** Settings → Appearance has an “Animate the
  background” toggle. Turning it off stops the continuous repaint and leaves
  the backdrop static, saving CPU/GPU wakeups.

## 3.20.17 — 2026-11-06

- **Settings Discard now restores the saved snapshot.** 3.20.16 re-read the
  preferences file, but every immediate toggle had already written that file,
  so Discard could not undo theme, scale, tray or notification changes. The app
  keeps a prefs baseline from startup/last Save, and Discard restores and
  persists it before reloading the sync draft.
- **Toast lifetimes start when a message reaches the screen.** A success
  queued behind a persistent error now gets its full four seconds after the
  error is dismissed, instead of expiring unseen. Errors are never evicted by
  the queue cap.
- **Toasts are real live regions.** Errors use `Alert` + Assertive; ordinary
  confirmations use `Status` + Polite, and restart failures are errors rather
  than expiring info.
- **The folder tree is keyboard-operable.** Enter opens, Space toggles,
  Left/Right collapse/expand, and AccessKit gets TreeItem level/expanded state.
  Active tree/nav text uses the darker accent token (AA on the light pill).

## 3.20.16 — 2026-11-06

- **Feedback no longer disappears before you can read it.** Messages are
  queued instead of replacing each other. Errors stay until **Dismiss** and
  are exposed to screen readers as an Alert; ordinary confirmations still fade
  after four seconds.
- **Settings Discard now really discards.** Theme, scale, tray and notification
  changes apply immediately, so Discard reloads those preferences from disk
  instead of only resetting the sync draft. The save bar says which changes are
  immediate.
- **The file tree is in the accessibility tree.** “My files” and every folder
  row expose their names and selected state; Enter still opens them.
- **Selected-file metadata is readable.** Size and modified time switch to the
  selection foreground on the accent fill instead of the low-contrast weak
  grey.

## 3.20.15 — 2026-11-06

- **Confirmation dialogs now block background file actions.** Double-click,
  context-menu actions and empty-area selection are ignored while a dialog is
  open, so ConfirmTrash and Move cannot act on a selection different from the
  one the dialog described. The same guard covers AccessKit and keyboard
  activation.
- **Photo-tile focus rings meet the 3:1 non-text bar** (full accent instead of
  60% opacity).

## 3.20.14 — 2026-11-06

- **Focused selected rows keep a visible ring.** The 3.20.13 accent ring sat on
  the accent selection fill (1.0:1 in light, 1.9:1 in dark); selected rows now
  use the white selection stroke (5.09:1), while unselected rows keep the
  accent ring.
- **Dialogs block row and tile activation**, including AccessKit Click, so a
  screen reader or keyboard cannot change the selection behind a confirmation
  dialog whose action reads the live selection.

## 3.20.13 — 2026-11-06

- **Keyboard focus is visible on file rows in both themes.** The focus ring was
  using the selection stroke, which is white in the light theme and disappeared
  on the near-white card; it now uses the accent colour.
- **Screen-reader activation works on file rows.** AccessKit's Click action now
  selects the row explicitly, while physical Space still toggles and Enter
  opens exactly once.
- **Photo tiles ignore keyboard activation while a dialog is open**, matching
  the file-list guard.

## 3.20.12 — 2026-11-06

- **A blocked account is one click from signing in.** Home shows **Sign in** (or
  **Sign in again** when the session expired) directly under the status, instead
  of hiding the only sign-in action inside the account menu.
- **No more vanishing window on tray-less desktops.** Close-to-tray and
  start-in-tray only hide the window when a StatusNotifier tray actually
  exists; otherwise closing quits normally. The background notification no
  longer tells you to right-click a tray icon that is not there.
- **Delete no longer fires while typing.** The file-list Delete shortcut is
  ignored while a text field owns the keyboard, and while a dialog is open, so
  pressing Delete in search cannot propose trashing the previous selection.
- **Photos errors are retryable.** A failed timeline fetch shows its error with
  a **Retry** button instead of leaving the page stuck until an unrelated sync.
- **Keyboard and screen-reader support for the file list and photo grid.**
  Rows and tiles expose their name and selected state to AccessKit; Enter opens
  a row, Space toggles selection, and clicking a row moves keyboard focus to it.
  Sidebar navigation rows now expose their selected state too.
- **Primary CTA contrast.** The cyan cap over the violet buttons dropped white
  label contrast to about 3.8:1; the flat violet button passes WCAG AA.

## 3.20.11 — 2026-11-05

- **The shipped licence file now covers the signing crates.** Regenerating it
  added `ed25519-dalek`, `curve25519-dalek`, `fiat-crypto` and their
  dependencies. CI now regenerates the file and fails if the committed copy is
  stale, so a future dependency cannot ship without its licence text.
- **The signature gate is regression-tested.** The fixture suite now has 12
  cases, including a missing signature, a tampered signature, the wrong signing
  key, and embedded-key drift.
- **Key rotation is executable.** `SIGN_RELEASE_VERIFY_KEY` and
  `ALLOW_KEY_ROTATION=1` support a transitional release signed by the old key
  while embedding the new one; the exact commands are in the signed-updates
  design doc.
- The release notes now state that pre-3.20.10 versions do not verify
  signatures.

## 3.20.10 — 2026-11-05

- **Updates are signed.** Every release now publishes `SHA256SUMS.sig`, an
  Ed25519 signature over the checksum manifest. The updater downloads and
  verifies it against the project public key before trusting any digest, and
  refuses an unsigned or tampered manifest. GitHub's own asset digest is only a
  consistency check; the signed manifest is the authority.
- **The release gate verifies the signature too.** CI signs with the
  `UPDATE_SIGNING_KEY` secret, then `check-release.sh` verifies the signature
  against both the tracked public key and the key embedded in the updater, so
  the two cannot drift apart.
- **Key policy:** the private key is stored in the GitHub secret and an offline
  backup at `~/Backup/proton-update-signing-key.pem` (0600). Losing it means
  future builds cannot sign updates; rotate by shipping a new key in a release
  signed with the old one.
- Signed updates are verified offline; the updater still checks the AppImage
  size/checksum and the staged build's `--smoke` start before installing.

## 3.20.9 — 2026-11-05

- **Releases are verified before publishing.** CI now runs the full test suite
  and clippy in a read-only job before the release job can start. A separate
  release gate checks the changelog section, AppDir metadata, GUI version,
  engine binary, `--smoke` output and SHA-256 sums. A stale version or an empty
  changelog fails instead of publishing a maintenance stub.
- **The sync engine now has the same 2.28 glibc floor as the GUI.** It was
  built natively on the CI runner, so the 3.20.8 engine actually required
  GLIBC_2.39 while the README promised 2.28. GUI, engine and AppRun are now
  cross-built and checked with `readelf`; a newer symbol version fails the
  build.
- **Build tools are pinned.** appimagetool 1.9.1 and the AppImage type2
  runtime (20251108) are checksum-verified, and Zig 0.15.2 is checksum-verified
  when fetched and version-checked on a cache hit. The Rust toolchain is pinned
  in `rust-toolchain.toml`.
- **The updater is harder to redirect.** Downloads go into a private 0700
  directory with `create_new`, only this repository's GitHub release host and
  GitHub's asset CDN are accepted (including each redirect hop), the body is
  capped, malformed digests are rejected, and a private writable install
  directory is required before any download starts.
- **Update recovery cannot delete another file.** The marker is validated in
  both the recovery and health-confirmation paths: `old` must be exactly
  `<app>.old` and `new` must equal the running AppImage. A failed swap restores
  the old build immediately.
- **Metadata is correct.** The desktop/metainfo ids move from `zhafrisbs` to
  `zhafribs`, developer and homepage point at this project, and the AppStream
  and desktop versions are filled from `GUI_VERSION` at build time.

## 3.20.8 — 2026-11-05

- **A conflict can no longer overwrite your local edit.** The remote revision
  is downloaded into staging, verified, and saved under the conflict name
  first; only then is the untouched local file uploaded. If the copy fails,
  nothing is uploaded and the path is retried next cycle. Conflict names are
  unique, so two conflicts in the same second cannot overwrite each other.
- **Interrupted downloads are verified before they replace anything.**
  Downloads land in `.proton-sync-staging` beside the destination, are checked
  against the cloud size and fsynced, then renamed into place. A short or
  missing file leaves the existing local file untouched and is retried. This
  covers both normal sync and the first-sync path.
- **A bad Photos timeline can no longer delete your library.** A non-array
  response is reported as an error and nothing moves; unreadable entries, an
  empty timeline, or a response that loses more than half of the known library
  hold the whole photo cycle and report the reason instead of trashing local
  files 25 at a time. Local photo removals go to `.proton-photos-trash` inside
  the photo folder (same filesystem), never `unlink`.
- **A corrupt `state.json` stops the cycle.** Sync now fails closed when the
  history cannot be read, instead of treating every path as new and disabling
  the wipe and mass-delete guards. State writes fsync the file and best-effort
  the directory before finishing.
- **Cloud paths are validated before any local join.** Names containing `/`,
  `..`, NUL or an absolute component are refused at the listing boundary and
  again before execution, so a malformed snapshot cannot write or delete
  outside the sync root.

## 3.20.7 — 2026-11-05

- **Palette PNG photos preview again.** Indexed-color PNGs (4-bit or 8-bit,
  with or without transparency), grayscale and 16-bit PNGs are expanded to
  RGBA8 before decoding. The files were downloaded intact; only the thumbnail
  decoder rejected their color type, which is why small wallpapers showed no
  preview.
- **Preview wording stops blaming the file.** A format the app cannot decode
  says "No preview for this format"; a genuine decode failure says "Preview
  failed". The tile tooltip states that the downloaded file is unchanged.

## 3.20.6 — 2026-11-05

- **The updater verifies with GitHub's own asset digest.** The release API now
  carries a `sha256:` digest for each asset, so the updater checks that first
  instead of depending on a second download of `SHA256SUMS`. `SHA256SUMS` stays
  as the fallback for older releases, with three retries before the build is
  downloaded. A genuine failure now says whether the checksum could not be
  fetched or the entry was missing, instead of the misleading "checksum entry
  not found" after a full download.

## 3.20.5 — 2026-11-05

- **Photos has its own Sync button.** "Sync photos" checks Proton Photos
  without going through Home, and the photo pass now runs even when the file
  pass fails. Home's button and the tray still run a full sync.
- **Interrupted photo downloads repair themselves.** A 0-byte local file left
  by a failed download is re-fetched into a temp folder, checked against the
  cloud size, then atomically swapped in. The file is never deleted first, a
  real file that appears during the download is left alone, and repeated
  failures back off instead of retrying every wake. A failed repair also vetoes
  trashing the cloud copy if the empty placeholder is later removed.
- **Honest photo tiles.** Each tile shows its size and says whether it is
  missing, incomplete, a differing copy, or a duplicate name. The header reads
  "Last photo sync" with an attention count instead of claiming "In sync", and
  download failures are reported instead of leaving the old success line.
- **Large libraries stay responsive.** Only tiles on screen are measured and
  decoded; thumbnails use a least-recently-used cache instead of clearing the
  whole cache when it fills.
- Known limits: two cloud photos with the same name still cannot be told apart
  locally (the app reports the duplicate and transfers neither), and a
  non-zero partial file stays a "both copies kept" conflict rather than being
  overwritten.

## 3.20.4 — 2026-11-05

- **Large deletions now wait for your approval.** A batch affecting 25 or more
  paths, at least half of a small library, or an entire folder with five or
  more synced files is held for one cycle. Nothing is deleted until you review
  the exact path set on Home and approve that batch once.
- **An empty side is still never approvable.** If one side looks completely
  empty while the other has files, the wipe guard holds everything and tells
  you to restore the folder or fix the sync path.
- **Approval is one-shot and path-bound.** The engine matches the reviewed
  counts and a stable digest of both path sets, consumes the approval on read,
  and rejects stale or mismatched approvals. Disabling deletion propagation
  still disables all deletions.
- **Local trash keeps its paths.** `~/.local/state/proton-sync/trash` now keeps
  the relative folder layout and suffixes collisions instead of overwriting
  same-named files. A failed move is reported and retried instead of being
  logged as success.
- **Honest status.** Home and the tray show "Deletions paused" with the counts
  and top-level names; the old "You're up to date" claim is gone while a hold is
  active.
- Known residual: a malformed Proton listing that drops fewer than the above
  thresholds can still be treated as an ordinary deletion. That scan-validation
  hardening is the next safety patch.

## 3.20.3 — 2026-11-05

- **The tile grid fills its card.** Columns are computed from the card width and
  the thumbnails are sized so a full row spans it exactly - no leftover strip -
  with tiles kept between 110 and 220 px. The size depends only on the width, so
  a card with three photos uses the same thumbnails as one with twenty instead of
  stretching to fill.

## 3.20.2 — 2026-11-05

- **Settings → Photos is just the controls.** The two explanatory paragraphs are
  gone: the card is now the sync switch, the "Propagate deletions" switch and the
  photo folder field.

## 3.20.1 — 2026-11-05

- **Each day card has its date back on top.** The label sits above its card and
  only becomes the pinned one once it scrolls off the top, so the date is never
  drawn twice.
- The date is gone from **under every photo** - that is what was noise. It is
  still in the day header and in a tile's hover tooltip.

## 3.20.0 — 2026-11-05

- **Photo deletions now travel both ways.** Delete a file in `~/ProtonPhotos` and
  the cloud photo goes to the Proton trash; trash a photo in any Proton app and
  the local copy is removed. It only touches photos the engine has *seen in
  sync* (recorded in `photos-state.json`), so a first run can never delete
  anything.
- **Guards.** Nothing is deleted when the photo folder looks like an unmounted
  disk (it is empty but photos are known to be synced), deletions obey a new
  **Settings → Photos → Propagate deletions** switch
  (`PROTON_SYNC_PHOTOS_DELETES=0`), and at most 25 deletions run per cycle - a
  mass delete crawls instead of wiping the library.
- The page reports deletions too: "last pass 1 up, 0 down, 1 trashed, 1 removed".
- Fixed on the way: a photo uploaded in one cycle was not recorded as synced
  (the timeline snapshot predated the upload), so the next cycle downloaded it
  back instead of propagating a local delete.

## 3.19.1 — 2026-11-05

- **The day label hands over later.** It now stays put while any part of that
  day's card is still on screen, and is only replaced once the card has scrolled
  completely out of view. (The first cut compared content coordinates with
  screen coordinates, so it switched far too early.)
- **No date inside the day card.** The label is only the pinned one at the top.
- **Thumbnails retry after a file comes back.** A photo deleted locally and
  restored kept a stale "failed to decode" mark, so it never showed a picture;
  failures now remember the file's stamp and retry when the file changes.
- **Settings → Photos.** New card with "Sync my photos with this computer" and
  the photo folder (blank = `~/ProtonPhotos`); saving it restarts the sync
  service, so it applies immediately.
- **No more flicker.** "Loading the photo timeline..." now appears only on the
  first load; background re-reads are silent, and they only happen when the
  folder or the cloud set actually changed - the summary file is rewritten every
  engine cycle, which used to trigger a reload each time.

## 3.19.0 — 2026-11-05

- **Delete photos.** Select tiles and press Delete: they go to the Proton photos
  trash (`fs trash /photos/<uid>`, uid paths because timeline names repeat) and
  their copies in the photo folder are removed, so the next sync cannot bring
  them back. Confirmation dialog; restorable from Photos in any Proton app.
- **Photos are grouped by day, and the day stays on screen.** The grid is split
  per capture day; its label pins to the top of the list as you scroll and hands
  over when the next day arrives.
- **No Reload button.** The page re-reads the timeline itself when the photo
  folder or the engine's sync summary changes, and the engine now watches that
  folder, so dropping a file in uploads it on that cycle instead of waiting for
  the next scheduled check.

## 3.18.1 — 2026-11-05

- **One sync button.** Photos no longer has its own "Sync now": Home's (and the
  tray's) Sync now drives the engine, which syncs files and photos in the same
  cycle. The page keeps a **Reload list** button, which only re-reads the photo
  list from Proton Drive.
- **The Photos header stays put.** Title, sync state and actions are pinned;
  only the tile grid scrolls, so nothing you might click scrolls away.
- The list also re-reads itself when you come back to the page after more than a
  minute, so a sync started from Home shows up without pressing anything.
- Measured while investigating "local photos did not upload": the upload path is
  fine (`photo upload -c rename` uploaded a test image in 300 B), the engine on
  that machine was still 3.17.0, which has no photo sync at all.

## 3.18.0 — 2026-11-05

- **Photos show as pictures.** The timeline is a month-grouped tile grid with
  real thumbnails: PNG through the bundled `png` decoder and JPEG through
  `zune-jpeg` (pure Rust, no C). Decoding runs off the UI thread, each photo
  becomes one texture downscaled to 256px, and the cache is capped so a big
  library cannot fill the GPU. Formats with no decoder (HEIC) keep the
  placeholder tile and a Download button.
- **Photos sync both ways.** The engine uploads new files found in
  `~/ProtonPhotos` and downloads photos that are only in the cloud (batched
  `photo upload` / `photo download -c rename`), and the page reports the result:
  "In sync · 15 in the cloud, 15 on disk". Additive on purpose: nothing is
  deleted or overwritten, and a name whose size differs on both sides is
  reported once and left alone. `PROTON_SYNC_PHOTOS=0` turns it off;
  `PROTON_SYNC_PHOTOS_LOCAL` moves the folder; it only runs once that folder
  exists (the page creates it).
- Sizes shown and compared are the original `claimedSize`, not the encrypted
  `totalStorageSize` - otherwise every synced photo looked like a conflict.

## 3.17.0 — 2026-11-05

- **New Photos page.** The sidebar is now Home, My files, Photos, Settings,
  Trash, About. Photos lists the account's photo timeline (`photo timeline -d`)
  grouped by month with resolution, size and capture date, and downloads a
  single photo, a selection or the whole timeline into `~/ProtonPhotos`
  (`PROTON_SYNC_PHOTOS_LOCAL` overrides it). Downloads go through
  `photo download -c rename`, so two photos with the same name both land.
  No thumbnails: the CLI exposes no image bytes, and decoding JPEG would mean
  bundling an image codec for a nicety.

## 3.16.94 — 2026-11-04

- **The tray dot keeps its colour at panel size.** The badge was a 12px fill
  inside a 17px white ring. Panels draw the icon at 16-22px, so the ring was
  half of the visible dot and washed the hue out: cyan read as light green and
  amber as pink. The badge is now a 16.5px fill inside an 18px circle with a
  thin dark edge, so only ~17% of its area is not the state colour. Measured at
  16px, "syncing" is `#3bc5d5` (was `#96e5f3`) and "paused" `#c66d0a` (was
  `#e6b27d`).

## 3.16.93 — 2026-11-04

- **The update button survives a moved AppImage.** When the running file had been
  replaced by one with a new name, `APPIMAGE` pointed at a path that no longer
  existed and the whole update aborted with "not running as an AppImage" even
  though the download itself was fine. The build is now placed beside the old
  path when it cannot replace it, and the download is staged in that same
  directory, so an update always lands where the launcher can run it.
- **A rate-limited update check says what it is.** GitHub's unauthenticated API
  allows 60 requests an hour per IP; hitting the limit showed a bare "HTTP 403".
  It now reads "GitHub is rate-limiting update checks - try again in ~N min".

## 3.16.92 — 2026-11-04

- **A stale service unit no longer kills syncing.** When an update installed the
  new build under a different file name, the systemd unit kept naming the file
  that had just been replaced, so every start failed with `203/EXEC` and Home
  sat on "Sync engine isn't running". Writing the unit now follows the build
  that is really on disk: the version being run first, else the newest Drive
  AppImage beside the old path.
- **"Restart syncing now" checks that it worked.** `systemctl restart` exits 0
  even when the unit dies right after exec, so the toast claimed success over a
  dead engine. It now waits for the unit to report active and says so when it
  does not.
- **"Run full check now" really checks everything.** The request also skips the
  cached-snapshot fast path for that cycle, so every folder is listed instead of
  the `fs size` fingerprint answering the request. The one-shot engine commands
  (`--once`, `--dry-run`) consume a pending request too.

## 3.16.91 — 2026-11-04

- **"Run full check now" does something again.** It wrote `audit-request` into
  the state directory and the Rust engine never read it, so the button only
  showed a toast (the file from the last press was still sitting there). The
  engine now treats that file as a wake request, consumes it within a second
  and runs a full walk. The toast no longer promises a delete hold the engine
  does not implement.
- **"Restart syncing now" reports what really happened.** A failing `systemctl`
  was discarded and the toast always claimed success; it now says
  "Syncing restarted." only when the restart succeeded, and names the command
  to run otherwise.

## 3.16.90 — 2026-11-04

- **The tray icon shows the state as a coloured dot again.** The fade-in/out
  pulse and the red X badge are gone: one dot in the lower-right corner is
  green when synced, cyan while syncing or signing in, blue while the engine
  starts, magenta on the welcome screen, amber when paused or your session
  expired, grey when the engine is stopped and red on error. The colours are
  the ones the Home orb and the sidebar dot already use, apart from two the
  panel forced: the working states take cyan because their purple would vanish
  into the purple cloud, and the welcome screen takes magenta because blue
  there means "engine booting". The tooltip still names the exact state.
- **The tray no longer animates.** The 400 ms pulse thread and its icon-only
  refreshes are deleted, so the icon is repainted only when the status
  changes.

## 3.16.89 — 2026-11-02

- **No false “session expired” at boot.** The sync service starts before the
  login keyring is unlocked. A login/keyring error now retries every 10 s for
  the first minute instead of leaving `needs_login` on screen until the next
  configured check (which could be an hour). Once a cycle succeeds, the event
  listener starts normally.
- The 3 s fingerprint watcher no longer starts at boot while the keyring is
  locked; it starts only after a successful cycle if the event listener is
  unavailable.

## 3.16.88 — 2026-11-01

- **Known-file cloud updates skip the folder listing.** When the event is a
  content update of a file already in the snapshot and the local file still
  matches its saved state, it is downloaded directly by node UID: one CLI
  call instead of two, saving ~3.5 s per edit. Anything ambiguous (rename,
  move, trash, local edit, multiple events) falls back to the scoped list and
  planner.
- **CLI startup investigated.** The official CLI is a Bun/TypeScript bundle,
  not .NET: `--version` alone costs ~1.3 s, and an authenticated `fs size`
  ~3.8 s. A long-lived official CLI server is the only zero-key way to remove
  that floor; the request is documented in `docs/UPSTREAM.md`.

## 3.16.87 — 2026-11-01

- **Less CLI overhead.** Removed two ignored `fs info` calls that added ~3 s
  to every transfer phase and full walk.
- **Downloads start before the cache write.** The fallback fingerprint and
  snapshot are written after planning/transfers, so a 3 s `fs size` call no
  longer delays the file you are waiting for.
- **Event polling tightened to 1 s** while idle (was 2.5 s), keeping the
  existing rate-limit backoff.

## 3.16.86 — 2026-11-01

- **Real-time cloud sync restored.** The engine subscribes to Proton's volume
  event stream with the official CLI's own keyring session, shares the CLI's
  event cursor, and re-lists only the folder that changed — the same design
  the old Python engine used. Cloud adds, edits, renames and deletes no longer
  trigger a full tree walk.
- **Scoped post-upload refresh.** After a local upload, trash or move, only
  the affected folders are re-listed instead of re-walking the whole cloud.
- The 3 s `fs size` fingerprint watcher and the check interval are now
  fallbacks: used when the event listener cannot start, or for fingerprint-
  invisible changes if events are unavailable. `PROTON_SYNC_PUSH=0` disables
  both push paths.
- File contents and all mutations still go through the official Proton CLI;
  the listener only reads the same event stream with the CLI's session.

## 3.16.85 — 2026-10-31

- **Tray pulse updates only the icon.** A local ksni patch adds an icon-only
  refresh, so the pulse no longer rebuilds the StatusNotifier menu or the
  other tray properties while it animates.
- **Cloud changes are watched every 3 s.** The engine polls the remote
  `fs size` fingerprint and wakes immediately on a change; the check interval
  stays a fallback for fingerprint-invisible edits. `PROTON_SYNC_PUSH=0`
  disables the watcher and `PROTON_SYNC_PUSH_INTERVAL` tunes it (1–30 s).
- `PROTON_SYNC_FAST_REMOTE=0` and `PROTON_SYNC_PUSH=0` now actually disable
  their features; integer parsing used to clamp 0 up to 1.

## 3.16.84 — 2026-10-31

- **Expired sessions are visible.** When the engine reports `needs_login`, the
  app now shows **Your session expired** and the Sign in action even if a
  cached identity probe still says signed in; the tray shows the X instead of
  a misleading “starting” pulse.
- **Signing in restarts sync.** A fresh login starts the service if it was
  stopped, unless you paused syncing yourself.
- **Tray pulse no longer floods the bar.** The icon updates at ~2.5 fps
  instead of ~8 fps, so other tray icons stay clickable.
- **The check interval explains itself.** “Off” is now “Off (manual)”, and the
  README states that cloud changes arrive on the chosen interval — 30 s on
  **Instant**, while **Off** means local changes or **Sync now** only.

## 3.16.83 — 2026-10-31

- **Sync waits for complete files.** The recursive watcher now wakes on
  close, move and delete instead of every write, so a file being copied is
  never uploaded half-written. A missing sync folder is created and watched
  on first start instead of silently falling back to interval-only syncing.
- **Toasts appear again.** Their timer mixed two clocks, so most expired on
  the first frame.
- **Account menu is keyboard- and screen-reader-friendly.** Enter/Space opens
  it once, its actions have accessible labels, and the dark-theme sign-out
  text now meets WCAG AA contrast.
- **Sidebar width is preserved.** Resizing the window below the pane floor no
  longer overwrites the saved width.
- **Tray idles cheaper** (1 Hz instead of 8) and stops retrying unsupported
  Hyprland dispatch syntax.
- README screenshots and feature copy refreshed; releases auto-prune to the
  seven newest builds.

## 3.16.82 — 2026-10-30

- **Tray icon states, without the dot.** A synced app shows the plain icon;
  while syncing it fades in and out; paused, signed out, engine-stopped and
  error states show a red X badge.
- **Account menu restyled.** The bottom-left account pill opens a compact
  glass card with the avatar, email and live status, a proper sign-out row,
  or a centered gradient Sign in button. It opens from the keyboard and no
  longer stretches across the page.

## 3.16.81 — 2026-10-30

- **The tray opens the window floating on Hyprland.** Showing the app from
  the tray (or a notification) asks Hyprland to float it, retrying until the
  freshly mapped window stays floating, so it behaves like a launcher instead
  of dropping back into the tiling layout. Other desktops are unchanged.

## 3.16.80 — 2026-10-30

- **New and changed files sync immediately.** The engine watched only the
  top-level folder, so a file added inside a subfolder (for example
  `personal/`) waited for the next interval or a manual **Sync now**. Every
  folder is now watched, including folders created later.
- A change that arrives while a cycle is running no longer loses its
  wake-up, so the next cycle starts right away instead of sleeping until
  the following interval.
- `PROTON_SYNC_INOTIFY=0` now actually disables the watcher.

## 3.16.79 — 2026-10-30

- **The sidebar really resizes now.** The stat chips were 4px wider than the
  pane on every frame, so the panel crept back to its maximum width no matter
  where you dragged it. The chips now have exact widths, and the footer is
  measured instead of guessed at, so the status card and account row no
  longer overlap at narrow widths.
- The brand wordmark switches to a compact mark, and the status text and
  account email elide, so a 150px pane is usable. Long stat values elide
  rather than overflow.
- New layout regression test: a too-wide chip row fails the suite.

## 3.16.78 — 2026-10-30

- **Sidebar drag fix.** The pane no longer snaps back to its maximum: the
  live width is only persisted after the pointer is released, so the panel
  stops fighting the drag.

## 3.16.77 — 2026-10-30

- Version bump only — forces a fresh in-app update so the 3.16.76 sidebar
  fixes actually land on machines whose updater compared against the same
  number.

## 3.16.76 — 2026-10-30

- **Sidebar resize is back and remembered.** The pane defaults to a narrower
  200px, can shrink to 150px, shows a visible grab line on its edge (glowing
  on hover), and the width you drag is saved in preferences across restarts.

## 3.16.75 — 2026-10-30

- More breathing room between the sidebar stat chips and the status card
  below them.

## 3.16.74 — 2026-10-30

- **Sidebar footer redesign.** The file count and disk usage are now two
  glass stat chips with accent numbers (`1,037 files` / `1.1 GB on disk`),
  and the account row is a glass pill with a gradient violet→cyan avatar, a
  live status presence dot and a chevron; its edge glows on hover and
  keyboard focus.

## 3.16.73 — 2026-10-30

- **Aurora Glass redesign.** The app now wears a living dark interface:
  a deep-navy page under five slowly drifting violet/cyan/magenta glow discs,
  translucent glass cards and sidebar with hairline edges, a glowing
  two-tone status orb with a glass core, gradient violet→cyan CTA buttons
  and glass pill navigation with a glowing active row. The light theme is
  the same language in frosted daylight. `PROTON_UI_MOTION=0` freezes all
  motion; the backdrop is a handful of painted discs - no textures, no
  shaders, no per-frame allocations.
- Copy polished: “Open local folder”, “Independent community edition”, and a
  calmer activity empty state.

## 3.16.72 — 2026-10-30

- **Transfer queue.** The engine now publishes the in-flight paths; the Home
  hero shows the progress bar plus the files currently moving.
- **Accessibility pass.** WCAG contrast fixes (darker success/warning glyph
  text, theme-aware muted text, #6B7280 neutral for indicators) with a
  contrast regression test; Enter activation and visible focus outlines on
  nav rows, My Files, the folder tree and selective-sync rows; a hover ring
  on cards instead of the unavailable shadow lift; activity rows highlight on
  hover.
- **Workers up to 32** (was 16) in the engine and Settings — a measurable
  ~10% on full walks for users who raise it.
- **Testing depth.** Fuzz-lite readers (state, snapshot, journal), a
  golden-layout gate that the activity row never overflows a narrow card, and
  the WCAG contrast gate join the fake-CLI end-to-end test.

## 3.16.71 — 2026-10-30

- **My Files search.** A client-side search box filters the current listing.
- **Drag-and-drop upload.** Dropping files onto My Files uploads them into
  the open folder.
- **Transfer progress.** The Home hero shows a progress bar with percentage
  while files are moving.
- **Keyboard + focus.** Nav rows and My Files respond to Enter, with a visible
  focus outline; the activity segmented control shows a pointer cursor;
  activity rows get a hover highlight.
- **Density.** Slightly tighter card gaps on Home and Settings.
- **Tests.** Parser robustness tests (garbage state/snapshot/journal) and an
  end-to-end engine test against a fake official CLI.
- `docs/UPSTREAM.md` records the upstream requests (recursive listing,
  per-folder change stamp, arm64 CLI) that would unlock the next big wins.
- Deferred with reasons: conflict-resolution UI (the engine auto keeps-both
  and exposes no per-conflict list), zsync delta updates, golden UI
  snapshots, aarch64 AppImage (official CLI is x86_64-only).

## 3.16.70 — 2026-10-30

- **Fast unchanged cycles.** The engine now checks one `fs size` fingerprint
  before walking folders: when the totals match the last full walk, it reuses
  the cached tree and skips every per-folder listing. Measured on the live
  tree: **2m12s → 4.5s** for an unchanged cycle. A full re-walk is forced
  every `PROTON_SYNC_FAST_REMOTE_MAX_SKIPS` cycles (default 6) or after
  `PROTON_SYNC_FAST_REMOTE_MAX_AGE` seconds (default 6 h), which bounds
  same-size renames the totals cannot see. Set `PROTON_SYNC_FAST_REMOTE=0`
  to disable.
- **Design slices 2–4.** The primary CTA gets a soft gradient sheen; the
  status orb's glow fades in when the status changes; toasts slide up and
  fade instead of popping.

## 3.16.69 — 2026-10-30

- **Design overhaul, first slice.** The Home hero now sits on a soft layered
  accent glow, the status orb's core gets a glass highlight with a stronger
  halo, and cards keep the deeper 16px shadow language from 3.16.68. No new
  colours or layout changes - depth cues only.

## 3.16.68 — 2026-10-30

- **Agency agents integrated.** Vendored nine specialist playbooks from
  `msitarzewski/agency-agents` (MIT) under `agents/`, and `AGENTS.md` now
  points every UI, layout, review, refactor and accessibility change at the
  matching agent.
- **2026-style polish.** Card corners rounded to 16px with softer, deeper
  shadows; nav rows rounder (12px) with a pointing-hand cursor.
- **Friendlier interface.** The hero actions and the sidebar account row now
  have hover tooltips ("Open your Proton Drive folder…", "Account · click to
  sign out or sign in", pause options).
- **Code clarity.** The removed Activity tab's dead references are gone
  (`journal_open` naming, Home-feed module docs, error copy now points at
  Recent activity instead of a page that no longer exists).

## 3.16.67 — 2026-10-29

- The **Recent activity** label now sits directly on top of its card (2px
  gap instead of 12px).

## 3.16.66 — 2026-10-29

- **Hero buttons are back under the subtitle** where they were before
  3.16.65. Only that row's padding/spacing is compacted, so Open my folder,
  Sync now and Pause stay on one line at the minimum width without changing
  position as the window grows.
- The activity timestamp reserves a small right margin so the list scrollbar
  never covers it.

## 3.16.65 — 2026-10-29

- **Activity rows never overlap.** Each row reserves a fixed time column and
  truncates the name/path to the remaining width, so long paths cannot paint
  over the timestamp or widen the card past the window. The search/filter row
  wraps too.
- **Hero actions stay on one line.** Open my folder, Sync now and Pause share
  a single full-width row (indented under the hero text on wide windows).

## 3.16.64 — 2026-10-29

- **Home is the activity feed.** The separate Activity tab is gone; Home keeps
  the status hero and shows the Files moved / Technical log feed.
- Removed the Home account, folder and recently-synced cards.
- **Sign out / Sign in** now lives in a click menu on the sidebar email row.
- The `1037 files · 1.1 GB` line moved to the sidebar footer, above the
  “Unofficial community app” box.

## 3.16.63 — 2026-10-29

- The Home folder card drops the local path, which is already shown under
  Settings → Where your files live. The file count/size stays centered against
  the folder icon; both cards keep the same height.

## 3.16.62 — 2026-10-29

- The compact red **Sign out** box is taller (24px). The account avatar and
  folder icon grow to 50px with it, so both two-row cards stay the same height
  without a hardcoded card box.

## 3.16.61 — 2026-10-29

- **Two content-driven rows.** The account card is email + `Signed in` with a
  compact red-outline **Sign out** on the status line. The folder card is
  stats + path. No third row, no tag, and no hardcoded card height: the 44px
  avatar/folder icon sets the height, so both cards stay equal.

## 3.16.60 — 2026-10-29

- Removed the **Open folder** button again. The folder card keeps a matching
  third row with a quiet, non-clickable **Local folder** tag, so both cards
  stay equal height without a hardcoded card box. The red **Sign out** box is
  retained under the email.

## 3.16.59 — 2026-10-29

- **Red Sign out box restored**, under the email. No more hardcoded card
  height: both cards now use the same three-row structure — email / status /
  action, and stats / path / Open folder — so their heights match naturally.

## 3.16.58 — 2026-10-29

- The quiet account action follows the card's own state: while the card says
  `Signed in`, it offers **Sign out** even if the identity probe is still
  waiting on the CLI lock.

## 3.16.57 — 2026-10-29

- **Quiet account action.** The outlined red Sign out button is gone. The
  account card now reads `Signed in · Sign out` on one line; the action is
  muted until hovered, then turns red with a pointer and underline. Both Home
  cards keep the same height and the folder card uses the file count/size as
  its main line.

## 3.16.56 — 2026-10-29

- **Home card alignment.** The Sign out button now sits under the email instead
  of overlapping it. The folder card drops the redundant “ProtonDrive” label
  and uses `1037 files · 1.1 GB` as its main line, with the path underneath.
  Both cards share a minimum content height, so they stay the same size.

## 3.16.55 — 2026-10-29

- **Home folder card tightened.** Removed the "Your folder on this computer"
  heading, moved the file count/size onto the same line as the folder name,
  and matched the folder icon to the account avatar height so the two cards
  line up evenly.

## 3.16.54 — 2026-10-29

- **Less explanatory clutter.** Removed the long muted notes under How often,
  Performance and Appearance, plus the redundant folder note.
- **Advanced cards join the columns.** Fine-tuning, If something goes wrong
  and Extra environment variables are now always part of the same 1/2/3-column
  packing instead of a separate collapsible full-width block.
- **Home cards pair up sooner.** Account and Your folder sit side by side from
  640px of content width, so a normal window no longer stacks them while there
  is still room.

## 3.16.53 — 2026-10-29

- **Wide windows use their width.** Settings now packs its cards into 1, 2 or
  3 columns as space allows, like TMOG's `MetricsPanelBoxLayout`, instead of
  one full-width column with empty space on the right. Home puts the account
  and folder cards side by side on wide screens.
- The 720×540 layout floor from 3.16.52 still applies: narrow windows keep the
  single-column layout and the wrap/stack rules.

## 3.16.52 — 2026-10-29

- **Compact window sizes are remembered.** The startup floor was still
  920×780, so a window resized to the 720×540 minimum was silently grown
  again on the next launch. The floor now matches the viewport (720×540), so
  the window reopens at the size you left it — the same layout-safe minimum
  behaviour Task Manager OG uses.
- No resize animation was added: TMOG's fluid look comes from native layout
  reflow, not a transition. egui already relayouts every frame; the visible
  clipping was the row/column overflow fixed in 3.16.51.

## 3.16.51 — 2026-10-29

- **Settings cards stay inside a narrow window.** The two nested control
  groups in Performance (and Fine-tuning) made that row wider than the card,
  which widened the whole Settings scroll content and pushed every card past
  the right edge. Each label + dropdown is now its own row; the label stays
  with its input, and below 220px it stacks directly above it instead of
  splitting across lines.

## 3.16.50 — 2026-10-29

- **My files keeps the Modified column** at the narrowest window with the
  sidebar dragged to its maximum. The three columns now use 132/84/124px
  instead of 140/96/132px, so they fit the 720px floor; Modified still drops
  first only below that floor.
- **Settings labels stay with their dropdowns.** Each label + combo is now a
  single wrapped item, so a narrow Performance or Appearance card moves the
  whole pair to the next line instead of leaving the text on the line above
  the dropdown.

## 3.16.49 — 2026-10-29

- **Settings cards fit a narrow window.** Rows in Performance, Notifications,
  Running in the background, Appearance, Updates, Installing on Linux, the
  advanced cards and the Save bar now wrap their controls onto the next line
  instead of being clipped on the right. No behaviour or settings changed.

## 3.16.48 — 2026-10-28

- **Tray icon now fills its slot.** The cloud PNG has ~19% transparent
  padding, so it looked much smaller than full-bleed tray icons like ZapFast.
  The tray composition now crops to the artwork's alpha bounds and scales it
  to 72×66; at a 22px tray slot the cloud is 22×20 instead of ~17×13. The
  24px status dot stays bottom-right and is unchanged.

## 3.16.47 — 2026-10-28

- **Tray icon rebalanced.** The 24px status dot stays bottom-right, but the
  purple cloud is now 68px (C66 plus a little) and sits at the top-left, so
  less of the folder is hidden and the upload arrow stays readable at 22–24px.
  The dot's white ring and green/purple colours are unchanged.

## 3.16.46 — 2026-10-28

- **Readable tray status dot.** The dot is now 24px across (was 14px) with a
  5px white ring, so green (synced) and purple (syncing) stay distinct even
  when the panel scales the 72px icon down to 22–24px. The app icon itself is
  drawn at 70/72 instead of 66/72, a little larger in the same tray slot.
- The CI cache needs no manual bump: its key hashes `Cargo.lock` and
  `Cargo.toml`, and the 3.16.45 run saved the fresh key after the dependency
  cleanup.

## 3.16.45 — 2026-10-28

- **Smaller dependency tree.** Removed direct dependencies nothing referenced:
  GUI `anyhow` and `tokio`; engine `reqwest`, `ureq` and `urlencoding`. The
  workspace graph drops 227 nodes and the lockfile 73 packages. No runtime
  behaviour changes; builds and tests are the same code paths as before.
- Added `AGENTS.md` with the YAGNI/“smallest complete change” rules used here.

## 3.16.44 — 2026-10-28

- **No more "Checking your account..." under the Home buttons.** The account
  card now uses the cached email with **Signed in** immediately (or a neutral
  **Account**/"Signed in" on a first run with no cached email) while the
  identity probe runs. The probe can spend up to a minute behind the engine's
  CLI lock; the UI no longer displays that wait. The Home subtitle now follows
  the real engine status ("Just a moment...") instead of the identity wording.
- The first successful probe still caches the email; later starts show it
  immediately.

## 3.16.43 — 2026-10-28

- **Accurate sign-in states.** A previously signed-in account whose keyring
  session is lost now shows the padlock **"Your session expired"** and
  **Sign in again**; a fresh install still shows **Welcome** / **Sign in**.
- **The engine reports failures.** A failed cycle now writes `status: "error"`
  or `"needs_login"` with the message and duration, instead of leaving the
  health file stuck on "syncing". The Home shows the real error.
- **Signing in wakes sync.** After a successful sign-in the app writes the
  engine wake file and keeps probing identity during the sign-in window (once
  per result, so probes do not stack), so an active engine starts a cycle
  immediately; a stopped engine still offers **Start syncing**.
- **The engine's state wins over the account probe.** If the engine is already
  syncing/checking, Home shows **"Syncing your files..."** immediately, even
  while the identity probe is still waiting on the CLI lock. The account card
  shows the last known email with "Checking...".
- Health metadata: `started` is preserved and `next_poll` is cleared when the
  interval is 0.

## 3.16.42 — 2026-10-28

- **AppStream metadata is fully clean.** Lowercased the component/desktop IDs
  (`io.github.zhafrisbs.protondrivecli`), so `appstreamcli validate --pedantic`
  reports zero issues.
- **Removed the linker-warning suppression.** The GUI and AppRun now build
  with `cargo-zigbuild 0.24.0`, which drops the deprecated `-Wl,-O1`; CI pins
  and verifies that version instead of relying on
  `#![allow(linker_messages)]`. The cache key includes the cargo-zigbuild
  version so a stale 0.23.x cannot come back.

## 3.16.41 — 2026-10-28

- **Clean CI logs.** Silenced Zig's known `linker_messages` false positive
  (`ignoring deprecated linker optimization setting '1'`;
  cargo-zigbuild#487, rust#158192) for the GUI and AppRun builds.
- **Faster CI/AppImage builds.** Release builds now use thin LTO with parallel
  codegen units instead of fat LTO + one unit (GUI links in well under a
  minute on the 4-core runner); fixed the Rust cache key (the workspace
  `Cargo.lock`, not the old `gui/Cargo.lock`), restore the cache before
  installing cargo-zigbuild, cache Zig, and trigger on `crates/**`,
  `Cargo.toml`, `Cargo.lock` and `.cargo/**`.

## 3.16.40 — 2026-10-28

- **The project is now Rust-only.** Removed the legacy Python sync engine, its
  parity tests, the Python bytecode ignores, the PSF licence and the CPython
  bundling path from `build.sh`.
- **The AppImage entry point is Rust.** A new `crates/apprun` binary replaces
  the old `src/AppRun` shell script and dispatches to the GUI, the sync engine
  or the upstream Proton CLI. The packaged payload now contains only Rust
  binaries: the upstream Rust CLI, the GUI, the engine and the launcher.
- Updated the README, source-build guide and third-party notices to match.

## 3.16.39 — 2026-10-27

- **More Home status-orb states.** The circle now has its own glyph and colour
  for: signed out (sign-in arrow), signing in (pulsing key), session expired
  (padlock), engine not running (power symbol), engine starting (clock), paused
  (pause bars), syncing (animated), up to date (check) and errors (exclamation).
- **The Home buttons follow the state:** "Sign in" / "Sign in again", a waiting
  message while the terminal sign-in is open, and "Start syncing" when the
  engine is stopped but the account is fine.
- **Pausing is remembered across restarts** (a marker file), so a paused app
  still reads as paused instead of "engine stopped". A timed pause clears the
  marker once the service comes back.

## 3.16.38 — 2026-10-27

- **In-app updates now update the sync engine too.** The updater replaces the
  AppImage in place, so the systemd unit path never changed and the background
  service kept running the old engine (this machine was on a 3.16.37 GUI with a
  3.16.29 engine). The unit now carries the GUI build, and the app restarts the
  service whenever the running engine's build does not match - so the instant
  folder tree from 3.16.37 actually takes effect.
- The engine publishes `remote-snapshot.json` immediately after the remote scan
  (before any transfers), so the sidebar turns instant as soon as the scan ends.
- Added a regression test that the GUI reads the exact snapshot shape the engine
  writes.

## 3.16.37 — 2026-10-27

- **Instant My files sidebar.** The sync engine now publishes its full remote
  folder/metadata walk to `remote-snapshot.json` after every cycle. The GUI has
  always read that file, but the Rust engine never wrote it - so the sidebar
  (and the first paint of every folder) was falling back to live CLI calls. Now
  the whole tree renders from the snapshot with no network call, and expanding
  a folder is instant.
- **Empty folders are remembered.** A folder known to have no children is
  served from the snapshot as empty instead of being re-listed every time.
- **The engine now actually upgrades.** If a new AppImage changes the sync
  service's path, the app restarts the service. Before this, the service kept
  running the old engine indefinitely (on this machine it was still 3.16.29).
- Raised the persistent folder-listing cache from 300 to 1000 folders.
- **Selective sync:** the snapshot is only written for an unfiltered walk,
  because a filtered one would omit folders you can still browse in the cloud.
  With selective sync, a folder is still listed only the first time you open it
  (metadata only - file contents are unaffected), then served from the cache.

## 3.16.36 — 2026-10-27

- **Removed the sidebar "+ New" button.** Creating a folder and uploading are
  already available in the My files toolbar and the right-click menu, so the
  leftover pill only added clutter.

## 3.16.35 — 2026-10-27

- **Faster My files browsing.** Opening a folder you have already visited, and
  going back/forward, now serves the cached listing for 30 seconds instead of
  spawning a fresh `proton-drive fs list` against the cloud on every click. The
  first visit to a folder still lists it live; Refresh always re-lists.
- **Removed two UI-thread stalls.** The engine's snapshot is no longer
  deep-copied for every folder and sidebar lookup, and the on-disk cache is
  written at most once a second instead of re-serialising the whole cache after
  every listing.
- **Refresh in Trash** now re-lists the Trash instead of jumping back to the
  last files folder.

## 3.16.34 — 2026-10-27

- **Clicking a file or folder name now selects it.** The name text is still
  selectable for copying (drag across it), but a plain click no longer lands
  only on the text - the row underneath selects, so you no longer have to hunt
  for empty space. Double-clicking a name opens it.
- Right-clicking the name, size or date now opens the same context menu as the
  row background.

## 3.16.33 — 2026-10-27

- **Trash gets its own toolbar.** Upload, New folder and Download are gone while
  browsing Trash. Instead there is an **Empty trash** button, plus **Restore**
  and **Delete permanently** whenever items are selected.
- **Rows keep the pointer cursor.** Hovering a file or folder name no longer
  flips to a text-select (I-beam) cursor. Click-dragging a name still
  highlights it for copying.

## 3.16.32 — 2026-10-27

- **My files rebuilt.** The header and rows now share one column geometry and
  render through egui's own text layout, so every cell is vertically centred
  and lines up perfectly. The sort direction is a small caret beside the
  active column title instead of the oversized arrow.
- **Richer right-click menu.** Added Select all, Duplicate, Details, and - in
  the trash - Empty trash and Delete permanently (both with a confirm).

## 3.16.31 — 2026-10-27

- **Deeper visual refresh.** Cards now float on a soft drop shadow with a
  larger radius, the active sidebar row carries a purple accent bar, file rows
  are taller with more padding, and the empty-folder state is a designed disc
  with a message instead of a bare emoji.

## 3.16.30 — 2026-10-27

- **Polished My files.** Column headers are now the small uppercase, muted
  labels of a real data table; folders wear the Proton purple; the sidebar's
  "+ New" is a filled primary button; and an empty folder shows a friendly
  centred message instead of a blank card.
- **Code cleanup.** Removed dead imports, unused fields and helpers across the
  engine and GUI; a folder chooser loop that never looped is now a single
  block; the poll message boxes its health payload; clippy is clean.

## 3.16.29 — 2026-10-27

- **The first download starts while the cloud is still being read.** A fresh
  sync used to walk the whole remote tree, plan, transfer, then walk the tree
  again. Now each folder is handed to a transfer pool the moment it is listed,
  so files land within seconds and the walk runs alongside them. Measured on a
  1,255-entry account: first file on disk after ~12s (was after the full walk),
  ~530 files in the first 4 minutes.
- **No second walk after a download.** The cloud is walked again only when
  this machine changed it (upload, remote trash, remote move); a download-only
  cycle reuses the tree it already has.
- **Eight transfers at once by default** (`PROTON_SYNC_TRANSFERS`).

## 3.16.28 — 2026-10-27

- **In-app update check works again.** Release tags that carry a suffix
  (`v0.9.0-gui3.16.27-rust`) were read as version `3.16.0`, so a newer build
  looked older and the app reported "already on the latest version". Only the
  leading digits of each part count now, so any tag suffix is ignored.

## 3.16.27 — 2026-10-27

- **"Waiting for the first check" is gone.** The engine wrote no check time,
  interval or next-check time, so the window had nothing to show after a
  cycle. It now records the time of the last check and when the next one is
  due, and keeps the schedule across a restart.

## 3.16.26 — 2026-10-27

- **No more endless re-downloads.** A folder that did not exist locally was
  downloaded as a whole, and its files were then downloaded again one by one.
  Each file was fetched once per folder above it, and the same files were
  rewritten again and again. A folder now only creates its local folder; its
  files are downloaded once.
- **Live progress.** Each finished transfer batch updates the health file, so
  the file totals and "Recently synced" move during a sync instead of only
  when the whole cycle ends.

## 3.16.25 — 2026-10-27

- **Files land in the right folder.** Downloads from the cloud were saved to
  the top of the sync folder instead of their own subfolder. Each file now
  goes to the folder it lives in on Proton Drive. Files from earlier syncs
  stay where they are; a copy left at the top level is a duplicate and can be
  deleted.
- **Faster sync.** Uploads and downloads run in batches per folder, up to
  four at a time (`PROTON_SYNC_TRANSFERS`). The remote walk uses a shared
  work queue with `PROTON_SYNC_WORKERS` parallel listings. A full scan of a
  large tree dropped from about 2.5 minutes to about 1.3 minutes.
- **Retries on folder listing.** A listing that fails is retried
  (`PROTON_SYNC_RETRIES`, default 3) before the cycle gives up. CLI errors now
  show their real message instead of a line of `=` signs.

## 3.16.24 — 2026-10-27

- **First sync starts at once.** The Rust engine waited a full sync interval
  (5 minutes by default) before its first cycle, so a fresh install sat on
  "Checking your files" until then. The first cycle now runs immediately.

## 3.16.23 — 2026-10-27

- **Rust sync engine fixes.** Uploads and downloads were being rejected by the
  official CLI because the engine passed conflict strategies it does not
  accept (`skip-identical`), so changes never reached Proton. Transfers now
  use the same flags as the Python engine (`-d merge`, `-f remove` for
  downloads). Failed transfers are logged, and the health file reports
  uploaded, downloaded and failed counts.
- **Faster idle cycles.** The remote tree is walked in parallel under one CLI
  lock, and a cycle with nothing to do no longer walks the tree a second time.
- **Build fails loudly.** `build.sh` stops when the Rust engine does not
  compile, instead of silently shipping an older engine.

## 3.16.22 — 2026-10-26

- **The status poller stopped doing busywork.** A background thread asked the
  system for everything every three seconds: it spawned `journalctl` for the
  Activity log and the conflict/error notifications, ran two `systemctl`
  queries, and walked the whole sync folder every ~21 s to count files and
  bytes. The state directory is now watched with inotify, so a second launch
  brings the window forward at once, and the expensive reads only run when
  something actually happened: the log follows new engine telemetry (still
  every 3 s while Activity is on screen), the unit state is checked every 5 s,
  and the file totals refresh when a sync cycle finishes instead of on a blind
  timer. Sync itself is unchanged - the engine already watches your folder
  with inotify and receives Proton's change events; this only cuts idle CPU
  and makes the window react faster.

## 3.16.21 — 2026-10-26

- **"Match my system" follows your desktop.** Choosing it after Light used to
  leave the app light, and a fresh start ignored the desktop entirely: on
  Linux egui is never told the system theme (winit reports none, and the
  window deliberately runs on X11/XWayland). The app now reads the XDG
  portal's colour scheme, falling back to `gsettings` and GTK's settings file,
  so the Appearance choice matches the desktop and follows a dark/light flip
  while it runs. Setups that expose none of those start light.

## 3.16.20 — 2026-10-26

- **The missing icons are back.** The pane carets, the cloud sort arrow, the
  selective-sync tri-state boxes, the upload/download arrows and the folder
  cards render again. Rendering the interface with OpenGL in 3.16.19 also
  dropped eframe's system font fallback, so every character the bundled fonts
  lack was drawn as a blank box. Restoring the fallback fixes non-Latin
  filenames (Chinese, Arabic, Cyrillic) showing as boxes in the file list too.
  Costs about 0.6 MB of binary size, and a test now fails the build if the
  fallback is removed again.

## 3.16.19 — 2026-10-24

- **A smaller, lighter interface.** The GUI now renders with OpenGL (`glow`)
  instead of wgpu, and the release binary is built with `panic = "abort"`.
  In a same-machine A/B (both tray-hidden), the interface process' idle memory
  dropped from **114.5 to 55.4 MiB PSS** (−59 MiB, −52%), the GUI binary from
  **22 to 15 MiB**, and the AppImage from **81 to 78 MB**. No visual or
  behavioural change.

## 3.16.18 — 2026-10-24

- **Worker dropdowns always work.** "Scan workers" and "Parallel connections"
  are selectable even while adaptive scans is on, instead of greying out with
  no way to open them. Your chosen value is now a real ceiling: adaptive scans
  may use fewer workers under memory pressure or rate limiting, but never more
  than you picked. Rate-limit recovery stops at your limit too.

## 3.16.17 — 2026-10-24

- **Settings numbers are dropdowns.** Scan workers, Transfer concurrency,
  Parallel connections and Retries per folder are picked from a list instead
  of dragged. Every value in range is offered, an out-of-range value already
  in the config clamps to the nearest valid choice, and the two cards that
  edit the same setting stay in step (both grey out while adaptive scans is
  on).
- **Tooltips on greyed rows.** Hovering a worker row while adaptive scans is
  on now explains what the setting does instead of showing nothing.

## 3.16.16 — 2026-10-24

- **Fast start on a warm machine.** The engine now loads the last remote
  metadata snapshot and the sync state immediately, publishes a healthy
  status, and starts syncing without waiting for the old ~90-second full walk.
  A cached remote tree is shown as *Ready from cache* while a background audit
  catches up. On the current 1,230-entry account the status is ready in about
  0.12 s instead of 4.1 s (and the old path still walks first, at ~90 s when
  there is no cache).
- **A chunked background audit replaces the blocking startup walk.** The audit
  takes the CLI lock for small folder chunks, releases it between chunks, and
  reports progress in health/GUI. It completes in about 96 s for 213 folders
  without freezing local work. Push/local events queue behind the chunks.
- **Deletes and existing-file edits are held while the cache is being
  audited.** New files and new folders can upload. Once the audit is trusted,
  held paths are reconciled: local edits upload, deletes go to trash in the
  correct direction, and a both-sides edit keeps both versions as a conflict
  copy. Audit failures leave the deletes held and show the error; they never
  silently claim the cache is up to date.
- **Adaptive scans and parallel transfers.** Scan workers start at the
  configured 12 and cap at 16, backed off by the RAM budget and by 429/rate
  limits. Independent parent-folder transfers run in parallel with a default
  of 4 (allowed 1–10). A disposable 64 × 512 KiB benchmark improved from
  119 s at 2 workers to 89/87/81 s at 4/6/10, with zero failures; 10 is stable
  on this account but remains opt-in.
- **Performance controls and diagnostics.** Settings → *Performance* adds fast
  start, adaptive concurrency, transfer concurrency, **Run full check now**,
  and **Clear snapshot cache** (confirmation required; removes only the
  snapshot). Home shows audit progress and the “held for safety” note.
- **One-key rollback.** `PROTON_SYNC_FAST_START=0` restores the old
  fingerprint/peek/full-walk path; state, local files and trash history are
  untouched. Periodic audits use `PROTON_SYNC_AUDIT_EVERY` (default 1800 s;
  disabled when scheduled checks are Off). Details and measurements:
  [docs/performance/fast-start-sync.md](docs/performance/fast-start-sync.md).

## 3.16.15 — 2026-10-23

- **Scheduled checks can be turned off.** Settings → *How often* now has an
  **Off** choice (`PROTON_SYNC_INTERVAL=0`). The timer-driven cycles stop,
  and with them the periodic safety walk; cloud push and local edits still
  sync immediately, and the Home **Sync now** button still runs a full check.
  The status line simply shows the last check with no countdown. One cycle
  still runs when the service starts, so the snapshot and status are ready.

## 3.16.14 — 2026-10-23

- **The window leaves the welcome screen once the keyring unlocks.** The
  startup sign-in probe runs at the same moment the keyring may still be
  locked, so it can report "not signed in" even though the engine has since
  unlocked and resumed. The window now re-checks the account when the engine
  reports a keyring problem and again when the engine is healthy, instead of
  waiting for the next launch.

## 3.16.13 — 2026-10-23

- **No more false "session expired" after boot or resume.** The official CLI
  says "You need to login first" both when the session item is really gone
  *and* when the OS keyring that holds it is still locked - which is normal
  on a machine that boots straight into the desktop (auto-login) or wakes
  from suspend. The engine now asks the Secret Service first: if the item is
  present but in a locked collection, it triggers the desktop's own keyring
  unlock prompt and retries in 30 seconds instead of telling you to sign in
  again. A fresh Proton login is only requested when the item is actually
  missing, or when Proton keeps rejecting a session that is still stored
  (5 consecutive attempts), which is the genuine server-side revocation.
- **Why it looked like a multi-PC problem.** Using the app on two machines is
  fine - each keeps its own keyring item and Proton allows several active
  sessions. The morning failure on the second machine was its keyring being
  locked at boot, so the stored session could not be read. The log showed an
  unchanged session item and a later sync without a new sign-in, which is
  exactly that case.
- The auto-unlock prompt can be turned off with
  `PROTON_SYNC_KEYRING_UNLOCK=0`; the status message then tells you to unlock
  the keyring, without claiming the session expired.

## 3.16.12 — 2026-10-23

- **Much lower memory while scanning.** The engine runs the official CLI once
  per folder, and each live `proton-drive` process costs roughly 100 MB - so
  the worker count, not the Python bookkeeping, was the real RAM knob. The
  parallel-scan ceiling now defaults to 6 instead of 12, and it is capped
  again at runtime from `MemAvailable`: a low-RAM machine quietly uses fewer
  parallel CLI processes instead of swapping or getting OOM-killed mid-scan,
  while a roomy machine keeps the configured ceiling.
- **Less idle work.** The safety poll defaults to every 5 minutes (was 1),
  the full-walk ceiling to 30 minutes (was 10) and the peek age to 15 minutes
  (was 10). Real-time push detection is unchanged (~1 s), so the app stays
  responsive while polling the server far less.
- **Tidier process memory.** The service unit sets `MALLOC_ARENA_MAX=2`
  (glibc otherwise creates one allocation arena per thread, which keeps
  freed memory resident), and the engine runs a collection after every cycle
  so the per-cycle tree dictionaries are released before it idles.

## 3.16.11 — 2026-10-21

- **Sync no longer loses the sign-in.** The official Proton Drive CLI keeps
  its session in the OS keyring and rewrites that item whenever it refreshes
  the access token - which happens the moment a call comes back 401. The
  app ran a whole scan of `proton-drive` processes in parallel, so when the
  token expired every one of them hit the 401 at the same second and every
  one of them rewrote the same keyring item. gnome-keyring's `login`
  collection is an alias collection: it refuses the duplicate ("asked to
  register item .../login/N, but it's already registered"), and the item
  that got dropped was the session itself. From then on every call came
  back 401 "You need to login first" and the sync was dead until the user
  signed in again - usually a day or two after the last sign-in.
  The engine now probes the session with a single call, alone, before it
  lets its workers run, so a token refresh is absorbed once instead of
  twelve times; the GUI and a sign-in take the same app-wide lock
  (`~/.local/state/proton-sync/cli.lock`), so neither can rewrite the
  session while a scan is running.
- **A lost session says so.** The engine reports it as a distinct state
  instead of a generic sync error, and the Home screen shows "Your session
  expired" with a **Sign in again** button - no more reading "You need to
  login first" in the Activity log and guessing.

## 3.16.10 — 2026-10-21

- **Interface size setting.** The app draws its own text at a fixed point
  size and does not follow the desktop's font settings the way a browser
  does, so on a small 15" laptop panel the type could look oversized next
  to Firefox. Settings → Appearance → *Interface size* now offers Compact
  (85%), Default (100%) and Large (110%); it scales text and widgets
  together like a browser zoom, applies immediately and is saved per
  machine.

## 3.16.9 — 2026-10-21

- **The Home status ring is concentric again.** The orb's inner glass
  disc was painted slightly above the centre as a fake dome highlight,
  which read as a circle sitting off-centre in both the idle and syncing
  states. The core is now a flat disc, so the ring, core and state glyph
  share one centre.

## 3.16.8 — 2026-10-21

- **Autostart keeps working after the AppImage moves.** The login entry
  used to keep whatever path existed when the switch was flipped; if the
  AppImage was later moved (or an old copy removed), the entry pointed at
  nothing and systemd's autostart generator skipped it silently - the
  switch still showed on, but nothing started at login. The app now
  rewrites its login and app-menu entries to the current AppImage path on
  startup, so a moved file heals itself instead of needing the switch
  toggled off and on again.

## 3.16.7 — 2026-10-21

- **A designed minimum window size.** The window now bottoms out at
  720x540 - half of a 1440-wide screen, enough for the sidebar plus the
  three list columns. The sidebar's drag limit follows the window, so it
  can never squeeze the file list below the width its columns need.
  Should a compositor still force a smaller window (X11 size hints are
  advisory), the list degrades cleanly: it drops the Modified column,
  then Size, instead of letting names overlap values.
- **Sidebar folders always expand and collapse from the caret.** The tree
  used to force the path you were inside open, so its caret did nothing
  until you navigated elsewhere. Navigation now expands the path once,
  and every branch collapses immediately - the current folder keeps its
  highlight.

## 3.16.6 — 2026-10-21

- **Long names stay in the Name column.** Dragging the sidebar wide makes
  the list narrow; file names were cut at a fixed character count, so a
  long name painted straight over the Size and Modified values. Names are
  now measured against the real column width and end in an ellipsis before
  the next column starts.
- **Column headers sit over their values.** Name aligns with the file names
  (past the icon), and Size/Modified are right-aligned exactly like the
  numbers and dates below them.

## 3.16.5 — 2026-10-21

- **The collapsed address bar sits on one line again.** With a long final
  folder name the breadcrumb's hidden-path "..." was drawn as a disabled
  button, which dropped a few pixels below the crumbs either side of it.
  It is now a plain ellipsis label on the same baseline as the rest of
  the row.

## 3.16.4 — 2026-10-21

- **The file list stays aligned after you shrink the window.** Going deep
  into folders while the window was stretched and then narrowing it left
  Size and Modified misaligned against their values, dates clipped at the
  window edge and the address bar running past the pane. Two rows could
  claim more width than the card they sit in: the breadcrumb's collapse
  step inserted "..." without ever dropping a segment, and the header row
  forgot the spacing between its three cells - egui grows a row's frame
  to whatever its content claims, so the table inherited that inflated
  width while the header kept the real one. The breadcrumb now actually
  drops middle segments (first and last crumbs always kept, each still
  pointing at its full path) and the Name column subtracts the cell
  spacing, so header, values and dates share one width at every window
  size.

## 3.16.3 — 2026-10-20

- **Folder changes finally show their name.** The engine logs whole-subtree
  downloads as `DOWNLOAD dir  <path>/ (whole subtree)` and marks trash/mkdir
  lines with a `remote`/`local` scope word; the row parser passed those
  tokens through verbatim, so Home showed a bare "Downloaded" with no name
  and Activity rendered the row as just "/". The parser (and the
  notifications' name computation) now reduce every change line to the
  plain path - scope/type word, trailing slash and conflict annotation
  removed - so rows read "Personel", "old.txt" and the parent line lists
  the real folder chain.

## 3.16.2 — 2026-10-20

- **The files toolbar is icon-only now.** Refresh moved to the front of
  the row (before the navigation arrows) and all three controls are drawn
  glyphs in the sidebar's line style - a refresh ring and two arrows -
  with the words kept on hover. With no text button sharing the row, the
  breadcrumb gets the full remaining width before long paths collapse to
  "…".

## 3.16.1 — 2026-10-20

- **Compact address bar for deep folders.** A long path (e.g. My Files >
  UUM-Bungalow > Zhafri > PTW > New > Ah Beh) used to stretch the
  breadcrumb across the whole row — and slide it under the Refresh
  button. Breadcrumb segments are now unframed and tight, collapse
  earlier, and their area is bounded so Refresh can never be overlapped.
- **The Home status orb is now a status ring:** a soft halo, a Proton-navy
  glass core, and the status color worn as a ring — a smooth comet sweep
  with pulsing dots while syncing, a checkmark when synced, pause bars
  when paused, an exclamation on errors.

## 3.16.0 — 2026-10-20

- **All six pages live in the left pane now, in Proton Drive's own order:**
  Home, Activity, My files, Settings, Trash, About. The four pages that
  3.15.0 had moved behind the account row are first-class nav rows again;
  the account row stays at the bottom showing your avatar and e-mail.
- **The pane is resizable:** drag its right edge (176–400 px wide). The
  default width is unchanged - 216 px, exactly what the pane has always
  been - and the width you pick is remembered across restarts.

## 3.15.1 — 2026-10-20

- **The in-app updater works again.** Releases listed their checksums as
  `./ProtonDrive-...AppImage` (the way `sha256sum` prints path arguments)
  while the updater looks entries up by the plain file name - a mismatch
  that has failed every verified in-app update with "Update failed:
  checksum entry not found" since checksum verification was introduced.
  Releases now list plain file names, which every already-installed
  version accepts, and the updater additionally understands the old `./`
  form. Updating from the Settings page works again from any installed
  version - including 3.13.x straight to this release.

## 3.15.0 — 2026-10-20

- **The left pane now follows Proton Drive's own sidebar layout** (the one
  shown in donniedice/protondrive-linux, which wraps the official web app):
  a **+ New** button with a New folder / Upload files / Upload folder menu,
  an expandable **My files** folder tree, and **Trash**. The tree is lazy -
  expanding a folder fetches its real contents on demand with read-only
  listings, the route down to your current folder stays expanded, and the
  tree never touches your sync state. The other pages (Home, Activity,
  Settings, About) moved behind a new account row at the bottom of the
  pane, showing your avatar and e-mail; click it to switch pages.
- The selected theme (Light/Dark/System) is now applied at startup - the
  preference was saved but never used before.
- The status line under the file list no longer swallows its own actions:
  Download / New folder / Upload show again at the right edge (a justified
  label in that row used to take the whole width and push them out of the
  window).
- The Activity search rows no longer overflow their card: the "Everything"
  filter (Files moved tab) and "Copy log" button (Technical log tab) were
  clipped past the window's right edge.

## 3.14.0 — 2026-10-20

- **The desktop app is rewritten in Rust.** Same app - dashboard, system
  tray, Cloud page, selective sync, notifications, logs, settings, the
  self-installing updates - but it starts faster and the AppImage shrinks
  from 167 MB to 81 MB, because the bundled Python/Qt desktop stack is
  gone and the app is now a single native binary. The sync engine and your
  sync folder are untouched, and your settings carry over as-is.
- The glibc requirement drops to **2.28+** (Debian 10 / Ubuntu 20.04 era
  or newer) - previously 2.34+, a floor that came from the Qt wheels.
- With Qt gone the AppImage contains no GPL/LGPL component at all: it is
  distributed MIT like the rest of this project (full inventory in
  THIRD_PARTY_NOTICES.md).
- **aarch64 builds are discontinued.** Releases now ship x86_64-only: the
  ARM64 binary could never be executed by CI (cross-compiled and shipped
  untested), while x86_64 is built, smoke-tested and run on every release.
  The 3.13.x aarch64 AppImages remain downloadable on their release pages.

## 3.13.2 — 2026-10-19

- **Faster both ways, and cloud changes can no longer queue behind
  housekeeping.** After an upload the engine used to re-walk the entire
  cloud before handling anything else (30-40s), so a cloud change arriving
  in that window just waited; it now re-lists only the folders that cycle
  actually changed (2-4s), keeping the real-time event stream free. Local
  saves also go out sooner (quiet-before-upload 4s → 2s, minimum gap
  between cycles 10s → 4s) and the size check runs beside the folder
  re-list instead of after it. Measured on this machine: create locally →
  in the cloud **8.5s**, create in the cloud → on disk **9.7s**, delete
  locally → gone from the cloud **7.3s**, delete in the cloud → gone from
  disk **5.5s** (previously ~8-13s, and ~20-28s when an event collided
  with the re-walk).
- New optional knobs: `PROTON_SYNC_DEBOUNCE` (default 2) and
  `PROTON_SYNC_MIN_GAP` (default 4).

## 3.13.1 — 2026-10-19

- **Deleting a folder now deletes it in the cloud too.** Removing files
  locally already propagated, but folders were silently kept on the server
  forever (a folder has no saved local fingerprint to compare against), so
  an empty folder you trashed would linger in Drive and the two sides
  drifted apart with no error anywhere. Folder deletions now reach cloud
  trash (recoverable), a folder re-created in the cloud is downloaded
  instead of trashed, and its contents ride along with it rather than
  erroring one by one.

## 3.13.0 — 2026-10-19

- **Cloud changes now sync in ~10 seconds instead of a minute or more.**
  The engine listens to Proton Drive's own change events - using the
  session the official CLI already keeps in your keyring (nothing new to
  install, tokens never shown or touched) - and re-checks only the folder
  that changed instead of re-walking the whole tree. Measured on this
  machine: a file added on another device appears locally in ~11s, and
  deleting it remotely removes it locally in ~8s (previously up to two
  minutes, because Proton's size summary lags behind deletions).
- **Much less work while idle.** Changes now arrive as events, so the
  engine just takes a quick peek every 60 seconds and does a full safety
  walk every 10 minutes instead of walking constantly. Local edits still
  upload within seconds.
- **Nothing changes if events are unavailable** (no keyring session,
  unusual setup): the engine silently falls back to the previous polling
  behaviour - sync correctness never depends on it.

## 3.12.5 — 2026-10-19

- **Edited files are no longer silently skipped and never uploaded.** If you
  saved a change at the exact moment the sync engine was busy (within its short
  working window), it could mistake your edit for one it had already sent and
  report "in sync" without ever uploading it - so a file you just edited could
  still download as the old version, with no error anywhere. The engine now
  remembers what it actually planned and re-sends any change it hasn't truly
  uploaded yet, so an edit always reaches the cloud.
- **Editor lock and temporary files are no longer copied to the cloud.**
  Opening a file in LibreOffice, MS Office, or vim used to upload its lock or
  swap file (for example `.~lock.report.xlsx#`) as junk. These are now skipped,
  and any already uploaded are cleaned out of the cloud.

## 3.12.4 — 2026-10-19

- **"Send a test notification" now actually sends one** (and so do every
  other notification: sync finished, problems, update available). The app
  built the message with the wrong argument type for the hints field, so
  the notification daemon rejected it - and because it never asked for a
  reply, it couldn't see the rejection and stayed silent instead of falling
  back to the tray balloon. Notifications now use the correct type, notice a
  refusal, and fall back to the tray so you always see something.
- **Updating the app works - before, "Download and restart" could never
  finish.** The update was downloaded to /tmp and then moved into place in
  one step; those are two different filesystems on most computers, and
  moving a file across them fails ("Invalid cross-device link") every time.
  The download now lands next to the running app, so installing is one
  simple rename.
- **A new version now proves it starts before it replaces yours.** Following
  the updater in ArtCraft (a launcher we studied for this), the freshly
  downloaded AppImage is booted once, off screen, before it is allowed to
  replace the copy you are using. A build that fails that start-up check is
  thrown away and you keep your current version. If the new version still
  fails to come up on its real first start, the app restores the previous
  version automatically on the next launch and tells you.
- **The relaunch after an update no longer races the old copy.** The new
  version is started only after the old one has fully exited and released
  the "already running" lock, so you no longer get an "Already running"
  message right after updating.
- **The update check no longer claims "you're up to date" by mistake.**
  Right after startup it used to compare against a version number that was
  still being fetched from the sync engine (which can take a minute), found
  it empty, and reported up-to-date even when a new release existed. The
  check now reads the installed version directly, immediately.
- **The update check is now polite to GitHub and survives busy hours.**
  Like ArtCraft's updater, replies are cached with their ETag and re-requests
  are conditional; GitHub doesn't count those against its per-hour limit, so
  checking every six hours while the app is open (new) can never be throttled
  into "Could not check for updates". If GitHub does rate-limit anyway, the
  app shows the last known release instead of failing.
- The app no longer crashes for a moment when it quits. Closing the app while
  it was still checking your account (which happens on every launch for a
  second or two) crashed the program on its way out. The app now shuts down
  cleanly instead.
- The app now also re-checks for updates every six hours while it is open,
  not just once at startup, and the downloaded update is verified against
  the published SHA-256 checksum as before.

## 3.12.3 — 2026-10-18

- **The tray menu works again and looks like your system's menus.**
  Since 3.9.0 the app popped its own copy of the tray menu at the mouse
  position; on a Wayland bar that menu is a window without popup rights, so
  clicks dismissed it instead of activating its entries, and it was painted
  in the app's colours rather than your desktop's. The menu is now handed
  back to your desktop tray (the standard DBusMenu protocol your panel
  already speaks), which draws it itself, at the tray icon, in your system's
  own style - the same menu you get from any other tray app.

## 3.12.2 — 2026-10-18

- **"Open my folder" works again (and so do the sign-in terminal and every
  link the app opens).** The bundled Qt libraries were announced to the whole
  session, so every program the app started for you - the file manager behind
  "Open my folder", the terminal used for the one-time sign-in, your browser
  from the update page - was handed the app's own Qt copy instead of the
  system's and crashed before showing anything. The bundle now stays inside
  the app, and the app launches those programs with your system's libraries
  again.
- **The app icon shows properly in the window title and the system tray.**
  Inside the AppImage the icon file wasn't found, so both fell back to a
  plain purple square. The icon now travels with the app, and "add to app
  menu" installs the real icon too.
- **The folder picker in Settings now follows "Only sync the folders I
  pick".** Turning that option off greys the list out, and saving with the
  option on but nothing picked asks you to pick instead of silently syncing
  the whole cloud (an empty pick means "everything" to the sync engine).
- Fixed a crash in the status refresh on desktops without a system tray.

## 3.12.1 — 2026-10-18

- **Long file names no longer squeeze the window wider than its minimum.**
  The recently-synced rows on Home and every entry in Activity showed file
  names as plain labels, whose width follows the text - a 60-character name
  (like a long photo title) dragged the page ~612 px wide and gave both
  pages a horizontal scrollbar at the compact window size. Names and paths
  now shorten in the middle with "…" and show the full text on hover,
  whatever the file is called.

## 3.12.0 — 2026-10-18

- **No other software needed - the AppImage is completely self-contained.**
  A portable Python 3.12 and the full Qt6/PyQt6 desktop runtime now live
  *inside* the file, so double-clicking works on a machine with nothing but a
  normal desktop: no `python3-pyqt6`, no `python3-qt6`, no `python-pyqt6`,
  no AUR, no terminal commands. Every bundled component's licence travels
  with it (CPython PSF-2.0, Qt6 LGPL-3.0-only, PyQt6 GPL-3.0-only - texts in
  `usr/share/licenses/`, catalogued in `THIRD_PARTY_NOTICES.md`); this
  project's own scripts stay MIT.
- **System requirements are now just a graphical session**: FUSE + OpenGL
  (already present on anything running a desktop), glibc 2.34+ on x86_64
  (Debian 12 / Ubuntu 22.04 / Fedora 36 era or newer) and glibc 2.39+ on
  aarch64 (Debian 13 / Ubuntu 24.04 / Fedora 40 era or newer).
- **The download grows from ~40 MB to ~167 MB (x86_64) / ~153 MB (aarch64)**
  - that is the price of carrying the whole desktop stack instead of
  borrowing the host's. Source checkouts without the bundle still fall back
  to the system's Python + PyQt6, with the same friendly install hint as
  before.
- The sign-in button still opens Proton's official terminal login once (their
  CLI handles email, password and 2FA there); the app picks the session up
  automatically a few seconds later. That is the only step that touches a
  terminal, ever.

## 3.11.0 — 2026-10-18

- **The AppImage is now the whole app - double-click and it opens.**
  Previously the released file carried only Proton's command-line tool; the
  app window and the sync engine now live *inside* the AppImage, so a fresh
  computer needs nothing but the download: double-click, sign in, pick a
  folder. The upstream CLI is untouched - running the file with arguments
  (`auth login`, `--help`, ...) still goes straight to Proton's binary.
- **First run installs itself.** Saving your settings for the first time
  creates the background sync service and points it at wherever this AppImage
  file lives. Move or upgrade the file later and the app re-points the
  service on its next launch - nothing to reinstall, no paths to fix by hand.
- **One host requirement: Python 3 with PyQt6** from your distribution's
  official repositories (`python3-pyqt6` on Debian/Ubuntu, `python3-qt6` on
  Fedora, `python-pyqt6` on Arch/CachyOS). If either is missing, the app
  says exactly what to install instead of failing silently.
- The sign-in button still opens Proton's official terminal login once (their
  CLI handles email, password and 2FA there); the app picks the session up
  automatically a few seconds later.

## 3.10.1 — 2026-10-18

- **The tray icon now behaves like every other app's.** Left-clicking it
  opens the window; clicking it again tucks it back into the tray. (Before,
  a click could only ever open - never close.)
- **Right-click menus open where you right-clicked.** The file/folder menu
  in the cloud browser - and every other menu (upload, pause, the tray's
  own) - used to pop up in the middle of the screen on compositors that
  centre floating windows; they now open at the cursor, next to the item
  you clicked.

## 3.10.0 — 2026-10-18

- **Deletions now follow you both ways.** Deleting a file locally removes it
  from the cloud, and deleting it in the cloud removes it locally - the
  behavior you expect from any modern sync client, and the one complaint
  this app had. Nothing is ever deleted *permanently*: a local delete goes
  to Proton's trash, a cloud delete goes to your desktop trash, so both
  directions are one click away from being undone. (This was previously an
  opt-in setting; it is now on by default. *Also remove files I delete* in
  Settings turns it off.)
- **A broken scan can no longer empty a side.** With deletions following
  by default, the dangerous what-if deserves an answer: if a scan ever
  comes back empty on one side while the other side has files (a moved or
  unmounted disk, a broken login, a mistyped folder), every deletion is
  held for that cycle and logged loudly - the engine never interprets a
  sick scan as a user decision. Downloads and uploads are unaffected.
- **Renames and moves are followed, not re-uploaded.** Renaming or moving a
  file (or a whole folder) used to mean: trash the old copy, upload the
  entire file again - a 2 GB folder rename could re-transfer gigabytes. The
  engine now recognizes a vanished path paired with an identical newcomer
  (exact size + timestamp match) and simply relocates the file server-side
  in one call, transferring zero bytes. This is the same technique rclone
  ships as `--track-renames` (and what Google Drive does natively). A
  rename made in the web app or on another device is mirrored back with a
  plain local rename - nothing is re-downloaded (Proton file IDs are stable
  across renames and moves, so the match is exact, never guessed).
- **Local changes reach the cloud in seconds, not ~45 seconds.** The
  file-watcher used to wait 12 seconds for edits to settle and up to 30
  seconds between cycles; it now waits 4 seconds and at most 10, so a save
  is on your drive in roughly 5-14 seconds - the same feel as Google
  Drive's client. A burst of edits still coalesces into one cycle.
- **Cloud-side deletions are caught within a cycle or two, not hidden for
  minutes.** The engine peeks a cheap server-side size signal to skip
  re-listing your Drive; measured against the live API, that signal can lag
  **over 80 seconds** behind a deletion while the real listing is fresh in
  5 seconds. The peek is now time-bounded (`PROTON_SYNC_PEEK_AGE`, default
  120s, 0 = trust it), so a file deleted in the cloud or on another device
  lands in your desktop trash within about a minute instead of hiding until
  the next safety walk. (Proton offers no change-feed API, so some polling
  ceiling always exists - this is the same limitation rclone has on
  Proton.)
- **A deletion that is held once is still respected later.** Fixed a state
  bug where a deletion held for one cycle (by the wipe guard or a stale
  peek) lost its memory of the other side and could then never propagate -
  it now fires on the first sane cycle. Deletions-off keeps the opposite
  behavior on purpose: an old held deletion never fires if you turn
  deletions on later.
- **A brand-new top-level folder can no longer wedge the sync.** Creating
  a folder directly in `~/ProtonDrive` (not inside a subfolder) crashed
  every following cycle with an uncaught error - found by the new live
  end-to-end test, fixed, and covered by a regression test. One bad path
  can no longer take the whole sync down: unexpected per-file errors are
  now caught, logged, and retried instead of killing the cycle.
- Config cleanup: `PROTON_SYNC_DELETES` now defaults to `1`; existing
  configs are honored as-is (no surprise behavior change on upgrade until
  you flip it in Settings or the file). Inotify timings are derived from
  the same constants the log prints, so the journal always tells the truth.
- Tested: 31 engine unit tests (both deletion directions, the
  hold-deletions-on-sick-scan guard, single-call folder moves, ambiguous
  matches falling back safely, real edits never being mistaken for moves,
  and the held-deletion-memory fix) + a live end-to-end run against the
  real Drive (27 checks: seed, delete each way, rename, whole-folder move,
  settle, clean-up) + the full 190-check GUI suite - all passing, and a
  live dry-run against the real Drive confirmed zero pending actions with
  deletions enabled.

## 3.9.0 — 2026-10-18

- **Dialogs are readable again.** Confirmation boxes (*Move to trash?*,
  *New folder*, *Rename…*, file pickers) could come out dark-on-dark on
  desktops with a dark system palette - every button, label and input
  inside them is now explicitly themed, in light and dark mode alike.
  Verified on a real desktop, not just in tests.
- **Dialogs stay on the app window.** On compositors that re-centre every
  floating window (a common Hyprland setup), popups used to appear in the
  middle of the *screen*, far from a window you had dragged aside. They are
  now placed on the app window itself, wherever it is.
- **Floats like a launcher, with zero setup.** On tiling compositors
  (Hyprland, Sway, i3) the app now asks for the dialog window type, which
  those compositors float natively - no window-rule files to edit, on any
  machine. GNOME and KWin already float ordinary windows and are untouched.
- **Tray right-click menu appears where you click.** Over a Wayland bar
  (SNI host) the menu used to pop up mid-screen; it now opens at the real
  cursor position, read from the compositor when needed.
- **Your selection survives a refresh.** A background reload landing while
  you had a file selected used to silently deselect it - the row was still
  there, but the Download button went dead under your cursor. Reloads of
  the same folder now keep the selection, exactly like a browser keeps
  your place when a page reloads (navigating still clears it, as before).
- **Buttons respond to real clicks again.** A subtle bug (present since
  3.7.0) made *Download*, *New folder* and *Upload* silently do nothing
  when clicked - Qt hands a click's on/off state to the clicked slot, and
  the app's crash-guard wrapper passed it on to functions that take no
  arguments, which failed invisibly. The guard now hands over only what
  each function accepts. Everything worked when invoked from menus and
  the keyboard, which is why this stayed hidden - it is now covered by
  tests that press the buttons with real synthesized mouse clicks, so it
  cannot regress.
- **The Trash button shows its state.** The selected look was defined but
  never applied because the button was missing its style object name; the
  Trash view now visibly marks itself as active.
- **Tested the way a user clicks.** The test suite now presses the
  Download / New Folder / Upload buttons with real synthesized mouse
  clicks (not direct function calls), so a button that stops responding
  to actual clicks is caught immediately - plus new checks for dialog
  theming, the tray menu and the trash state.

## 3.8.0 — 2026-10-17

- **Cloud browsing no longer waits for the network.** Every folder now
  paints instantly - from the sync engine's latest cloud scan (the whole
  tree, seconds old) and from your own browsing history, which is kept
  between app launches. Opening a folder shows its contents immediately
  (measured: about a millisecond, vs 2–5 seconds of waiting before) while
  a fresh copy loads behind it and confirms the view within seconds.
  A folder you just created, renamed or deleted can never linger as a
  stale row - actions always clear the way for fresh data.
- For the record: a real cloud check still takes the same ~2–3 seconds
  (that is Proton's network round-trip, and the AppImage adds nothing to
  it - measured) - you simply never wait for it anymore.
- Sync engine hardening: a completed full scan is now always cached even
  if the quick follow-up check fails for a moment (a network hiccup used
  to silently discard the scan and force another ~2-minute one on the
  next cycle), and the rare "walking instead of peeking" fallbacks are
  logged visibly instead of only at debug level.

## 3.7.0 — 2026-10-17

- **Cloud is now a full file manager, like browsing in a web browser**:
  right-click anything for **Rename…**, **Move to…** (folder picker included)
  and **Move to trash**; create folders; **upload files or whole folders**;
  select several items to download, move or trash them together. A **Delete**
  key does the same as *Move to trash* — and trash is never permanent.
- **Back / Forward buttons and clickable breadcrumbs** (My Files › Personel › …)
  jump straight to any level — no more retracing steps one folder at a time.
- **Instant navigation**: each folder's last listing paints immediately while
  fresh data loads behind it, exactly like a browser's back button — going
  back or up a level no longer waits on the network.
- Click a column heading to **sort by name, size or date** (ascending or
  descending); folders always stay on top.
- **Trash view**: see everything recently deleted (with the time it was
  trashed) and **restore** it in one click. Files you trash from the app are
  removed from the cloud; with the default safe settings your local copy
  stays on disk (deleting is off by default).
- Download a **whole folder** or several items at once into a folder you
  choose; cloud calls inside the AppImage now spawn the bundled CLI directly
  (fewer moving parts per click).

## 3.6.0 — 2026-10-17

- **New Cloud page**: browse your Proton Drive inside the app like a file
  manager — click through folders (~0.2 s each), see sizes and dates, and
  download any file with one click or double-click. Every view is live:
  website changes appear the moment you open the folder.
- **Smart instant sync**: the engine now asks the cloud one cheap ~2.5-second
  question ("did anything change anywhere?") instead of walking all ~1,200
  folders every cycle. A full walk (~2 minutes) only happens when something
  actually changed, after Sync now, or every 10 minutes as a safety net for
  the rare change the quick question cannot see (renames, same-size edits).
  Website files still land in about 2–3 minutes — at roughly a tenth of the
  background load — and local files upload even faster (no walk in the way).
  New optional knob `PROTON_SYNC_FULL_EVERY` (default 600 seconds).

## 3.5.1 — 2026-10-17

- Engine: a successful cycle now clears any stale error in the health file
  (an old failure message used to linger forever after recovery).

## 3.5.0 — 2026-10-17

- New "Instant (30 seconds)" option for cloud checks — files added on the
  Proton Drive website or phone reach the PC within about half a minute
  (Proton's API has no push/notify, so polling is the fastest physically
  possible; one cloud listing takes only ~0.2 s, so 30-second checks stay
  light). Local changes were already near-instant via inotify.

## 3.4.0 — 2026-10-17

- **One release per update from now on**: every new CLI or app version gets its
  own GitHub release tagged `v<cli>-gui<gui>` (e.g. `v0.9.0-gui3.4.0`) instead
  of re-uploading into a single release — older builds stay downloadable and
  the releases list shows the app's progress.
- **AppImage filenames carry both versions**:
  `ProtonDrive-CLI-0.9.0-GUI-3.4.0-x86_64.AppImage`.
- The in-app updater understands both the new tags and the original `v0.9.0`
  tag, offers updates when either version is newer, and still verifies
  downloads against SHA256SUMS.

## 3.3.0 — 2026-10-17

- Slimmer window: minimum width 960 → **920**, default width 980 → **940**,
  with **zero horizontal scrolling on every page** at every allowed size -
  same guarantees as 3.2, 40px less window (sidebar 232 → 216, page margins
  tightened, pill padding trimmed 2px). The About title/version line still
  never wraps.

## 3.2.0 — 2026-10-17

- Fit guarantee: minimum window width set to 960 so **every** page shows with
  zero horizontal scrolling, and "Proton Drive for Linux" plus the version and
  engine line never break into two lines (Visit-the-website button moved below
  them). Verified against the real window styling on x11, with margin for font
  differences across distros.
- Default height 780: Home, Activity and About are fully visible without
  scrolling; remembered window sizes are floored to this no-scroll layout.
- Slightly tighter pill buttons so rows fit at the smaller sizes.
- Version history now ships in every GitHub release note (this file), next to
  the GUI version shown on the app's About page.

## 3.1.0 — 2026-10-17

- Text diet: shorter labels everywhere; detailed explanations moved to the
  README so the app stays quiet and readable.
- Tray recent-files menu, welcome state when signed out, 12 scan workers.
- Larger app and tray icon: the cloud fills the canvas at every size.
- Notifications carry the Proton Drive cloud icon instead of a lightbulb.

## 3.0.0 — 2026-10-17

- Friendly consumer redesign: status orb, plain language, pill buttons,
  light/dark/system themes, four-page layout (Home, Activity, Settings, About).

## 2.0.0 — 2026-10-17

- Selective sync tree, exclusions, conflict-safe behaviour, health telemetry,
  notifications, pause durations, verified self-updater, app-at-login,
  one-click menu install, background settings sections.

## 1.0.0 — 2026-10-16

- First release: two-way Google-Drive-style folder sync on top of the official
  Proton Drive CLI (systemd user service, never deletes unless opted in),
  packaged as reproducible AppImages for x86_64 and aarch64.
