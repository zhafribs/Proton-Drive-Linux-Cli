# Planner parity harness

`scripts/proton-sync.py` stays the reference until the Rust engine has been
shadow-run. This harness feeds identical trees to both planners and compares
the normalized action list and hold count.

```sh
python3 tests/parity/run.py
python3 tests/parity/run.py --filter move --verbose
```

It builds `proton-engine` first, imports the real Python module, and runs each
fixture in `cases.py` through both. A failure is a Rust porting bug unless the
case itself is wrong; add cases for every behaviour change, not just the easy
ones.

Fixtures use the on-disk shapes:

- remote: `{"uid", "type", "mtime", "size", "doc"}`
- local: `{"type": "dir"}` or `{"type": "file", "mtime_ns", "size"}`
- state: any of `l_size`, `l_mtime_ns`, `r_mtime`, `r_uid`

`WIPE_MIN` and `PROPAGATE_DELETES` are set per case, because the Python
planner reads them as module globals.
