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
