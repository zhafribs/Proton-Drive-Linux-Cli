---
name: thermo-nuclear-audit
description: Merciless codebase quality review audit to catch technical debt, maintainability traps, security flaws, and structural issues.
---

# Thermo-Nuclear Code Quality Review

Runs an unsparing, top-to-bottom audit across the codebase.

## Audit Dimensions
1. **Bugs & Edge Cases**: Unhandled errors, lock deadlocks, panics, race conditions.
2. **Technical Debt & Bloat**: Dead code, duplicate state, unnecessary abstractions.
3. **Security & Keyring**: Data leakage, unsafe subprocess handling, file permissions.
4. **Performance & Memory**: Slow paths, redundant I/O, unnecessary allocations.
