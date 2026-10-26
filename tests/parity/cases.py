"""Planner fixtures shared by the Python/Rust parity harness.

The Python engine remains the reference. Every case here is a small,
self-contained set of the three inputs `plan_cycle` takes: the remote tree,
the local tree, and the state file. Keep the two implementations' outputs
identical; when a case fails, fix the Rust port, not the fixture.
"""

R1 = "2025-01-01T00:00:00.000Z"
R2 = "2025-02-01T00:00:00.000Z"
L1 = 1735689600000000000  # 2025-01-01T00:00:00Z in ns
L2 = 1738368000000000000  # 2025-02-01T00:00:00Z in ns


def rfile(uid, size, mtime=R1, doc=False):
    return {"uid": uid, "type": "file", "mtime": mtime, "size": size, "doc": doc}


def rdir(uid):
    return {"uid": uid, "type": "dir", "mtime": None, "size": 0, "doc": False}


def lfile(size, mtime=L1):
    return {"type": "file", "mtime_ns": mtime, "size": size}


def ldir():
    return {"type": "dir"}


def st(local=None, remote=None):
    """One state row. Pass (size, mtime_ns) and/or (r_mtime, r_uid)."""
    row = {}
    if local is not None:
        row["l_size"], row["l_mtime_ns"] = local
    if remote is not None:
        row["r_mtime"], row["r_uid"] = remote
    return row


CASES = [
    {
        "name": "basic",
        "remote": {
            "both-down.txt": rfile("u1", 10, R2),
            "both-up.txt": rfile("u2", 10, R1),
            "dir": rdir("d1"),
            "remote-new.txt": rfile("u3", 1, R1),
        },
        "local": {
            "both-down.txt": lfile(11, L1),
            "both-up.txt": lfile(11, L2),
            "dir": ldir(),
            "dir/local-new.txt": lfile(2, L1),
        },
        "state": {},
    },
    {
        "name": "edits-conflicts",
        "remote": {
            "conflict.txt": rfile("u1", 4, R2),
            "local-edit.txt": rfile("u2", 5, R1),
            "remote-edit.txt": rfile("u3", 6, R2),
            "same-size.txt": rfile("u4", 4, R2),
        },
        "local": {
            "conflict.txt": lfile(5, L2),
            "local-edit.txt": lfile(5, L2),
            "remote-edit.txt": lfile(5, L1),
            "same-size.txt": lfile(4, L2),
        },
        "state": {
            "conflict.txt": st((3, L1), (R1, "u1")),
            "local-edit.txt": st((5, L1), (R1, "u2")),
            "remote-edit.txt": st((5, L1), (R1, "u3")),
            "same-size.txt": st((3, L1), (R1, "u4")),
        },
    },
    {
        "name": "deletes-on",
        "remote": {"deleted-locally.txt": rfile("u1", 1, R1)},
        "local": {"deleted-remotely.txt": lfile(1, L1)},
        "state": {
            "deleted-locally.txt": st((1, L1), (R1, "u1")),
            "deleted-remotely.txt": st((1, L1), (R1, "u2")),
        },
    },
    {
        "name": "deletes-off",
        "propagate_deletes": False,
        "remote": {"deleted-locally.txt": rfile("u1", 1, R1)},
        "local": {"deleted-remotely.txt": lfile(1, L1)},
        "state": {
            "deleted-locally.txt": st((1, L1), (R1, "u1")),
            "deleted-remotely.txt": st((1, L1), (R1, "u2")),
        },
    },
    {
        "name": "local-move",
        "remote": {"old.txt": rfile("u1", 3, R1)},
        "local": {"new.txt": lfile(3, L1)},
        "state": {"old.txt": st((3, L1), (R1, "u1"))},
    },
    {
        "name": "remote-move",
        "remote": {"new.txt": rfile("u1", 3, R1)},
        "local": {"old.txt": lfile(3, L1)},
        "state": {"old.txt": st((3, L1), (R1, "u1"))},
    },
    {
        "name": "folder-move",
        "remote": {"old": rdir("d1"), "old/a.txt": rfile("u1", 3, R1)},
        "local": {"new": ldir(), "new/a.txt": lfile(3, L1)},
        "state": {
            "old": st(remote=("", "d1")),
            "old/a.txt": st((3, L1), (R1, "u1")),
        },
    },
    {
        "name": "wipe-guard",
        "remote": {f"f{i}.txt": rfile(f"u{i}", 1, R1) for i in range(5)},
        "local": {},
        "state": {
            f"f{i}.txt": st((1, L1), (R1, f"u{i}")) for i in range(5)
        },
    },
    {
        "name": "docs-held",
        "remote": {
            "doc-as-file": rfile("u1", 0, R1, doc=True),
            "doc-remote-new": rfile("u2", 0, R1, doc=True),
        },
        "local": {"doc-as-file": lfile(0, L1)},
        "state": {},
    },
    {
        "name": "remote-delete-local-edit-on",
        "remote": {},
        "local": {"a.txt": lfile(5, L2)},
        "state": {"a.txt": st((3, L1), (R1, "u1"))},
    },
    {
        "name": "remote-delete-local-edit-off",
        "propagate_deletes": False,
        "remote": {},
        "local": {"a.txt": lfile(5, L2)},
        "state": {"a.txt": st((3, L1), (R1, "u1"))},
    },
    {
        "name": "first-meeting",
        "remote": {
            "equal.txt": rfile("u1", 4, R1),
            "local-newer.txt": rfile("u2", 5, R1),
            "remote-newer.txt": rfile("u3", 6, R2),
        },
        "local": {
            "equal.txt": lfile(4, L1),
            "local-newer.txt": lfile(6, L2),
            "remote-newer.txt": lfile(5, L1),
        },
        "state": {},
    },
    {
        "name": "remote-dir-delete",
        "remote": {},
        "local": {"dir": ldir(), "dir/a.txt": lfile(1, L1)},
        "state": {
            "dir": st(remote=("", "d1")),
            "dir/a.txt": st((1, L1), (R1, "u1")),
        },
    },
]


