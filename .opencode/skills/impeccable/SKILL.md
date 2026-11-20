---
name: impeccable
description: Design vocabulary and craft for frontend UI. Steer with `polish` and `bolder`, review with `critique` and `audit`, catch AI slop before it ships. Use for design, redesign, shape, critique, audit, polish, harden, animate, or improve a frontend interface.
version: 4.10.0
license: Apache 2.0
source: https://github.com/pbakaus/impeccable
---

# Impeccable (pbakaus/impeccable)

Upstream: [`pbakaus/impeccable`](https://github.com/pbakaus/impeccable) (Apache 2.0).

Install the upstream skill for full functionality (scripts, engine binary, live mode):

```
npx impeccable install
```

## What it does

A design vocabulary for coding agents. Provides 24 commands split into four groups:

- **Steer**: `bolder`, `quieter`, `distill`, `colorize`, `typeset`, `layout`, `animate`, `delight`, `overdrive`
- **Review**: `critique` (scored UX review), `audit` (a11y/perf/responsive)
- **Finish**: `polish`, `harden`, `clarify`, `adapt`, `optimize`, `onboard`
- **Plan**: `shape`, `init`, `document`, `extract`
- **Iterate**: `live`, `generate`

It writes `PRODUCT.md` and `DESIGN.md` context files and runs a 62-rule deterministic slop detector.

## Project note

This project is a **Rust + egui desktop GUI**, not a web app. Impeccable's web-centric commands (live mode, shader effects, browser detection) do not apply. Use the review commands (`critique`, `audit`) and the steer commands (`bolder`, `quieter`, `typeset`, `layout`) against the `gui/` source instead. `adapt` and `optimize` are partially relevant for window sizing and render performance.