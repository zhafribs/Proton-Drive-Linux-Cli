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