REFRESH_CASES = [
    {
        "name": "both-sides-plan-time",
        "remote": {"a": rfile("u", 3, R1)},
        "local": {"a": lfile(3, 2)},
        "planned_local": {"a": lfile(3, 1)},
    },
    {
        "name": "download-fresh-local",
        "remote": {"a": rfile("u", 3, R1)},
        "local": {"a": lfile(3, 2)},
        "planned_local": {"a": lfile(3, 1)},
        "downloaded": ["a"],
    },
    {
        "name": "preserve-held",
        "remote": {"edit.txt": rfile("u1", 3, R1)},
        "local": {"edit.txt": lfile(4, 2)},
        "old_state": {"edit.txt": st((3, 1), (R1, "u1"))},
        "preserve": ["edit.txt"],
    },
    {
        "name": "failed-keeps-old",
        "remote": {"a": rfile("u", 3, R1)},
        "local": {"a": lfile(4, 2)},
        "old_state": {"a": st((3, 1), (R1, "u"))},
        "failed": ["a"],
    },
    {
        "name": "deletes-on-keeps-vanished",
        "remote": {"a": rfile("u", 3, R1)},
        "local": {},
        "old_state": {"a": st((3, 1), (R1, "u"))},
    },
    {
        "name": "deletes-off-nulls-vanished",
        "propagate_deletes": False,
        "remote": {"a": rfile("u", 3, R1)},
        "local": {},
        "old_state": {"a": st((3, 1), (R1, "u"))},
    },
    {
        "name": "remote-delete-keeps-remote",
        "remote": {},
        "local": {"a": lfile(3, 1)},
        "old_state": {"a": st((3, 1), (R1, "u"))},
    },
    {
        "name": "one-sided-no-action-skipped",
        "remote": {"a": rfile("u", 3, R1)},
        "local": {},
        "acted": [],
    },
    {
        "name": "one-sided-acted-recorded",
        "remote": {"a": rfile("u", 3, R1)},
        "local": {},
        "acted": ["a"],
    },
    {
        "name": "both-sides-dir",
        "remote": {"d": rdir("d1")},
        "local": {"d": ldir()},
    },
    {
        "name": "one-sided-dir-no-record",
        "remote": {},
        "local": {"d": ldir()},
        "acted": ["d"],
    },
]

