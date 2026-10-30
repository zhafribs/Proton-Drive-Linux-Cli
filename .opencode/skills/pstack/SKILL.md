---
name: pstack
description: Lauren Tan's pstack workflow (MIT) for rigorous, verifiable engineering. Use for pstack, /pstack, poteto-mode, or when a task needs deep-first, low-slop, verified work.
---

# pstack (OpenCode adapter)

pstack is vendored at `../../../agents/pstack` (MIT, Lauren Tan, v0.15.36).
Read the upstream entry point before starting:

`../../../agents/pstack/skills/poteto-mode/SKILL.md`

## Non-negotiables

- Open your first reply by restating the goal, the problem it solves, and the
  smallest change you expect to make. Do not wait for confirmation.
- Name the data shape before writing code, then choose the organizing
  structure (state machine, typed model, table, boundary) deliberately.
- Fix root causes. Subtract before adding. Keep the diff minimal.
- Prove it works with a tight check that can actually go red on this change.
  Re-run the original scenario, not just the new test.
- Write like pstack's `unslop`: direct, specific, no filler or ceremony.

## OpenCode translation

- `AskQuestion` maps to the `question` tool. Ask only when no experiment can
  settle the fork; otherwise build the smallest prototype or inspect evidence.
- `Task` and `poteto-agent` map to the `subagent` tool: `general` for delegated
  implementation, `explore` for search. pstack's Cursor model routing does not
  apply — do not invent model slugs or write `~/.cursor/rules`.
- Cursor slash commands, plugins, automations, `cursor-team-kit`, `deslop`, and
  Bugbot are unavailable here. Skip those steps with a one-line note.
- Ignore `disable-model-invocation` in the vendored copies: this adapter is the
  only advertised entry point.
- Inside the vendored skills, `../<name>/SKILL.md` means
  `agents/pstack/skills/<name>/SKILL.md`.
- Project rules win: Rust only, the official Proton CLI stays a subprocess,
  user-visible changes bump `GUI_VERSION` and add a CHANGELOG entry, and
  `cargo test` plus `cargo clippy --release --all-targets` must be clean.

## How to run

1. Read `poteto-mode/SKILL.md` and choose the playbook it names for the task
   (feature, bug-fix, refactor, perf, investigation, PR, and so on) from
   `agents/pstack/skills/poteto-mode/playbooks/`.
2. Load only the principles and skills that playbook names; never bulk-read
   the tree.
3. Check the plan against `AGENTS.md` before editing.
4. Verify against the user's original request, not the plan.
