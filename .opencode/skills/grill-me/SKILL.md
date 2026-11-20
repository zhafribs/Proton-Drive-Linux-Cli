---
name: grill-me
description: Interview workflow (Matt Pocock style) that asks tough qualifying questions about edge cases and domain rules to one-shot complex features.
---

# Grill Me

Forces the agent to interview the user with tough, single questions to extract precise requirements before coding.

## Guidelines
- Ask **one sharp question at a time**.
- Probe edge cases, error modes, state persistence, and domain constraints.
- Continue interviewing until ~95% confidence on underlying intent is reached.
- Proceed to execution once all ambiguities are resolved.
