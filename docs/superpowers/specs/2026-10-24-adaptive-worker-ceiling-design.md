# Adaptive workers: enabled dropdowns, configured value as ceiling — short design (2026-10-24)

## Intent

After 3.16.17 shipped, both worker rows ("Scan workers" in Performance and
"Parallel connections" in Fine-tuning) were greyed out whenever adaptive scans
was on — which is the default. A greyed `ComboBox` cannot be opened, so the
setting looked broken rather than read-only. The user asked for the dropdowns
to work. Decision: keep them enabled at all times and make the configured value
a real ceiling for the adaptive loop.

## Change

**GUI (`gui/src/ui/settings.rs`)**

- `number_combo` loses its `enabled` parameter; all four rows are always
  interactive and always show a normal tooltip.
- `workers_enabled()` and its test are removed. Both worker rows write the same
  draft field, so they cannot disagree.
- Hover copy: the adaptive checkbox now says it starts from the scan-worker
  limit and never exceeds it; the workers rows describe the value as an upper
  limit rather than "raw ceiling used when adaptive scans are off".

**Engine (`scripts/proton-sync.py`)**

- `adjust_worker_limit(current, saw_rate_limit, clean_chunks, ceiling=16)` caps
  its clean-chunk creep at `ceiling` instead of a hard-coded 16.
- The four scan/audit call sites pass `ceiling=SCAN_WORKERS` (the configured
  value), so adaptive mode may use **fewer** workers under RAM pressure or
  throttling but never more than the user chose.

## Alternatives considered

- **Keep rows read-only and add an inline hint** — no engine change, but the
  user still cannot set a value without first finding the adaptive checkbox.
- **Remove the adaptive checkbox** — simplest UI, but small-RAM machines lose
  automatic protection against OOM-killing scan processes.

## Consequences

- Adaptive scans still lower concurrency on low-RAM machines and back off on
  rate limits; recovery stops at the configured limit.
- With adaptive off, behaviour is unchanged (`SCAN_WORKERS` is used directly).
- Config keys, defaults (`WORKERS=12`, `AUTO_WORKERS=1`), and the save path do
  not change.

## Verification

- Python: new `test_clean_chunks_never_exceed_the_configured_ceiling` (32
  tests total) covers cap-at-ceiling and growth-to-ceiling.
- Rust: 53 tests (the removed `workers_enabled` test is gone; the range and
  option-list tests remain).
- Build → `--smoke` reports `gui 3.16.18`; both worker dropdowns open in the
  running GUI while adaptive scans is on.
