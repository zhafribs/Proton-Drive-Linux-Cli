#!/usr/bin/env python3
"""
proton-sync.py - two-way folder sync for Proton Drive using the OFFICIAL CLI.

Part of Proton-Drive-Linux-Cli (MIT). Talks only to the official Proton Drive
CLI (the `proton-drive` AppImage); no third-party binaries, no AUR.

How it works
------------
1. scan the remote tree (official CLI `fs list --json`, one call per folder,
   calls run in parallel - each call costs ~4s of CLI startup + API, so
   parallelism is what makes a full scan take ~30s instead of ~5min)
2. scan the local tree (os.walk)
3. decide per path by comparing BOTH sides against the local state snapshot -
   each side is compared to its last-seen value, never to the other side,
   which is what prevents upload/download ping-pong
4. execute with the official CLI using explicit non-interactive conflict
   strategies, re-scan, record the new state
5. in loop mode, inotify (via ctypes - no extra packages) wakes the loop the
   moment something changes locally, so local edits upload within seconds;
   cloud changes arrive in ~1s through the real-time push listener (below),
   while a cheap server-side size-peek (`fs size`) and the
   PROTON_SYNC_PEEK_AGE/FULL_EVERY walks stay as the correctness backstop -
   Proton's size aggregate itself can lag 80s+ behind a deletion, so peek-only
   detection can never be trusted alone (the same ceiling rclone hits)

Real-time push (default on; PROTON_SYNC_PUSH=0 disables it)
-----------------------------------------------------------
The official CLI keeps a per-volume event cursor in
~/.local/share/proton-drive-cli/events.json and re-reads its session from the
OS Secret Service on every invocation. The engine subscribes to the SAME Proton
event stream with that session (read via jeepney - pure python, bundled in the
AppImage, nothing installed on the host), shares the cursor file atomically
(write-back after every delivered batch; on 400 "Invalid ID" it re-reads what
the other consumer advanced), and turns each Link event into ONE targeted
re-list of just the affected folder instead of a full recursive walk:

    cloud-side commit -> event in ~1.0s -> folder re-listed -> transfer
    (~8s end-to-end, versus 45-120s walking the whole tree)

Events are hints, the snapshot walk is the truth: unknown/unresolvable events,
push outages, auth failures and rate limits all fall back to the peek cadence,
and PEEK_AGE/FULL_EVERY still bound how long any change can stay invisible.
Tokens are handled in memory only and the refresh token is never touched.

Safety defaults
---------------
* deletions sync both ways by default - but ONLY ever to trash, never
  permanently: local deletions go to Proton's trash, remote deletions to the
  desktop trash (both recoverable; opt out with PROTON_SYNC_DELETES=0)
* a scan that comes back empty on one side while the other side has files
  (moved disk, broken login, wrong path) holds every deletion that cycle -
  no mass deletes from a sick scan
* renames/moves are followed, not re-transferred: a path that vanished and an
  identical path that appeared on the same side is relocated (server-side
  `fs move` for local moves, a local rename for cloud moves) - the same
  technique rclone ships as --track-renames
* with deletion propagation off, a file deleted on one side stays on the
  other side ("held"): a local delete does not re-download, a remote delete
  does not re-upload; editing a held file brings it back
* conflicts (both sides changed since last sync) keep BOTH versions: the
  remote one is saved beside the local one as "name (conflict <timestamp>)",
  then the local one is uploaded as canonical
* uploads use -f create-new-revision: an existing remote file is never
  replaced - the old version stays in Proton's revision history

CLI contract (verified against Proton Drive CLI 0.9.0)
-------------------------------------------------------
* the global --json flag goes at the END of the command
* `fs download PATH... LOCALDIR -f S -d S`: single files land flat by
  basename, folder paths nest one level; mtimes are NOT preserved
* `fs upload LOCALPATH... REMOTEDIR -f S -d S`: identical content is
  auto-skipped by the CLI; -f/-d are mandatory or the CLI prompts
  (a prompt would hang the service)

Environment knobs
-----------------
  These are read from the environment, falling back to the settings file
  ~/.config/proton-sync.conf, so a manual run matches the service exactly.
  PROTON_DRIVE_BIN       path to the official CLI (default: proton-drive on PATH)
  PROTON_SYNC_LOCAL      local folder           (default ~/ProtonDrive)
  PROTON_SYNC_REMOTE     remote folder          (default /my-files)
  PROTON_SYNC_INTERVAL   remote poll seconds    (default 60)
  PROTON_SYNC_DELETES    0 = never delete (default 1 = deletions go to trash)
  PROTON_SYNC_PUSH       0 = disable the real-time cloud event listener
  PROTON_SYNC_PUSH_POLL  idle poll cap for the event stream (default 2.5)
  PROTON_SYNC_WORKERS    parallel scan calls    (default 12)
  PROTON_SYNC_RETRIES    scan attempts per folder (default 3)
  PROTON_SYNC_SUBFOLDERS comma-separated subfolders to mirror (default all)
  PROTON_SYNC_EXCLUDE    comma-separated globs to skip (e.g. "*.tmp,.cache")
  PROTON_SYNC_INOTIFY    0 = disable the inotify fast path
  PROTON_SYNC_FULL_EVERY safety full-walk seconds (default 600) - between the
                        cheap fingerprint peeks, a complete remote walk runs at
                        least this often to catch changes a size peek cannot
                        see (renames, same-byte-size edits)
  PROTON_SYNC_PEEK_AGE  max seconds a fingerprint peek may skip walking
                        (default 600, 0 = trust the peek until FULL_EVERY) -
                        Proton's aggregate size signal lags minutes behind a
                        deletion, so the peek alone would hide cloud-side
                        deletes for far too long

Telemetry: ${XDG_STATE_HOME:-~/.local/state}/proton-sync/health.json is
merge-updated at the start and end of every cycle for the GUI (never fatal).
State: ${XDG_STATE_HOME:-~/.local/state}/proton-sync/state.json (+ sync.lock)
"""

import argparse
import ctypes
import datetime
import fcntl
import fnmatch
import json
import logging
import os
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

log = logging.getLogger("proton-sync")

# ---------------------------------------------------------------- configuration


