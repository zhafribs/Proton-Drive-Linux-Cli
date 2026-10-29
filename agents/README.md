# Agency agents

Vendored specialist agent prompts from
[`msitarzewski/agency-agents`](https://github.com/msitarzewski/agency-agents)
(MIT license). They are reference playbooks, not code: the agents in this
directory describe how a specialist would review or build something. `AGENTS.md`
tells sessions when to consult them.

| File | Use |
| --- | --- |
| `design-ui-designer.md` | Design-system and pixel-level UI changes |
| `design-ux-architect.md` | Layout structure, CSS/layout foundations, developer-friendly interfaces |
| `design-ui-finish-gate-reviewer.md` | Pre-release check that the UI is not generic (design contract, evidence) |
| `product-trend-researcher.md` | Trend research for UI/UX decisions |
| `engineering-senior-developer.md` | Complex implementation judgment |
| `engineering-minimal-change-engineer.md` | Minimum-viable diffs, no scope creep |
| `engineering-code-reviewer.md` | Constructive review checklist |
| `engineering-rust-refactoring-specialist.md` | Behavior-preserving Rust refactors |
| `engineering-section-508-specialist.md` | Accessibility (contrast, keyboard, focus) baseline |

Upstream may add more agents over time; when a task needs a different
specialist, fetch that agent from the upstream repo rather than inventing one.
