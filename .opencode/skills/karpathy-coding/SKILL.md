---
name: karpathy-coding
description: Andrej Karpathy's coding discipline: think before coding, write minimal code, make surgical edits, and loop until verified.
---

# Karpathy's Coding Discipline

Enforces tight engineering rigor modeled after Andrej Karpathy's core coding habits.

## Execution Rules
1. **Think Before Coding**: Fully understand callers, dependencies, and side effects.
2. **Surgical Edits**: Touch only what is necessary; minimal, crisp diffs.
3. **No Bloat**: Write clean, direct code with no speculative abstractions.
4. **Verification Loop**: Run tests and checks (`cargo test`, `cargo clippy`) until criteria pass cleanly.
