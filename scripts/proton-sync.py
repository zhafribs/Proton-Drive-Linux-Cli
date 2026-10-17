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
   the interval only bounds how often *remote* changes are noticed

Safety defaults
---------------
* never deletes on its own (opt in: PROTON_SYNC_DELETES=1 - local deletions
  go to Proton's trash, remote deletions to the desktop trash: recoverable)
* a file deleted on one side stays on the other side ("held"): a local delete
  does not re-download, a remote delete does not re-upload; editing a held
  file brings it back
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
  PROTON_SYNC_DELETES    1 = propagate deletions (default 0 = never delete)
  PROTON_SYNC_WORKERS    parallel scan calls    (default 12)
  PROTON_SYNC_RETRIES    scan attempts per folder (default 3)
  PROTON_SYNC_SUBFOLDERS comma-separated subfolders to mirror (default all)
  PROTON_SYNC_EXCLUDE    comma-separated globs to skip (e.g. "*.tmp,.cache")
  PROTON_SYNC_INOTIFY    0 = disable the inotify fast path

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
SCAN_WORKERS = int(cfg("PROTON_SYNC_WORKERS", "12"))
SCAN_RETRIES = int(cfg("PROTON_SYNC_RETRIES", "3"))
TRANSFER_TIMEOUT = int(cfg("PROTON_SYNC_TIMEOUT", "3600"))
META_TIMEOUT = int(cfg("PROTON_SYNC_META_TIMEOUT", "120"))
PROPAGATE_DELETES = cfg("PROTON_SYNC_DELETES", "0") == "1"
INOTIFY_ENABLED = cfg("PROTON_SYNC_INOTIFY", "1") == "1"
# selective sync: only these remote subfolders are mirrored ("" = everything)
SUBFOLDERS = [s.strip("/") for s in cfg("PROTON_SYNC_SUBFOLDERS", "").split(",") if s.strip()]
# comma-separated globs matched against rel path and file name (e.g. "*.tmp,.cache")
EXCLUDE = [p.strip() for p in cfg("PROTON_SYNC_EXCLUDE", "").split(",") if p.strip()]
SKIP_NAME = re.compile(r"^\.Trash-\d+$")   # trash folders desktop apps leave behind

STATE_DIR = Path(os.environ.get("XDG_STATE_HOME", str(Path.home() / ".local/state"))) / "proton-sync"
STATE_FILE = STATE_DIR / "state.json"
LOCK_FILE = STATE_DIR / "sync.lock"
HEALTH_FILE = STATE_DIR / "health.json"
# dropping this file requests an immediate cycle (the GUI's "Sync now" button)
WAKE_FILE = STATE_DIR / "wake"

# minimum spacing between cycles + how long events must be quiet first
MIN_CYCLE_GAP = 30
DEBOUNCE_QUIET = 12


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
    name = rel.rsplit("/", 1)[1]
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

def local_changed(l, s):
    return s.get("l_mtime_ns") is None or l["mtime_ns"] != s["l_mtime_ns"] or l["size"] != s.get("l_size")


def remote_changed(r, s):
    return bool(r["mtime"]) and s.get("r_mtime") != r["mtime"]


def plan_cycle(remote, local, state):
    """Returns (actions, holds). Action tuples: (kind, rel, payload)."""
    actions, holds = [], 0
    for rel in sorted(set(remote) | set(local), key=lambda p: (p.count("/"), p)):
        r, l, s = remote.get(rel), local.get(rel), state.get(rel)

        if r and r.get("doc"):                          # Proton Doc/Sheet: CLI cannot download
            holds += 1
            log.debug("hold %s (Proton Doc/Sheet)", rel)
            continue

        # ---- only on the remote side -----------------------------------
        if r and not l:
            if s is None:                               # never seen -> new remotely
                actions.append(("dl", rel, r))
            elif PROPAGATE_DELETES and s.get("l_mtime_ns") is not None:
                actions.append(("trash_remote", rel, r))    # deleted locally, deletes opted in
            elif remote_changed(r, s):
                actions.append(("dl", rel, r))          # new revision -> bring it over
            else:
                holds += 1                              # respect the local delete
            continue

        # ---- only on the local side ------------------------------------
        if l and not r:
            if s is None:                               # never seen -> new locally
                actions.append(("up", rel, l))
            elif PROPAGATE_DELETES and s.get("r_mtime"):
                actions.append(("trash_local", rel, l))     # deleted remotely, deletes opted in
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
        subprocess.run(["gio", "trash", str(p)], check=False, capture_output=True)
        if not p.exists():
            return
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    bin_dir = STATE_DIR / "trash" / stamp
    bin_dir.mkdir(parents=True, exist_ok=True)
    shutil.move(str(p), str(bin_dir / p.name))


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

    # 1) new local folders -> create remotely (files inside follow via upload)
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

    # 6) opt-in deletions (off by default)
    if PROPAGATE_DELETES:
        for kind, rel, payload in actions:
            if kind == "trash_remote":
                try:
                    cli(["fs", "trash", remote_path(rel)], timeout=META_TIMEOUT)
                    log.info("TRASH remote %s (was deleted locally)", rel)
                except CliError as e:
                    log.error("FAILED trash remote %s: %s", rel, e)
                    failed.add(rel)
            elif kind == "trash_local":
                trash_local_path(rel)
                log.info("TRASH local  %s (was deleted remotely)", rel)

    return failed, downloaded, did_upload


# ------------------------------------------------------------------- state

def refresh_state(remote, local, old_state, failed, acted):
    """
    Record the observed reality for every trustworthy path.
    * failed paths keep their previous record so the exact same action retries
    * one-sided paths with no history and no action get NO record yet (they
      must be planned again next cycle - e.g. a fresh conflict copy)
    * a side that vanished gets an explicit null so deletions are respected
    """
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
                entry["l_mtime_ns"], entry["l_size"] = l["mtime_ns"], l["size"]
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
                entry["l_mtime_ns"], entry["l_size"] = None, None
            if not r and o.get("r_mtime") is not None:
                entry["r_mtime"], entry["r_uid"] = None, None
        if entry:
            new_state[rel] = entry
    return new_state


# ------------------------------------------------------------------ one cycle

def run_once(dry_run=False):
    LOCAL_ROOT.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    if not dry_run:
        write_health(status="syncing",
                     started=datetime.datetime.now().astimezone().isoformat(timespec="seconds"))
    log.info("scan: remote %s + local %s", REMOTE_ROOT, LOCAL_ROOT)
    remote = scan_remote()
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
                     error="",
                     remote_entries=len(remote), local_entries=len(local),
                     state_records=len(state), interval=INTERVAL)
        return True

    failed, downloaded, did_upload = execute(actions, remote, local)
    if downloaded:
        apply_mtimes(downloaded, remote)   # before the local rescan, so state records them

    local2 = scan_local()
    remote2 = scan_remote() if did_upload else remote
    saved = refresh_state(remote2, local2, state, failed, {a[1] for a in actions})
    save_state(saved)
    uploaded_n = sum(1 for kind, rel, _ in actions if kind == "up" and rel not in failed)
    conflicts_n = sum(1 for kind, rel, _ in actions if kind == "conflict" and rel not in failed)
    log.info("cycle done: %d failed, state=%d records", len(failed), len(saved))
    write_health(status="ok", cycle_seconds=round(time.time() - t0, 1),
                 actions=len(actions), holds=holds, failed=len(failed),
                 uploaded=uploaded_n, downloaded=len(downloaded), conflicts=conflicts_n,
                 remote_entries=len(remote), local_entries=len(local),
                 state_records=len(saved), interval=INTERVAL, error="")
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
        try:
            run_once()
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