def _conf_file_values() -> dict:
    """~/.config/proton-sync.conf (the file the GUI edits; systemd loads it as
    EnvironmentFile, so inside the service the same values arrive as env vars).
    Reading it here keeps manual runs (e.g. --dry-run) consistent with the
    service. Real environment variables always win over the file."""
    out = {}
    p = Path(os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config"))) / "proton-sync.conf"
    try:
        for line in p.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                out[k.strip()] = v.strip()
    except OSError:
        pass
    return out


_FILE_CONF = _conf_file_values()


def cfg(key: str, default: str = "") -> str:
    v = os.environ.get(key)
    return v if v is not None else _FILE_CONF.get(key, default)


BIN = os.environ.get("PROTON_DRIVE_BIN") or shutil.which("proton-drive")
LOCAL_ROOT = Path(cfg("PROTON_SYNC_LOCAL", str(Path.home() / "ProtonDrive"))).expanduser()
REMOTE_ROOT = cfg("PROTON_SYNC_REMOTE", "/my-files").rstrip("/") or "/my-files"
INTERVAL = int(cfg("PROTON_SYNC_INTERVAL", "60"))
FULL_EVERY = max(60, int(cfg("PROTON_SYNC_FULL_EVERY", "600")))
# Proton's `fs size` aggregate (the cheap change signal) lags minutes behind a
# deletion - measured 80s+ still stale while `fs list` was fresh at t+5s. Bound
# how long a peek may skip walking, or cloud-side deletes would hide for far
# too long. 0 = trust the peek until FULL_EVERY (old behavior).
# With the push listener active this is only a safety net (events carry
# deletions in ~1s), so the default is generous; without push it bounds the
# worst-case delete lag (the size signal itself still lags ~80s for deletes).
PEEK_AGE = max(0, min(int(cfg("PROTON_SYNC_PEEK_AGE", "600")), FULL_EVERY))
SCAN_WORKERS = int(cfg("PROTON_SYNC_WORKERS", "12"))
SCAN_RETRIES = int(cfg("PROTON_SYNC_RETRIES", "3"))
TRANSFER_TIMEOUT = int(cfg("PROTON_SYNC_TIMEOUT", "3600"))
META_TIMEOUT = int(cfg("PROTON_SYNC_META_TIMEOUT", "120"))
PROPAGATE_DELETES = cfg("PROTON_SYNC_DELETES", "1") == "1"
INOTIFY_ENABLED = cfg("PROTON_SYNC_INOTIFY", "1") == "1"
# real-time cloud->local detection: subscribe to Proton's volume event stream
# with the official CLI's own session (read from the OS keyring). One event ->
# one targeted re-list of the affected folder instead of a full tree walk.
PUSH_ENABLED = cfg("PROTON_SYNC_PUSH", "1") == "1"
# idle poll cap for the event stream (no server-side long-poll exists; responses
# return in <1.2s). After an event we poll fast, then back off to this cap.
PUSH_POLL_MAX = max(0.5, float(cfg("PROTON_SYNC_PUSH_POLL", "2.5")))
PUSH_QUIET = 0.5        # coalescing window for push events (they are atomic
                        # server-side commits - no local write settling needed)
# selective sync: only these remote subfolders are mirrored ("" = everything)
SUBFOLDERS = [s.strip("/") for s in cfg("PROTON_SYNC_SUBFOLDERS", "").split(",") if s.strip()]
# comma-separated globs matched against rel path and file name (e.g. "*.tmp,.cache")
EXCLUDE = [p.strip() for p in cfg("PROTON_SYNC_EXCLUDE", "").split(",") if p.strip()]
# names that must never sync, anywhere on either side. Editor/OS lock-and-swap
# artifacts are skipped like real sync clients do - otherwise opening a file in
# LibreOffice/Office/vim uploads its lock file to the cloud as junk.
SKIP_NAME = re.compile(
    r"^\.Trash-\d+$"        # trash folders desktop apps leave behind
    r"|^\.~lock\..*#$"      # LibreOffice lock: .~lock.name.ext#
    r"|^~\$.*"              # MS Office lock:  ~$name.docx
    r"|^\.#.*"              # Emacs lock:      .#name
    r"|\..*\.sw[a-p]$"      # vim swap:        .name.ext.swp / .swo / ...
    r"|^\._.*"              # macOS AppleDouble junk
    r"|^\.DS_Store$"        # macOS folder metadata
)

STATE_DIR = Path(os.environ.get("XDG_STATE_HOME", str(Path.home() / ".local/state"))) / "proton-sync"
STATE_FILE = STATE_DIR / "state.json"
LOCK_FILE = STATE_DIR / "sync.lock"
HEALTH_FILE = STATE_DIR / "health.json"
# dropping this file requests an immediate cycle (the GUI's "Sync now" button)
WAKE_FILE = STATE_DIR / "wake"
SNAPSHOT_FILE = STATE_DIR / "remote-snapshot.json"
FORCE_NEXT = False          # manual wake (GUI Sync now / wake file) -> full walk
PUSH = None                 # PushListener instance (set by loop_forever when live)

# minimum spacing between cycles + how long events must be quiet first
# (tuned for a Google-Drive-like feel: a local save is on the cloud in ~15s)
MIN_CYCLE_GAP = 10
DEBOUNCE_QUIET = 4

# a side that scans completely empty while its own state remembers files is a
# sick scan (moved disk / broken login), not a user decision: hold deletions
WIPE_MIN = 5


class CliError(RuntimeError):
    pass


# -------------------------------------------------------------------- CLI glue

def cli(args, as_json=False, timeout=None):
    if not BIN:
        raise CliError("proton-drive not found (set PROTON_DRIVE_BIN or put it on PATH)")
    cmd = [BIN] + list(args)
    if as_json:
        cmd.append("--json")
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout or TRANSFER_TIMEOUT)
    except subprocess.TimeoutExpired as e:
        raise CliError(f"timeout after {e.timeout}s: {' '.join(args[:3])}...") from e
    if p.returncode != 0:
        raw = (p.stderr or p.stdout or "").strip()
        # the CLI wraps errors in '=' banners - keep the meaningful lines only
        lines = [l.strip() for l in raw.splitlines()
                 if l.strip() and set(l.strip()) != {"="}]
        msg = " / ".join(lines[:2]) if lines else f"exit {p.returncode}"
        raise CliError(f"{' '.join(args[:4])}: {msg}")
    if as_json:
        out = p.stdout.strip()
        if not out:
            return []
        try:
            return json.loads(out)
        except json.JSONDecodeError as e:
            raise CliError(f"bad JSON from CLI: {e}: {out[:200]}") from e
    return p.stdout


def cli_transfer(args):
    """Run a transfer command and echo its summary lines into the log."""
    out = cli(args)
    for line in out.splitlines():
        s = line.strip()
        if s:
            log.info("      %s", s)
    return out


def escape(component):
    """A '/' inside a node name must be escaped (CLI docs)."""
    return component.replace("/", r"\/")


def ts_ns(iso):
    if not iso:
        return None
    return int(datetime.datetime.fromisoformat(iso.replace("Z", "+00:00")).timestamp() * 1e9)


# --------------------------------------------------- selective sync + excludes

def allowed_dir(rel):
    """Selective sync: this folder is in view if it is selected, contains a
    selection, or is an ancestor of one ("" root is always in view)."""
    if not SUBFOLDERS or not rel:
        return True
    return any(rel == s or rel.startswith(s + "/") or s.startswith(rel + "/") for s in SUBFOLDERS)


def allowed_file(rel):
    if not SUBFOLDERS:
        return True
    return any(rel == s or rel.startswith(s + "/") for s in SUBFOLDERS)


def excluded(rel):
    """PROTON_SYNC_EXCLUDE globs, matched against the relative path and the name."""
    if not EXCLUDE:
        return False
    base = rel.rsplit("/", 1)[-1]
    return any(fnmatch.fnmatch(rel, pat) or fnmatch.fnmatch(base, pat) for pat in EXCLUDE)


