# Local scan benchmark

`scripts/bench-scan.sh [files]` creates a disposable tree, builds the release
engine and runs `scan-local` three times.

Baseline (x86_64 dev machine, release build, warm page cache):

| Files | Run 1 | Run 2 | Run 3 |
|---:|---:|---:|---:|
| 5,000 | 18 ms | 7 ms | 7 ms |
| 10,000 | 14 ms | 14 ms | 13 ms |

This measures local metadata scanning only: no network, no hashing, no
transfer. It is a regression signal, not a whole-cycle performance claim.
