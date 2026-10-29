# Upstream requests

Feature requests for the official Proton Drive CLI that would unlock work in
this app. Nothing below changes how this project calls the CLI today; each
item is a note for upstream.

1. **Recursive `fs list`** (or `fs tree`): one call returning a folder's whole
   subtree would let the engine replace the per-folder walk and cut unchanged
   cycles far below the current `fs size`-skip bound. Today the engine makes
   one short-lived CLI process per folder.

2. **Per-folder change stamp**: a reliable `modificationTime` (or revision)
   that updates when a folder's children change. `fs info` currently returns
   the creation time, which is why the skip uses size+count with a bounded
   full re-walk instead.

3. **arm64 Linux binary**: releases are x86_64-only because the official CLI
   ships no Linux arm64 build; an aarch64 AppImage cannot be produced without
   it (this project never bypasses the official CLI).
