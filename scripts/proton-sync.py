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
   a cheap server-side size-peek (`fs size`) skips re-listing when nothing
   changed, and PROTON_SYNC_PEEK_AGE bounds how long that peek may be
   trusted - Proton's size aggregate can lag 80s+ behind a deletion, so the
   bound is what keeps cloud-side deletions from hiding (Proton exposes no
   change-feed API, the same ceiling rclone hits on this provider)

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
  PROTON_SYNC_INTERVAL   remote poll seconds    (default 300)
  PROTON_SYNC_DELETES    0 = never delete (default 1 = deletions go to trash)
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
                        (default 120, 0 = trust the peek until FULL_EVERY) -
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
INTERVAL = int(cfg("PROTON_SYNC_INTERVAL", "300"))
FULL_EVERY = max(60, int(cfg("PROTON_SYNC_FULL_EVERY", "600")))
# Proton's `fs size` aggregate (the cheap change signal) lags minutes behind a
# deletion - measured 80s+ still stale while `fs list` was fresh at t+5s. Bound
# how long a peek may skip walking, or cloud-side deletes would hide for far
# too long. 0 = trust the peek until FULL_EVERY (old behavior).
PEEK_AGE = max(0, min(int(cfg("PROTON_SYNC_PEEK_AGE", "120")), FULL_EVERY))
SCAN_WORKERS = int(cfg("PROTON_SYNC_WORKERS", "12"))
SCAN_RETRIES = int(cfg("PROTON_SYNC_RETRIES", "3"))
TRANSFER_TIMEOUT = int(cfg("PROTON_SYNC_TIMEOUT", "3600"))
META_TIMEOUT = int(cfg("PROTON_SYNC_META_TIMEOUT", "120"))
PROPAGATE_DELETES = cfg("PROTON_SYNC_DELETES", "1") == "1"
INOTIFY_ENABLED = cfg("PROTON_SYNC_INOTIFY", "1") == "1"
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


def scan_remote():
    """Walk REMOTE_ROOT in parallel -> {relpath: {type, mtime, size, uid, doc}} ('' = root)."""
    tree = {}
    rel_of = {REMOTE_ROOT: ""}
    visited = set()
    # each BFS round lists ALL pending folders in parallel; next_dirs accumulates
    # the child folders of the whole round (reset once per round, not per folder!)
    pending = [REMOTE_ROOT]
    with ThreadPoolExecutor(max_workers=SCAN_WORKERS) as pool:
        while pending:
            next_dirs = []
            for rpath, entries, err in pool.map(_list_folder, pending):
                if err:
                    raise CliError(f"cannot list {rpath}: {err}")
                visited.add(rpath)
                rel = rel_of[rpath]
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
                    tree[child_rel] = node
                    if node["type"] == "dir":
                        child_rpath = f"{rpath}/{escape(name)}"
                        if child_rpath not in visited:
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

    for rel in sorted(set(remote) | set(local), key=lambda p: (p.count("/"), p)):
        if rel in moved:
            continue
        r, l, s = remote.get(rel), local.get(rel), state.get(rel)

        if r and r.get("doc"):                          # Proton Doc/Sheet: CLI cannot download
            holds += 1
            log.debug("hold %s (Proton Doc/Sheet)", rel)
            continue

        # ---- only on the remote side -----------------------------------
        if r and not l:
            if s is None:                               # never seen -> new remotely
                actions.append(("dl", rel, r))
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


# ------------------------------------------------------------------ one cycle

def run_once(dry_run=False, force_full=False):
    LOCAL_ROOT.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    if not dry_run:
        write_health(status="syncing",
                     started=datetime.datetime.now().astimezone().isoformat(timespec="seconds"))
    log.info("scan: remote %s + local %s", REMOTE_ROOT, LOCAL_ROOT)

    # smart cadence: a ~2.5s fingerprint peek decides if a full walk is needed
    mode = "full"
    snap = fp = None
    if not (dry_run or force_full):
        snap = load_snapshot()
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
    else:
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
    global FORCE_NEXT
    wake = threading.Event()
    if INOTIFY_ENABLED:
        try:
            InotifyWatcher(LOCAL_ROOT, wake).start()
            log.info("inotify fast path active (local changes sync within ~%ds)", DEBOUNCE_QUIET + MIN_CYCLE_GAP)
        except OSError as e:
            log.info("inotify unavailable (%s) - falling back to %ds polling only", e, INTERVAL)
    else:
        log.info("inotify disabled - polling every %ds", INTERVAL)

    while True:
        started = time.time()
        force = FORCE_NEXT
        FORCE_NEXT = False
        try:
            run_once(force_full=force)
        except CliError as e:
            log.error("cycle failed: %s (next try in %ds)", e, INTERVAL)
            write_health(status="error", error=str(e)[:300], interval=INTERVAL)
        except Exception as e:  # noqa: BLE001 - keep the loop alive, report it
            log.exception("cycle crashed (next try in %ds)", INTERVAL)
            write_health(status="error", error=str(e)[:300], interval=INTERVAL)
        wake.clear()

        # sleep until the interval expires, a local change, or a wake request
        deadline = time.time() + INTERVAL
        write_health(next_poll=datetime.datetime.fromtimestamp(deadline)
                     .astimezone().isoformat(timespec="seconds"), interval=INTERVAL)
        while time.time() < deadline:
            triggered = wake.wait(timeout=1)
            if WAKE_FILE.exists():
                WAKE_FILE.unlink(missing_ok=True)
                log.info("manual sync requested (wake)")
                FORCE_NEXT = True
                triggered = True
            if triggered:
                # debounce: wait until events stop for a while, respect min spacing
                quiet = time.time() + DEBOUNCE_QUIET
                while time.time() < quiet:
                    wake.clear()
                    if wake.wait(timeout=2):
                        quiet = time.time() + DEBOUNCE_QUIET
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
