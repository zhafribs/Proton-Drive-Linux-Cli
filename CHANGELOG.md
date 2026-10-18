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
