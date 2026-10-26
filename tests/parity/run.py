#!/usr/bin/env python3
"""Diff the Rust planner against the Python engine on shared fixtures.

This is intentionally a script, not a cargo test: it imports the real
`scripts/proton-sync.py` and needs a built engine binary. Run it before any
cutover change:

    python3 tests/parity/run.py
    python3 tests/parity/run.py --filter moves --verbose

The fixture inputs live in `cases.py`; both implementations receive exactly
the same dictionaries, and Python's `PROPAGATE_DELETES`/`WIPE_MIN` globals are
set per case (the planner is otherwise pure).
"""

import argparse
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))

from cases import CASES, REFRESH_CASES  # noqa: E402


def load_reference():
    spec = importlib.util.spec_from_file_location(
        "proton_sync_parity", ROOT / "scripts" / "proton-sync.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def normalize(actions):
    out = []
    for kind, rel, payload in actions:
        if kind in ("mv_remote", "mv_local"):
            out.append({"kind": kind, "old": payload[0], "new": payload[1]})
        elif kind == "conflict":
            out.append(
                {"kind": kind, "rel": rel, "remote": payload[0], "local": payload[1]}
            )
        else:
            out.append({"kind": kind, "rel": rel, "node": payload})
    return out


def normalize_state(state):
    """Drop explicit nulls: Rust omits unknown sides, Python writes null."""
    return {
        rel: {key: value for key, value in row.items() if value is not None}
        for rel, row in state.items()
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--filter", default="", help="only case names containing this")
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    if not args.no_build:
        subprocess.run(
            ["cargo", "build", "-q", "-p", "proton-engine"], cwd=ROOT, check=True
        )
    binary = ROOT / "gui" / "target" / "debug" / "proton-engine"
    if not binary.exists():
        sys.exit(f"missing {binary}; run without --no-build")

    reference = load_reference()
    failures = 0
    ran = 0
    with tempfile.TemporaryDirectory(prefix="pd-parity-") as tmp:
        for case in CASES:
            if args.filter and args.filter not in case["name"]:
                continue
            ran += 1
            fixture = {
                "name": case["name"],
                "propagate_deletes": case.get("propagate_deletes", True),
                "wipe_min": case.get("wipe_min", 5),
                "remote": case.get("remote", {}),
                "local": case.get("local", {}),
                "state": case.get("state", {}),
            }
            path = pathlib.Path(tmp) / f"{case['name']}.json"
            path.write_text(json.dumps(fixture, indent=2), encoding="utf-8")

            reference.PROPAGATE_DELETES = fixture["propagate_deletes"]
            reference.WIPE_MIN = fixture["wipe_min"]
            actions, holds = reference.plan_cycle(
                fixture["remote"], fixture["local"], fixture["state"]
            )
            expected = {"actions": normalize(actions), "holds": holds}

            proc = subprocess.run(
                [str(binary), "plan", "--fixture", str(path)],
                text=True,
                capture_output=True,
            )
            if proc.returncode != 0:
                failures += 1
                print(f"FAIL {case['name']}: engine exited {proc.returncode}")
                print(proc.stderr.strip())
                continue
            got_full = json.loads(proc.stdout)
            got = {"actions": got_full["actions"], "holds": got_full["holds"]}

            if got == expected:
                if args.verbose:
                    print(f"ok   {case['name']}")
                continue
            failures += 1
            print(f"FAIL {case['name']}")
            print("  python:", json.dumps(expected, ensure_ascii=False))
            print("  rust:  ", json.dumps(got, ensure_ascii=False))

    if failures:
        print(f"\n{failures}/{ran} plan case(s) differ")
        return 1
    print(f"{ran} plan parity case(s) match")

    # ---- refresh_state parity ----
    refresh_failures = 0
    refresh_ran = 0
    with tempfile.TemporaryDirectory(prefix="pd-refresh-parity-") as tmp:
        for case in REFRESH_CASES:
            if args.filter and args.filter not in case["name"]:
                continue
            refresh_ran += 1
            fixture = {
                "name": case["name"],
                "propagate_deletes": case.get("propagate_deletes", True),
                "remote": case.get("remote", {}),
                "local": case.get("local", {}),
                "old_state": case.get("old_state", {}),
                "failed": case.get("failed", []),
                "acted": case.get("acted", []),
                "planned_local": case.get("planned_local"),
                "downloaded": case.get("downloaded", []),
                "preserve": case.get("preserve", []),
            }
            path = pathlib.Path(tmp) / f"{case['name']}.json"
            path.write_text(json.dumps(fixture, indent=2), encoding="utf-8")

            reference.PROPAGATE_DELETES = fixture["propagate_deletes"]
            saved = reference.refresh_state(
                fixture["remote"],
                fixture["local"],
                fixture["old_state"],
                set(fixture["failed"]),
                set(fixture["acted"]),
                planned_local=fixture["planned_local"],
                downloaded=set(fixture["downloaded"]),
                preserve=set(fixture["preserve"]),
            )
            expected = normalize_state(saved)

            proc = subprocess.run(
                [str(binary), "refresh", "--fixture", str(path)],
                text=True,
                capture_output=True,
            )
            if proc.returncode != 0:
                refresh_failures += 1
                print(f"FAIL refresh/{case['name']}: engine exited {proc.returncode}")
                print(proc.stderr.strip())
                continue
            got = normalize_state(json.loads(proc.stdout))

            if got == expected:
                if args.verbose:
                    print(f"ok   refresh/{case['name']}")
                continue
            refresh_failures += 1
            print(f"FAIL refresh/{case['name']}")
            print("  python:", json.dumps(expected, ensure_ascii=False, sort_keys=True))
            print("  rust:  ", json.dumps(got, ensure_ascii=False, sort_keys=True))

    if refresh_failures:
        print(f"\n{refresh_failures}/{refresh_ran} refresh case(s) differ")
        return 1
    print(f"{refresh_ran} refresh parity case(s) match")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