def write_health(**kw):
    """Merge-update the telemetry file the GUI reads. Never breaks a cycle."""
    try:
        data = {}
        try:
            data = json.loads(HEALTH_FILE.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            data = {}
        data.update(kw)
        data["ts"] = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        tmp = HEALTH_FILE.with_name("health.json.tmp")
        tmp.write_text(json.dumps(data, indent=1), encoding="utf-8")
        os.replace(tmp, HEALTH_FILE)
    except Exception:  # noqa: BLE001 - telemetry must never break syncing
        log.debug("health write failed", exc_info=True)


# ----------------------------------------------------------------- remote scan

def _list_folder(rpath):
    """One `fs list --json` call with retry; returns (rpath, entries, error)."""
    last = None
    for attempt in range(SCAN_RETRIES):
        try:
            return rpath, cli(["fs", "list", rpath, "--json"], as_json=True, timeout=META_TIMEOUT), None
        except CliError as e:
            last = e
            if attempt + 1 < SCAN_RETRIES:
                time.sleep(2 * (attempt + 1))
                log.warning("listing %s failed (attempt %d/%d): %s", rpath, attempt + 1, SCAN_RETRIES, e)
    return rpath, [], last


def _children_nodes(rel, rpath, entries):
    """Parse one `fs list --json` response into direct-child nodes.
    Returns [(child_rel, child_rpath, node), ...] with scan_remote's filters
    (scope, excludes, SKIP_NAME, undecryptable names)."""
    out = []
    for e in entries:
        if "type" not in e:                       # pseudo-sections (/, /trash, ...)
            continue
        name = e.get("name", {}).get("value")
        if not e.get("name", {}).get("ok", True) or not name:
            log.warning("skipping entry with undecryptable name (uid %s)", e.get("uid", "?"))
            continue
        if SKIP_NAME.match(name):
            continue
        child_rel = f"{rel}/{name}" if rel else name
        if excluded(child_rel):
            continue
        media = e.get("mediaType") or ""
        node = {
            "uid": e.get("uid"),
            "type": "dir" if e.get("type") == "folder" else "file",
            "mtime": e.get("modificationTime"),
            "size": int((e.get("activeRevision") or {}).get("claimedSize") or 0),
            "doc": media.startswith("application/vnd.proton"),  # Docs/Sheets: not downloadable
        }
        if node["type"] == "file" and not allowed_file(child_rel):
            continue
        if node["type"] == "dir" and not allowed_dir(child_rel):
            continue
        out.append((child_rel, f"{rpath}/{escape(name)}", node))
    return out


def scan_remote(start_rel=None):
    """Walk from START_REL (default REMOTE_ROOT) in parallel -> {relpath: node}
    for that subtree only ('' = root). relpaths stay absolute from REMOTE_ROOT
    so a subtree result merges straight into the cached full snapshot."""
    tree = {}
    start_rpath = remote_path(start_rel) if start_rel else REMOTE_ROOT
    rel_of = {start_rpath: (start_rel or "")}
    visited = set()
    # each BFS round lists ALL pending folders in parallel; next_dirs accumulates
    # the child folders of the whole round (reset once per round, not per folder!)
    pending = [start_rpath]
    with ThreadPoolExecutor(max_workers=SCAN_WORKERS) as pool:
        while pending:
            next_dirs = []
            for rpath, entries, err in pool.map(_list_folder, pending):
                if err:
                    raise CliError(f"cannot list {rpath}: {err}")
                visited.add(rpath)
                rel = rel_of[rpath]
                for child_rel, child_rpath, node in _children_nodes(rel, rpath, entries):
                    tree[child_rel] = node
                    if node["type"] == "dir" and child_rpath not in visited:
                        rel_of[child_rpath] = child_rel
                        next_dirs.append(child_rpath)
            pending = next_dirs   # next BFS round lists all of these in parallel
    return tree


# ----------------------------------------------------------------- local scan

def scan_local():
    """Walk LOCAL_ROOT -> {relpath: {type, mtime_ns, size}} (files carry size)."""
    tree = {}
    for dirpath, dirnames, filenames in os.walk(LOCAL_ROOT):
        base = Path(dirpath)
        rel_base = base.relative_to(LOCAL_ROOT).as_posix()
        rel_base = "" if rel_base == "." else rel_base

        def rel_of(name):
            return f"{rel_base}/{name}" if rel_base else name

        dirnames[:] = sorted(d for d in dirnames
                             if not SKIP_NAME.match(d)
                             and not excluded(rel_of(d))
                             and allowed_dir(rel_of(d)))
        for d in dirnames:
            tree[rel_of(d)] = {"type": "dir"}
        for f in sorted(filenames):
            if SKIP_NAME.match(f) or excluded(rel_of(f)) or not allowed_file(rel_of(f)):
                continue
            p = base / f
            try:
                st = p.lstat()
            except FileNotFoundError:
                continue
            if not stat.S_ISREG(st.st_mode):
                log.debug("skip non-regular file %s", p)
                continue
            tree[rel_of(f)] = {"type": "file", "mtime_ns": st.st_mtime_ns, "size": st.st_size}
    return tree


# ------------------------------------------------------------------ state file

def load_state():
    try:
        with open(STATE_FILE, encoding="utf-8") as fh:
            return json.load(fh)
    except (FileNotFoundError, json.JSONDecodeError):
        return {}


def save_state(state):
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    tmp = STATE_FILE.with_name("state.json.tmp")
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(state, fh, indent=1, sort_keys=True)
    os.replace(tmp, STATE_FILE)


def lock():
    """Single-instance guard: refuse to overlap with another sync run."""
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    fh = open(LOCK_FILE, "w")
    try:
        fcntl.flock(fh, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        fh.close()
        raise SystemExit("another proton-sync run is already active - exiting")
    return fh  # held for the process lifetime


# ---------------------------------------------------------------- remote paths

def remote_path(rel):
    return "/".join([REMOTE_ROOT] + [escape(c) for c in rel.split("/")]) if rel else REMOTE_ROOT


def remote_parent(rel):
    if "/" not in rel:
        return REMOTE_ROOT
    return remote_path(rel.rsplit("/", 1)[0])


def ensure_remote_dir(rel, tree, created):
    """Create REL (+ missing ancestors) on the remote side if TREE lacks it."""
    if not rel or rel in tree:
        return
    parent = rel.rsplit("/", 1)[0] if "/" in rel else ""
    if parent:
        ensure_remote_dir(parent, tree, created)
    name = rel.rsplit("/", 1)[-1]        # works for top-level ("b") and nested ("a/b")
    try:
        cli(["fs", "create-folder", remote_parent(rel), escape(name)], timeout=META_TIMEOUT)
    except CliError as e:
        # defense in depth: if the scan under-reported, the folder may already
        # exist - that is fine, adopt it instead of failing the whole batch
        if "already exists" not in str(e):
            raise
        log.debug("remote folder %s already exists (scan gap?) - adopting", rel)
    tree[rel] = {"type": "dir", "mtime": None, "uid": None, "size": 0, "doc": False}
    created.add(rel)
    log.info("MKDIR  remote %s", rel)


# ----------------------------------------------------------------- decisions

def parent_of(rel):
    return rel.rsplit("/", 1)[0] if "/" in rel else ""


def detect_moves(remote, local, state, busy=frozenset()):
    """rclone --track-renames equivalent: pair a vanished path with an
    identical newcomer instead of re-transferring the bytes.

    * cloud-side moves: a path whose cloud `uid` vanished and reappeared
      elsewhere is a rename/move done in the web app or on another device
      (uids survive both `fs rename` and `fs move` - verified against the
      official CLI 0.9.0) -> ONE local rename, nothing downloaded.
    * local moves: a file that vanished locally (its cloud copy is still
      there) and an identical file (same size + mtime_ns) that appeared
      elsewhere locally is a move -> ONE server-side `fs rename`/`fs move`,
      nothing uploaded.
    * a whole folder moved at once collapses into a single folder move.

    Returns (local_moves, remote_moves): lists of (old_rel, new_rel).
    """
    remote_moves, local_moves = [], []

    # ---- cloud side: exact match on uid ----
    uid_at = {}
    for rel, node in remote.items():
        if node.get("uid"):
            uid_at.setdefault(node["uid"], []).append(rel)
    taken = set(busy)
    for old in sorted((r for r, s in state.items()
                       if s.get("r_uid") and r not in remote and r in local),
                      key=lambda p: p.count("/")):      # folders first
        if old in taken:
            continue
        cands = [c for c in uid_at.get(state[old]["r_uid"], [])
                 if c != old and c not in state and c not in local and c not in taken]
        if len(cands) != 1:
            continue
        new = cands[0]
        # a parent already moving carries its children along
        if any(new.startswith(t + "/") for t in taken):
            continue
        remote_moves.append((old, new))
        taken.add(old)
        taken.add(new)

    # ---- local side: exact match on (size, mtime_ns) ----
    key_at = {}
    for rel, node in local.items():
        if node["type"] == "file":
            key_at.setdefault((node["size"], node["mtime_ns"]), []).append(rel)
    taken = set(busy) | {p for mv in remote_moves for p in mv}
    matched = {}
    for old in sorted((r for r, s in state.items()
                       if s.get("l_mtime_ns") is not None and s.get("l_size") is not None
                       and r not in local and remote.get(r, {}).get("type") == "file"),
                      key=lambda p: p.count("/")):
        if old in taken:
            continue
        s = state[old]
        cands = [c for c in key_at.get((s["l_size"], s["l_mtime_ns"]), [])
                 if c not in state and c not in remote and c not in taken]
        if len(cands) == 1:
            matched[old] = cands[0]
            taken.add(old)
            taken.add(cands[0])

    # ---- collapse child moves into one folder move (one CLI call) ----
    consumed = set()
    old_dirs = sorted((r for r in state
                       if r not in local and remote.get(r, {}).get("type") == "dir"),
                      key=lambda p: p.count("/"))
    new_dirs = [r for r, n in local.items()
                if n["type"] == "dir" and r not in state and r not in remote]
    for po in old_dirs:
        for pn in new_dirs:
            if pn in consumed or po in consumed:
                continue
            pairs = [(o, n) for o, n in matched.items()
                     if o.startswith(po + "/") and n.startswith(pn + "/")]
            if not pairs:
                continue
            suf_o = {o[len(po) + 1:] for o, _ in pairs}
            suf_n = {n[len(pn) + 1:] for _, n in pairs}
            n_files_new = sum(1 for rel, node in local.items()
                              if node["type"] == "file" and rel.startswith(pn + "/"))
            # collapse only when the whole folder moved intact: identical
            # relative layout and no extra files in the new folder
            if suf_o != suf_n or len(pairs) != len(suf_o) or n_files_new != len(pairs):
                continue
            local_moves.append((po, pn))
            consumed.update(po)
            consumed.update(pn)
            consumed.update(o for o, _ in pairs)
            consumed.update(n for _, n in pairs)
    local_moves += [(o, n) for o, n in matched.items()
                    if o not in consumed and n not in consumed]
    return local_moves, remote_moves


def local_changed(l, s):
    return s.get("l_mtime_ns") is None or l["mtime_ns"] != s["l_mtime_ns"] or l["size"] != s.get("l_size")


def remote_changed(r, s):
    return bool(r["mtime"]) and s.get("r_mtime") != r["mtime"]


def plan_cycle(remote, local, state):
    """Returns (actions, holds). Action tuples: (kind, rel, payload)."""
    actions, holds = [], 0

    # moves first: they are the cheapest possible sync (no bytes move) and
    # must pre-empt the delete+recreate interpretation of the same paths.
    # A folder move also covers its whole subtree on both trees - its children
    # must not be planned individually (that would re-create them elsewhere).
    local_moves, remote_moves = detect_moves(remote, local, state)
    for old, new in local_moves:
        actions.append(("mv_remote", new, (old, new)))
    for old, new in remote_moves:
        actions.append(("mv_local", new, (old, new)))
    moved = set()
    for old, new in local_moves + remote_moves:
        moved.update((old, new))
        moved.update(p for p in set(remote) | set(local)
                     if p.startswith(old + "/") or p.startswith(new + "/"))

    # wipe guard: one side scanned empty while state remembers real content -
    # that is a sick scan (moved disk, broken login, wrong path), never a user
    # decision. Deletions from that side are held this cycle.
    state_local = sum(1 for s in state.values() if s.get("l_mtime_ns") is not None)
    state_remote = sum(1 for s in state.values() if s.get("r_mtime"))
    local_dead = not any(n["type"] == "file" for n in local.values()) \
        and state_local >= WIPE_MIN and any(n["type"] == "file" for n in remote.values())
    remote_dead = not any(n["type"] == "file" for n in remote.values()) \
        and state_remote >= WIPE_MIN and any(n["type"] == "file" for n in local.values())
    if local_dead or remote_dead:
        log.error("wipe guard: %s side scanned empty while state remembers files - "
                  "holding deletions this cycle",
                  "local" if local_dead else "cloud")

    trashed = set()                                     # folders sent to cloud trash this cycle
    for rel in sorted(set(remote) | set(local), key=lambda p: (p.count("/"), p)):
        if rel in moved:
            continue
        if trashed and any(rel.startswith(p + "/") for p in trashed):
            continue                                    # child rides along inside its trashed folder
        r, l, s = remote.get(rel), local.get(rel), state.get(rel)

        if r and r.get("doc"):                          # Proton Doc/Sheet: CLI cannot download
            holds += 1
            log.debug("hold %s (Proton Doc/Sheet)", rel)
            continue

        # ---- only on the remote side -----------------------------------
        if r and not l:
            if s is None:                               # never seen -> new remotely
                actions.append(("dl", rel, r))
            elif (r["type"] == "dir" and s.get("r_uid") == r.get("uid")
                  and PROPAGATE_DELETES and not local_dead):
                # folders carry no local fingerprint (state only remembers
                # their cloud identity), but a record with THIS uid proves
                # the folder was mirrored here before: cloud copy present +
                # folder gone locally = the user deleted it -> propagate to
                # cloud trash (recoverable). A folder re-created in the cloud
                # carries a different uid and falls through to the mirror-down
                # below; a wiped local tree is held by local_dead above.
                actions.append(("trash_remote", rel, r))
                trashed.add(rel)
            elif PROPAGATE_DELETES and not local_dead and s.get("l_mtime_ns") is not None:
                actions.append(("trash_remote", rel, r))    # deleted locally, deletes on
            elif remote_changed(r, s):
                actions.append(("dl", rel, r))          # new revision -> bring it over
            else:
                holds += 1                              # respect the local delete
            continue

        # ---- only on the local side ------------------------------------
        if l and not r:
            if s is None:                               # never seen -> new locally
                actions.append(("up", rel, l))
            elif PROPAGATE_DELETES and not remote_dead and s.get("r_mtime"):
                actions.append(("trash_local", rel, l))     # deleted remotely, deletes on
            elif l["type"] == "file" and local_changed(l, s):
                actions.append(("up", rel, l))          # edited -> (re)create remotely
            else:
                holds += 1                              # respect the remote delete
            continue

        # ---- present on both sides -------------------------------------
        if r and l:
            if l["type"] == "file":
                if s is None:                           # first meeting
                    if r["size"] == l["size"]:
                        holds += 1                      # identical: nothing to do
                    elif l["mtime_ns"] > (ts_ns(r["mtime"]) or 0):
                        actions.append(("up", rel, l))
                    else:
                        actions.append(("dl", rel, r))
                else:
                    lc, rc = local_changed(l, s), remote_changed(r, s)
                    if lc and rc:
                        if r["size"] == l["size"]:
                            holds += 1                  # same bytes, different clocks
                        else:
                            actions.append(("conflict", rel, (r, l)))
                    elif lc:
                        actions.append(("up", rel, l))
                    elif rc:
                        actions.append(("dl", rel, r))
                    else:
                        holds += 1
            else:
                holds += 1                              # folders: presence handled above
    return actions, holds


# -------------------------------------------------------------------- actions

def download_folder(rel):
    """Download a whole remote subtree whose local folder does not exist yet."""
    parent_local = LOCAL_ROOT / (rel.rsplit("/", 1)[0] if "/" in rel else "")
    parent_local.mkdir(parents=True, exist_ok=True)
    cli_transfer(["fs", "download", remote_path(rel), str(parent_local), "-f", "remove", "-d", "merge"])
    log.info("DOWNLOAD dir  %s/ (whole subtree)", rel)


def download_files(parent, rels):
    target = LOCAL_ROOT / parent if parent else LOCAL_ROOT
    target.mkdir(parents=True, exist_ok=True)
    cli_transfer(["fs", "download", *[remote_path(r) for r in rels], str(target),
                  "-f", "remove", "-d", "merge"])
    for r in rels:
        log.info("DOWNLOAD %s", r)


def upload_batch(parent, rels, tree, created):
    ensure_remote_dir(parent, tree, created)
    cli_transfer(["fs", "upload", "-f", "create-new-revision", "-d", "merge",
                  *[str(LOCAL_ROOT / r) for r in rels], remote_parent(rels[0])])
    for r in rels:
        log.info("UPLOAD %s", r)


def resolve_conflict(rel, r, l):
    """Keep BOTH versions: remote saved as 'name (conflict <ts>)', local canonical."""
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    p = LOCAL_ROOT / rel
    conflict_name = f"{p.stem} (conflict {stamp}){p.suffix}"
    with tempfile.TemporaryDirectory(dir=str(STATE_DIR)) as tmp:
        cli_transfer(["fs", "download", remote_path(rel), tmp, "-f", "remove", "-d", "merge"])
        got = Path(tmp) / p.name
        if not got.exists():
            candidates = [x for x in Path(tmp).iterdir() if x.is_file()]
            if not candidates:
                raise CliError("conflict download produced no file")
            got = candidates[0]
        target = p.with_name(conflict_name)
        shutil.move(str(got), str(target))
        ns = ts_ns(r.get("mtime"))
        if ns:
            os.utime(target, ns=(ns, ns))
    log.warning("CONFLICT %s: remote kept as %r", rel, conflict_name)
    cli_transfer(["fs", "upload", "-f", "create-new-revision", "-d", "merge",
                  str(p), remote_parent(rel)])
    log.info("UPLOAD %s (canonical after conflict)", rel)
    # the conflict copy is a brand-new local file; it goes up on the next cycle


def trash_local_path(rel):
    p = LOCAL_ROOT / rel
    if not p.exists():
        return
    if shutil.which("gio"):
        r = subprocess.run(["gio", "trash", str(p)], check=False, capture_output=True, text=True)
        if not p.exists():
            return
        if r.returncode != 0:
            # e.g. /tmp or another system-internal mount refuses; fall back to
            # a recoverable engine bin rather than losing the file
            log.warning("gio trash refused %s (%s) - keeping it in the engine bin",
                        rel, (r.stderr or "").strip().splitlines()[-1] if r.stderr else "no reason")
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    bin_dir = STATE_DIR / "trash" / stamp
    bin_dir.mkdir(parents=True, exist_ok=True)
    shutil.move(str(p), str(bin_dir / p.name))
    log.info("kept deleted file in %s (restore by moving it back)", bin_dir / p.name)


def apply_mtimes(paths, remote):
    """Downloads do not preserve timestamps - restore the remote modificationTime."""
    for rel in paths:
        node = remote.get(rel)
        if not node or node["type"] != "file" or not node.get("mtime"):
            continue
        p = LOCAL_ROOT / rel
        if p.is_file():
            ns = ts_ns(node["mtime"])
            try:
                os.utime(p, ns=(ns, ns))
            except OSError as e:
                log.debug("utime %s failed: %s", rel, e)


def execute(actions, remote, local):
    """Run the plan. Returns (failed_rels, downloaded_rels, did_upload)."""
    failed, downloaded, did_upload = set(), set(), False
    tree = dict(remote)
    created = set()

    def guarded(label, fn, *a):
        try:
            fn(*a)
        except CliError as e:
            log.error("FAILED %s: %s", label, e)
            failed.add(label)
        except Exception as e:        # one bad path must never wedge the whole sync
            log.exception("FAILED %s: %s", label, e)
            failed.add(label)

    # 1) server-side moves (rename/move tracking) - cheapest action first:
    #    one CLI call relocates the file, no bytes are transferred
    for kind, rel, payload in actions:
        if kind == "mv_remote":
            old, new = payload
            try:
                ensure_remote_dir(parent_of(new), tree, created)
                if parent_of(old) == parent_of(new):
                    cli(["fs", "rename", remote_path(old), escape(new.rsplit("/", 1)[-1])],
                        timeout=META_TIMEOUT)
                else:
                    cli(["fs", "move", remote_path(old), remote_parent(new)], timeout=META_TIMEOUT)
                    if old.rsplit("/", 1)[-1] != new.rsplit("/", 1)[-1]:
                        # `fs move` keeps the name - finish the rename too
                        cli(["fs", "rename",
                             "/".join([remote_parent(new), escape(old.rsplit("/", 1)[-1])]),
                             escape(new.rsplit("/", 1)[-1])], timeout=META_TIMEOUT)
                log.info("MOVE remote %s -> %s (server-side, no upload)", old, new)
                did_upload = True                     # remote changed: refresh snapshot
            except CliError as e:
                log.error("FAILED move remote %s -> %s: %s", old, new, e)
                failed.add(old)
                failed.add(new)

    # 1b) local moves: a rename/move in the cloud or on another device is
    #     mirrored here with a plain filesystem move - nothing re-downloaded
    for kind, rel, payload in actions:
        if kind == "mv_local":
            old, new = payload
            try:
                src, dst = LOCAL_ROOT / old, LOCAL_ROOT / new
                dst.parent.mkdir(parents=True, exist_ok=True)
                if src.exists():
                    shutil.move(str(src), str(dst))
                    log.info("MOVE local %s -> %s (renamed in the cloud, no download)",
                             old, new)
                else:                                 # already moved by the user
                    log.debug("local move %s -> %s: source already gone", old, new)
            except OSError as e:
                log.error("FAILED move local %s -> %s: %s", old, new, e)
                failed.add(old)
                failed.add(new)

    # 2) new local folders -> create remotely (files inside follow via upload)
    for kind, rel, payload in actions:
        if kind == "up" and payload["type"] == "dir":
            guarded(rel, ensure_remote_dir, rel, tree, created)
            did_upload = True

    # 2) uploads, grouped by parent folder
    by_parent = {}
    for kind, rel, payload in actions:
        if kind == "up" and payload["type"] == "file":
            by_parent.setdefault(rel.rsplit("/", 1)[0] if "/" in rel else "", []).append(rel)
    for parent, rels in sorted(by_parent.items()):
        try:
            upload_batch(parent, rels, tree, created)
            did_upload = True
        except CliError as e:
            log.error("FAILED upload batch (%d path(s), parent %r): %s", len(rels), parent, e)
            failed.update(rels)

    # 3) whole missing folders -> one CLI call per subtree (fast first mirror)
    covered = set()
    for kind, rel, payload in actions:
        if kind == "dl" and rel in covered:
            continue
        if kind == "dl" and payload["type"] == "dir" and rel not in local:
            before = len(failed)
            guarded(rel, download_folder, rel)
            if len(failed) == before:
                subtree = {k for k in remote if k == rel or k.startswith(rel + "/")}
                covered.update(subtree)
                downloaded.update(subtree)

    # 4) per-file downloads into existing folders, grouped by remote parent
    by_parent = {}
    for kind, rel, payload in actions:
        if kind == "dl" and rel not in covered and payload["type"] == "file":
            by_parent.setdefault(rel.rsplit("/", 1)[0] if "/" in rel else "", []).append(rel)
    for parent, rels in sorted(by_parent.items()):
        todo = [r for r in rels if r not in covered]
        if not todo:
            continue
        try:
            download_files(parent, todo)
            downloaded.update(todo)
        except CliError as e:
            log.error("FAILED download batch (%d path(s), parent %r): %s", len(todo), parent, e)
            failed.update(todo)

    # 5) conflicts: keep both versions
    for kind, rel, payload in actions:
        if kind == "conflict":
            before = len(failed)
            guarded(rel, resolve_conflict, rel, payload[0], payload[1])
            if len(failed) == before:
                downloaded.add(rel)   # canonical stays local; remote copy was fetched
                did_upload = True

    # 6) deletions (on by default; always to trash, never permanent)
    if PROPAGATE_DELETES:
        for kind, rel, payload in actions:
            if kind == "trash_remote":
                try:
                    cli(["fs", "trash", remote_path(rel)], timeout=META_TIMEOUT)
                    log.info("TRASH remote %s (was deleted locally)", rel)
                    did_upload = True               # refresh the remote snapshot too
                except CliError as e:
                    log.error("FAILED trash remote %s: %s", rel, e)
                    failed.add(rel)
            elif kind == "trash_local":
                try:
                    trash_local_path(rel)
                    log.info("TRASH local  %s (was deleted remotely)", rel)
                except OSError as e:
                    log.error("FAILED trash local %s: %s", rel, e)
                    failed.add(rel)

    return failed, downloaded, did_upload


# ------------------------------------------------------------------- state

def refresh_state(remote, local, old_state, failed, acted, planned_local=None, downloaded=None):
    """
    Record the observed reality for every trustworthy path.
    * failed paths keep their previous record so the exact same action retries
    * a both-sides file records the PLAN-TIME local fingerprint, not the
      post-execute rescan: a local edit that lands mid-cycle (while the batch
      runs) must never be stamped as already-synced, or the engine would think
      it is done and silently never upload it. Only a file we just downloaded
      records the fresh local, since that new content is the reality we hold.
    * one-sided paths with no history and no action get NO record yet (they
      must be planned again next cycle - e.g. a fresh conflict copy)
    * a side that vanished is remembered: with deletions ON the last-known
      values are kept (a deletion held this cycle - wipe guard, stale
      snapshot - still fires next cycle); with deletions OFF the side is
      nulled so an old deletion never fires if the user enables them later
    """
    downloaded = downloaded or set()
    if planned_local is None:
        planned_local = local
    new_state = {}
    for rel in sorted(set(remote) | set(local)):
        o = old_state.get(rel)
        if rel in failed:
            if o is not None:
                new_state[rel] = o
            continue
        r, l = remote.get(rel), local.get(rel)
        if r and l:
            entry = {}
            if l["type"] == "file":
                # fresh local for a completed download; otherwise the plan-time
                # view, so an edit made mid-cycle is re-planned next cycle
                # instead of being lost as "already synced"
                src = l if rel in downloaded else (planned_local.get(rel) or l)
                if src["type"] == "file":
                    entry["l_mtime_ns"], entry["l_size"] = src["mtime_ns"], src["size"]
            entry["r_mtime"], entry["r_uid"] = r["mtime"], r["uid"]
            new_state[rel] = entry
            continue
        # one side only: holds, completed deletions, partial oddities
        if o is None and rel not in acted:
            continue
        entry = {}
        if l and l["type"] == "file":
            entry["l_mtime_ns"], entry["l_size"] = l["mtime_ns"], l["size"]
        if r:
            entry["r_mtime"], entry["r_uid"] = r["mtime"], r["uid"]
        if o is not None:
            if not l and o.get("l_mtime_ns") is not None:
                if PROPAGATE_DELETES:
                    # keep the last-known values: a deletion HELD this cycle
                    # (wipe guard, stale remote snapshot) must still be
                    # actionable on the next sane cycle
                    entry["l_mtime_ns"], entry["l_size"] = o["l_mtime_ns"], o.get("l_size")
                else:
                    entry["l_mtime_ns"], entry["l_size"] = None, None
            if not r and o.get("r_mtime") is not None:
                if PROPAGATE_DELETES:
                    entry["r_mtime"], entry["r_uid"] = o["r_mtime"], o.get("r_uid")
                else:
                    entry["r_mtime"], entry["r_uid"] = None, None
        if entry:
            new_state[rel] = entry
    return new_state


# ------------------------------------------------- smart cadence (peek vs walk)

def remote_fingerprint():
    """Cheap deep-change signal: ONE server-side `fs size` call (~2.5s, 0.1s CPU).

    Returns [total_bytes, descendants] or None when unavailable. Any addition,
    deletion or size-changing edit anywhere in the tree changes these numbers;
    renames and same-byte-size edits do not (the safety walk catches those).
    """
    try:
        p = subprocess.run([BIN, "fs", "size", REMOTE_ROOT],
                           capture_output=True, text=True, timeout=META_TIMEOUT)
    except (subprocess.TimeoutExpired, OSError):
        return None
    if p.returncode != 0:
        return None
    m_size = re.search(r"size:\s*(\d+)", p.stdout)
    m_n = re.search(r"numberOfDescendants:\s*(\d+)", p.stdout)
    if not (m_size and m_n):
        return None
    return [int(m_size.group(1)), int(m_n.group(1))]


def walk_signature():
    """Effective settings that change what a remote walk returns - a cached
    snapshot is only trusted while this matches (selection/exclusion edits)."""
    return "|".join([REMOTE_ROOT,
                     cfg("PROTON_SYNC_SUBFOLDERS", ""),
                     cfg("PROTON_SYNC_EXCLUDE", "")])


def load_snapshot():
    try:
        data = json.loads(SNAPSHOT_FILE.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(data, dict) or data.get("sig") != walk_signature():
        return None
    if not isinstance(data.get("tree"), dict) or not isinstance(data.get("fp"), list):
        return None
    return data


def snapshot_tree(data):
    """The remote view from a cached walk: {rel: {uid, type, mtime, size, doc}}."""
    tree = {}
    for rel, node in data["tree"].items():
        if isinstance(node, dict) and node.get("uid") and node.get("type") in ("dir", "file"):
            tree[rel] = node
    return tree


def save_snapshot(tree, fp):
    if not isinstance(fp, list):
        return
    try:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        payload = {"sig": walk_signature(), "fp": fp, "ts": time.time(), "tree": tree}
        tmp = SNAPSHOT_FILE.with_name("remote-snapshot.json.tmp")
        tmp.write_text(json.dumps(payload, separators=(",", ":")), encoding="utf-8")
        os.replace(tmp, SNAPSHOT_FILE)
    except Exception:  # noqa: BLE001 - a cached-walk optimization must never break sync
        log.debug("snapshot save failed", exc_info=True)


# ------------------------------------------------------------------ push events
#
# Real-time cloud->local detection. The official CLI keeps a per-volume event
# cursor in ~/.local/share/proton-drive-cli/events.json and re-consumes the
# stream on every invocation; we subscribe to the SAME stream with the CLI's
# own session (read from the OS Secret Service - jeepney is pure python and
# bundled, so no host packages are needed) and share the cursor file so both
# consumers stay on one chain:
#   * after every delivered batch we advance the cursor on disk (atomic replace)
#   * on 400 "Invalid ID" we re-read the cursor (someone else consumed a link
#     first) and retry
# A Link event names the changed node (LinkID) and its folder (ParentLinkID);
# ids map to snapshot entries as "<volumeId>~<LinkID>". Measured: server-side
# commit -> event delivered in ~1.0s; the engine then re-lists ONLY the
# affected folder instead of walking the whole tree.

PUSH_EVENTS_FILE = (Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
                    / "proton-drive-cli" / "events.json")
PUSH_SECRET_LABEL = "ch.proton.drive/drive-sdk-cli/auth-session"
PUSH_API = "https://drive-api.proton.me"


def read_cli_session():
    """The official CLI's auth session from the OS Secret Service.

    Returns {"accessToken","uid"} or None - never fatal: without it the
    engine simply keeps polling as before. Tokens are handled in memory only.
    """
    try:
        from jeepney import DBusAddress, new_method_call
        from jeepney.io.blocking import open_dbus_connection
    except ImportError:
        log.info("push: jeepney not bundled - interval polling only")
        return None
    conn = None
    try:
        def addr(path, interface):
            return DBusAddress(path, bus_name="org.freedesktop.secrets",
                               interface=interface)

        def call(a, member, signature, body):
            reply = conn.send_and_get_reply(new_method_call(a, member, signature, body))
            if reply.header.message_type.name == "error":
                raise RuntimeError(f"{member}: {reply.body}")
            return reply.body

        conn = open_dbus_connection()
        paths, _ = call(addr("/org/freedesktop/secrets", "org.freedesktop.Secret.Service"),
                        "SearchItems", "a{ss}", ({},))
        item = None
        for p in paths:
            label = call(addr(p, "org.freedesktop.DBus.Properties"), "Get", "ss",
                         ("org.freedesktop.Secret.Item", "Label"))[0]
            if isinstance(label, tuple):
                label = label[1]
            label = str(label)
            if label == PUSH_SECRET_LABEL or ("proton" in label
                                              and label.endswith("/auth-session")):
                item = p
                break
        if not item:
            log.info("push: official CLI session not found in the keyring")
            return None
        out = call(addr("/org/freedesktop/secrets", "org.freedesktop.Secret.Service"),
                   "OpenSession", "sv", ("plain", ("s", "")))
        raw = bytes(call(addr(item, "org.freedesktop.Secret.Item"), "GetSecret", "o",
                         (out[1],))[0][2])
        wrapper = json.loads(raw)
        inner = wrapper.get("session") or {}
        if isinstance(inner, str):
            inner = json.loads(inner)
        return {"accessToken": inner.get("accessToken"), "uid": inner.get("uid")}
    except Exception as e:  # noqa: BLE001 - push must never take the engine down
        log.info("push: keyring read failed (%s)", e)
        return None
    finally:
        if conn is not None:
            try:
                conn.close()
            except Exception:  # noqa: BLE001
                pass


def push_cursor_read(volume_id):
    try:
        data = json.loads(PUSH_EVENTS_FILE.read_text(encoding="utf-8"))
        return data["drive"][volume_id]["lastEventId"]
    except (OSError, KeyError, TypeError, json.JSONDecodeError):
        return None


def push_cursor_write(volume_id, cid):
    """Merge our advanced cursor into the CLI's events.json (atomic replace) so
    the CLI continues the same chain instead of hitting "Invalid ID"."""
    try:
        data = json.loads(PUSH_EVENTS_FILE.read_text(encoding="utf-8"))
        data.setdefault("drive", {}).setdefault(volume_id, {})["lastEventId"] = cid
        tmp = PUSH_EVENTS_FILE.with_name("events.json.tmp")
        tmp.write_text(json.dumps(data, indent=2), encoding="utf-8")
        os.replace(tmp, PUSH_EVENTS_FILE)
        return True
    except Exception:  # noqa: BLE001 - a lost race just costs a re-read later
        log.debug("push: cursor write-back failed", exc_info=True)
        return False


class PushListener(threading.Thread):
    """Long-lived subscriber for one Drive volume's event stream.

    Every Link event records the changed node's PARENT folder id; the main
    loop drains those into one targeted re-list (resolve_push_scopes +
    apply_scope) instead of a full recursive walk. Startup problems raise in
    __init__ - the caller logs it and degrades to plain interval polling.
    """

    def __init__(self, trigger):
        super().__init__(name="push", daemon=True)
        self.trigger = trigger                 # shared Event: wake the main loop
        self._lock = threading.Lock()
        self.pending = set()                   # bare parent LinkIDs awaiting refresh
        self._stop = False
        info = cli(["fs", "info", REMOTE_ROOT, "--json"], as_json=True, timeout=META_TIMEOUT)
        uid = info.get("uid") or ""
        if "~" not in uid:
            raise RuntimeError("cannot resolve remote root uid")
        self.root_uid = uid
        self.volume_id = info.get("treeEventScopeId") or uid.split("~", 1)[0]
        self.cursor = push_cursor_read(self.volume_id)
        if not self.cursor:
            raise RuntimeError(f"no event cursor for volume {self.volume_id}")
        self.session = read_cli_session()
        if not (self.session and self.session.get("accessToken")):
            raise RuntimeError("no CLI session in keyring")
        log.info("push: subscribed to volume %s", self.volume_id)

    def drain(self):
        with self._lock:
            got, self.pending = self.pending, set()
        return got

    @staticmethod
    def _get(url, token, uid):
        req = urllib.request.Request(url, headers={
            "Authorization": f"Bearer {token}",
            "x-pm-uid": str(uid or ""),
            "x-pm-appversion": "Other",
            "Accept": "application/json",
            "User-Agent": "proton-drive-sync",
        })
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.read()

    def _record(self, events):
        parents = {(ev.get("Link") or {}).get("ParentLinkID")
                   for ev in events if isinstance(ev, dict)}
        parents = {p for p in parents if p}
        if parents:
            with self._lock:
                self.pending |= parents
            self.trigger.set()

    def run(self):
        delay = 0.4            # fast after activity, grows to PUSH_POLL_MAX when idle
        auth_fails = 0         # consecutive 401s: > 6 means the session is gone
        stale_cursor = 0       # consecutive 400s with an unchanged disk cursor
        sess_check = time.time()
        while not self._stop:
            # the CLI rotates its access token - pick up the refreshed one cheaply
            if time.time() - sess_check > 60:
                fresh = read_cli_session()
                if fresh and fresh.get("accessToken"):
                    self.session = fresh
                sess_check = time.time()
            url = (f"{PUSH_API}/drive/v2/volumes/{self.volume_id}/events/{self.cursor}")
            t0 = time.time()
            try:
                body = self._get(url, self.session.get("accessToken"),
                                 self.session.get("uid"))
            except urllib.error.HTTPError as e:
                if e.code == 400:
                    disk = push_cursor_read(self.volume_id)
                    if disk and disk != self.cursor:
                        log.info("push: cursor consumed elsewhere - re-adopting")
                        self.cursor = disk
                        delay, stale_cursor = 0.4, 0
                    else:
                        stale_cursor += 1
                        delay = min(max(delay, 0.8) * 2, 30)
                        if stale_cursor == 10:
                            log.warning("push: cursor rejected repeatedly - "
                                        "waiting for the CLI to refresh it")
                        elif stale_cursor and stale_cursor % 30 == 0:
                            time.sleep(30)   # long idle, the CLI's next run rewrites it
                elif e.code == 401:
                    auth_fails += 1
                    fresh = read_cli_session()
                    if fresh:
                        self.session = fresh
                        auth_fails = 0
                    delay = min(delay * 2, 15)
                    if auth_fails >= 6:
                        log.error("push: session rejected repeatedly - push disabled "
                                  "(interval polling continues)")
                        return
                elif e.code == 429:
                    delay = min(max(delay, 5) * 2, 120)
                    log.warning("push: rate limited - backing off %.0fs", delay)
                else:
                    delay = min(delay * 2, 30)
            except Exception as e:  # noqa: BLE001 - network hiccups are normal
                delay = min(delay * 2, 30)
                log.debug("push: %s: %s (backoff %.1fs)", type(e).__name__, e, delay)
            else:
                try:
                    data = json.loads(body)
                except json.JSONDecodeError:
                    data = {}
                events = data.get("Events") or []
                if events:
                    self._record(events)
                    log.info("push: %d event(s) received", len(events))
                    delay = 0.4
                else:
                    delay = min(delay * 1.6, PUSH_POLL_MAX)
                next_id = data.get("EventID")
                if isinstance(next_id, str) and next_id and next_id != self.cursor:
                    self.cursor = next_id
                    push_cursor_write(self.volume_id, next_id)
                    delay = 0.4
                delay = max(delay, time.time() - t0)   # responses already cost ~1s
            time.sleep(delay)

    def stop(self):
        self._stop = True


def resolve_push_scopes(pending, tree, volume_id, root_uid):
    """Parent LinkIDs from events -> snapshot rel paths to re-list.

    Unknown parents are skipped on purpose: they are either outside the
    synced subtree (no-op for us) or a folder chain not observed yet - the
    parent's own refresh event picks it up, and peek/PEEK_AGE backstop the
    rare race. Returns (sorted rels, unknown count)."""
    if not pending:
        return [], 0
    index = {node["uid"]: rel for rel, node in tree.items() if node.get("uid")}
    root_lid = root_uid.split("~", 1)[1]
    rels, unknown = set(), 0
    for lid in pending:
        if lid == root_lid:
            rels.add("")                     # volume root: not scopable
        else:
            rel = index.get(f"{volume_id}~{lid}")
            if rel:
                rels.add(rel)
            else:
                unknown += 1
    return sorted(rels), unknown


def apply_scope(remote, rel):
    """Refresh REL's DIRECT children inside the cached REMOTE view (in place).

    A volume event names the changed node's immediate parent, and a deeper
    change arrives as its own event - so ONE `fs list` of REL is enough;
    only children new to the snapshot get descended (a folder that just
    appeared may already carry a subtree, e.g. a move).

    Raises CliError on transient failures so the caller falls back to a full
    walk - a partial refresh must never leave the snapshot silently
    inconsistent. A definitively vanished target drops its subtree
    (plan_cycle then trash-deletes locally, like any cloud deletion)."""
    if not rel:
        raise CliError("root refresh must run as a full walk")
    rpath = remote_path(rel)
    _, entries, err = _list_folder(rpath)
    if err is not None:
        if re.search(r"not found|404|no such", str(err), re.I):
            for k in [k for k in remote if k == rel or k.startswith(rel + "/")]:
                del remote[k]
            log.info("push: %s vanished - dropped from snapshot", rel)
            return "dropped"
        raise CliError(f"cannot list {rpath}: {err}")

    prefix = rel + "/"
    fresh = {}                                   # child name -> (child_rel, child_rpath, node)
    for child_rel, child_rpath, node in _children_nodes(rel, rpath, entries):
        fresh[child_rel.rsplit("/", 1)[-1]] = (child_rel, child_rpath, node)
    old = {}                                     # direct child name -> snapshot key
    for k in remote:
        if k.startswith(prefix) and "/" not in k[len(prefix):]:
            old[k[len(prefix):]] = k
    fresh_uids = {t[2].get("uid") for t in fresh.values() if t[2].get("uid")}
    old_by_uid = {remote[rk].get("uid"): name for name, rk in old.items()
                  if remote[rk].get("uid")}

    descend, dropped, renamed = [], 0, 0
    vanished_new = set()                          # new dirs that vanished mid-descend
    for name, (child_rel, child_rpath, node) in fresh.items():
        rk = prefix + name
        uid = node.get("uid")
        prev_name = old_by_uid.get(uid) if uid else None
        if prev_name == name:
            remote[rk] = node                     # same link: adopt new mtime/size
        elif prev_name is not None:               # renamed: rebase the subtree keys
            old_rk = prefix + prev_name
            for k in [k for k in remote if k == old_rk or k.startswith(old_rk + "/")]:
                remote[prefix + name + k[len(old_rk):]] = remote.pop(k)
            remote[rk] = node
            renamed += 1
        else:
            if rk in remote:                      # same name, new link (recreated)
                for k in [k for k in remote if k == rk or k.startswith(rk + "/")]:
                    del remote[k]
                dropped += 1
            remote[rk] = node
            if node["type"] == "dir":
                descend.append(child_rel)
    for name, rk in old.items():                  # vanished children
        if rk in remote and remote[rk].get("uid") not in fresh_uids:
            for k in [k for k in remote if k == rk or k.startswith(rk + "/")]:
                del remote[k]
            dropped += 1
    for child_rel in descend:
        try:
            remote.update(scan_remote(child_rel))
        except CliError as e:
            m = str(e)
            if m.startswith(f"cannot list {remote_path(child_rel)}:") and \
                    re.search(r"not found|404|no such", m, re.I):
                remote.pop(child_rel, None)       # vanished again in the meantime
                vanished_new.add(child_rel.rsplit("/", 1)[-1])
            else:
                raise                             # transient -> full walk fallback
    # defensive: a pathological merge (two simultaneous renames swapping names
    # in one listing) must never leave a fresh child unrepresented - bail out
    # to a full walk instead of shipping a wrong snapshot
    for name in fresh:
        if name not in vanished_new and prefix + name not in remote:
            raise CliError(f"scope merge incomplete for {rpath} - full walk required")
    log.info("push: refreshed %s (%d children, %d new subtree(s), %d dropped, %d renamed)",
             rel, len(fresh), len(descend), dropped, renamed)
    return "updated"


# ------------------------------------------------------------------ one cycle

def run_once(dry_run=False, force_full=False, push_pending=None):
    LOCAL_ROOT.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    if not dry_run:
        write_health(status="syncing",
                     started=datetime.datetime.now().astimezone().isoformat(timespec="seconds"))
    log.info("scan: remote %s + local %s", REMOTE_ROOT, LOCAL_ROOT)

    # smart cadence: push events decide first (targeted re-list of one folder),
    # otherwise a ~2.5s fingerprint peek decides if a full walk is needed
    mode = "full"
    snap = fp = None
    scoped = None
    if not (dry_run or force_full):
        snap = load_snapshot()
        push_handled = False
        if snap is not None and push_pending and PUSH is not None:
            rels, unknown = resolve_push_scopes(push_pending, snapshot_tree(snap),
                                                PUSH.volume_id, PUSH.root_uid)
            if unknown:
                log.debug("push: %d event(s) unresolvable in snapshot", unknown)
            if "" in rels:
                log.info("push: change at the volume root - falling back to a full walk")
            elif rels:
                log.info("push: %d folder change(s) in scope: %s",
                         len(push_pending), ", ".join(rels[:5]) + ("..." if len(rels) > 5 else ""))
                mode, scoped, push_handled = "scope", rels, True
            else:
                # every event's parent is outside the snapshot - typically
                # out-of-scope cloud activity (phone uploads at the volume root,
                # other folders): adopt the fresh fingerprint so it stops
                # scheduling wasted scope walks. The one alternative - an
                # in-scope parent not observed yet (new folder chain race, see
                # resolve_push_scopes) - stays bounded by the PEEK_AGE walk.
                fp = remote_fingerprint()
                if fp is not None:
                    save_snapshot(snapshot_tree(snap), fp)
                log.info("push: %d event(s) outside the synced scope - adopted", len(push_pending))
                mode, push_handled = "keep", True
        if not push_handled:
            fp = remote_fingerprint()
            if fp is None:
                log.info("fingerprint unavailable - full walk")
            elif snap is None:
                log.info("no usable snapshot - full walk")
            elif fp != snap.get("fp"):
                log.info("peek: cloud changed - full walk")
            elif PEEK_AGE and time.time() - float(snap.get("ts") or 0) > PEEK_AGE:
                log.info("peek stale (> %ds) - full walk (cloud deletes hide behind the lagging size signal)",
                         PEEK_AGE)
            elif time.time() - float(snap.get("ts") or 0) > FULL_EVERY:
                log.info("safety walk due (> %ds)", FULL_EVERY)
            else:
                mode = "keep"

    if mode == "keep":
        remote = snapshot_tree(snap)
        log.debug("peek: cloud unchanged - reusing snapshot (%d entries)", len(remote))
    elif mode == "scope":
        remote = snapshot_tree(snap)
        try:
            for rel in scoped:
                apply_scope(remote, rel)
        except CliError as e:
            log.info("push: scoped refresh failed (%s) - full walk instead", str(e)[:140])
            mode = "full"
        else:
            fp_after = remote_fingerprint()
            if fp_after is not None:
                save_snapshot(remote, fp_after)
            else:
                log.info("push: fingerprint unavailable - snapshot left for the next peek")
    if mode == "full":
        remote = scan_remote()
        fp_after = remote_fingerprint()
        if fp_after is None and fp is not None:
            # transient size failure right after an expensive walk: cache the
            # walk we already paid for using the pre-walk value (conservative
            # - if the cloud changed mid-walk the next peek just says "changed"
            # and we walk again)
            fp_after = fp
            log.info("fingerprint refresh failed - caching the walk anyway")
        if fp_after is not None:
            save_snapshot(remote, fp_after)
        else:
            log.info("walk not cached (no fingerprint) - next cycle will walk again")

    local = scan_local()
    state = load_state()
    log.info("scan: %d remote / %d local / %d state records in %.0fs",
             len(remote), len(local), len(state), time.time() - t0)

    actions, holds = plan_cycle(remote, local, state)
    log.info("plan: %d action(s), %d hold(s)%s", len(actions), holds, "  [DRY RUN]" if dry_run else "")
    for kind, rel, _ in actions:
        log.info("  %-13s %s", kind.upper(), rel)

    if dry_run:
        return True

    if not actions:
        save_state(refresh_state(remote, local, state, set(), set()))
        log.info("cycle done: in sync")
        write_health(status="ok", cycle_seconds=round(time.time() - t0, 1),
                     actions=0, holds=holds, failed=0, uploaded=0, downloaded=0, conflicts=0,
                     error="", mode=mode,
                     remote_entries=len(remote), local_entries=len(local),
                     state_records=len(state), interval=INTERVAL)
        return True

    failed, downloaded, did_upload = execute(actions, remote, local)
    if downloaded:
        apply_mtimes(downloaded, remote)   # before the local rescan, so state records them

    local2 = scan_local()
    remote2 = scan_remote() if did_upload else remote
    if did_upload:
        fp2 = remote_fingerprint()          # keep the cached walk in step with reality
        if fp2 is not None:
            save_snapshot(remote2, fp2)
    saved = refresh_state(remote2, local2, state, failed, {a[1] for a in actions},
                          planned_local=local, downloaded=downloaded)
    save_state(saved)
    uploaded_n = sum(1 for kind, rel, _ in actions if kind == "up" and rel not in failed)
    conflicts_n = sum(1 for kind, rel, _ in actions if kind == "conflict" and rel not in failed)
    log.info("cycle done: %d failed, state=%d records", len(failed), len(saved))
    write_health(status="ok", cycle_seconds=round(time.time() - t0, 1),
                 actions=len(actions), holds=holds, failed=len(failed),
                 uploaded=uploaded_n, downloaded=len(downloaded), conflicts=conflicts_n,
                 remote_entries=len(remote), local_entries=len(local),
                 state_records=len(saved), interval=INTERVAL, error="", mode=mode)
    return not failed


# --------------------------------------------------------------- inotify wake

IN_CLOSE_WRITE = 0x00000008
IN_MODIFY = 0x00000002
IN_CREATE = 0x00000100
IN_DELETE = 0x00000200
IN_MOVED_FROM = 0x00000040
IN_MOVED_TO = 0x00000080
IN_ISDIR = 0x40000000
MASK = (IN_CLOSE_WRITE | IN_MODIFY | IN_CREATE | IN_DELETE | IN_MOVED_FROM | IN_MOVED_TO)


class InotifyWatcher(threading.Thread):
    """Watches LOCAL_ROOT recursively via libc inotify (stdlib ctypes only).

    Fires `wake` on any relevant change; new subfolders are watched on the fly.
    A failure here is never fatal - the interval poll still works.
    """

    def __init__(self, root, wake):
        super().__init__(name="inotify", daemon=True)
        self.root = Path(root)
        self.wake = wake
        self.wd2path = {}
        self.libc = ctypes.CDLL("libc.so.6", use_errno=True)
        self.libc.inotify_init1.argtypes = [ctypes.c_int]
        self.libc.inotify_init1.restype = ctypes.c_int
        self.libc.inotify_add_watch.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32]
        self.libc.inotify_add_watch.restype = ctypes.c_int
        self.fd = self.libc.inotify_init1(0)
        if self.fd < 0:
            raise OSError(ctypes.get_errno(), "inotify_init1 failed")
        for dirpath, dirnames, _ in os.walk(self.root):
            dirnames[:] = [d for d in dirnames if not SKIP_NAME.match(d)]
            self._add(Path(dirpath))

    def _add(self, path):
        wd = self.libc.inotify_add_watch(self.fd, os.fsencode(str(path)), MASK)
        if wd >= 0:
            self.wd2path[wd] = path
        else:
            log.warning("inotify: cannot watch %s (errno %s)", path, ctypes.get_errno())

    def run(self):
        while True:
            try:
                data = os.read(self.fd, 65536)
            except OSError:
                return
            off = 0
            while off + 16 <= len(data):
                wd, mask, _cookie, length = struct.unpack("iIII", data[off:off + 16])
                off += 16
                name = data[off:off + length].split(b"\0", 1)[0].decode(errors="replace")
                off += length
                base = self.wd2path.get(wd)
                if base is None:
                    continue
                if name and (mask & IN_ISDIR) and (mask & (IN_CREATE | IN_MOVED_TO)):
                    if not SKIP_NAME.match(name):
                        self._add(base / name)   # watch newly created folders too
                self.wake.set()


# ------------------------------------------------------------------------ main

def loop_forever():
    global FORCE_NEXT, PUSH
    wake = threading.Event()
    if INOTIFY_ENABLED:
        try:
            InotifyWatcher(LOCAL_ROOT, wake).start()
            log.info("inotify fast path active (local changes sync within ~%ds)", DEBOUNCE_QUIET + MIN_CYCLE_GAP)
        except OSError as e:
            log.info("inotify unavailable (%s) - falling back to %ds polling only", e, INTERVAL)
    else:
        log.info("inotify disabled - polling every %ds", INTERVAL)

    if PUSH_ENABLED:
        try:
            PUSH = PushListener(wake)
            PUSH.start()
            log.info("push listener active (cloud changes detected in ~1s)")
        except Exception as e:  # noqa: BLE001 - degraded mode must keep running
            PUSH = None
            log.info("push unavailable (%s) - falling back to %ds interval polling", e, INTERVAL)
    else:
        log.info("push disabled (PROTON_SYNC_PUSH=0) - %ds interval polling", INTERVAL)

    def drain_push():
        return PUSH.drain() if PUSH is not None else set()

    push_pending = None
    while True:
        started = time.time()
        force = FORCE_NEXT
        FORCE_NEXT = False
        try:
            run_once(force_full=force, push_pending=push_pending)
        except CliError as e:
            log.error("cycle failed: %s (next try in %ds)", e, INTERVAL)
            write_health(status="error", error=str(e)[:300], interval=INTERVAL)
        except Exception as e:  # noqa: BLE001 - keep the loop alive, report it
            log.exception("cycle crashed (next try in %ds)", INTERVAL)
            write_health(status="error", error=str(e)[:300], interval=INTERVAL)
        push_pending = None   # events are hints; peek/PEEK_AGE are the backstop
        wake.clear()

        # sleep until the interval expires, a local change, or a push event
        deadline = time.time() + INTERVAL
        write_health(next_poll=datetime.datetime.fromtimestamp(deadline)
                     .astimezone().isoformat(timespec="seconds"), interval=INTERVAL)
        while time.time() < deadline:
            more = drain_push()
            if more:
                push_pending = (push_pending or set()) | more
                log.info("push: %d folder change event(s) queued", len(more))
            triggered = bool(more) or wake.wait(timeout=1)
            if WAKE_FILE.exists():
                WAKE_FILE.unlink(missing_ok=True)
                log.info("manual sync requested (wake)")
                FORCE_NEXT = True
                push_pending = None     # manual wake runs a full walk anyway
                triggered = True
            if triggered:
                # debounce: local writes need settling time; push events are
                # atomic server-side commits and coalesce much faster
                quiet = time.time() + (PUSH_QUIET if push_pending else DEBOUNCE_QUIET)
                while time.time() < quiet:
                    wake.clear()
                    if wake.wait(timeout=0.5):
                        quiet = time.time() + DEBOUNCE_QUIET
                    more = drain_push()
                    if more:
                        push_pending = (push_pending or set()) | more
                        quiet = time.time() + PUSH_QUIET
                gap = MIN_CYCLE_GAP - (time.time() - started)
                if gap > 0:
                    time.sleep(gap)
                break


def main():
    global LOCAL_ROOT, REMOTE_ROOT

    ap = argparse.ArgumentParser(description="Two-way Proton Drive folder sync (official CLI only).")
    ap.add_argument("--once", action="store_true", help="run one cycle and exit (default)")
    ap.add_argument("--loop", action="store_true", help="run forever (systemd service mode)")
    ap.add_argument("--dry-run", action="store_true", help="scan and print the plan, change nothing")
    ap.add_argument("--local", default=str(LOCAL_ROOT), help="local sync folder (default ~/ProtonDrive)")
    ap.add_argument("--remote", default=REMOTE_ROOT, help="remote folder (default /my-files)")
    args = ap.parse_args()
    LOCAL_ROOT = Path(args.local).expanduser()
    REMOTE_ROOT = args.remote.rstrip("/") or "/my-files"

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)-7s %(message)s",
                        datefmt="%H:%M:%S")

    if not BIN:
        log.error("official CLI not found - set PROTON_DRIVE_BIN or put proton-drive on PATH")
        return 2
    log.info("CLI: %s | local=%s remote=%s deletes=%s interval=%ds",
             BIN, LOCAL_ROOT, REMOTE_ROOT, "on" if PROPAGATE_DELETES else "off", INTERVAL)

    fh = lock()
    try:
        if args.dry_run:
            run_once(dry_run=True)
        elif args.loop:
            loop_forever()
        else:
            return 0 if run_once() else 1
    finally:
        fcntl.flock(fh, fcntl.LOCK_UN)
        fh.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
