---
name: design-taste-frontend
description: Anti-slop frontend skill. Builds professional company websites at the level of figma.com and x.ai: plain copy, one system, one section per screen, real photographs, hero entrances, and a checker + finish gate that enforces it in a real browser.
version: 2.5.1
license: MIT
source: https://github.com/Leonxlnx/taste-skill
---

# Taste Skill (Leonxlnx/taste-skill)

Upstream: [`Leonxlnx/taste-skill`](https://github.com/Leonxlnx/taste-skill) (MIT).

Install the upstream skill for full functionality (Node 22+, Chrome/Chromium):

```
npx skills add https://github.com/Leonxlnx/taste-skill --skill "design-taste-frontend"
```

## What it does

Builds company/product landing pages with real photography, dealt palettes and type pairings, one section per screen, equal left/right edges, hover micro-interactions, and a headless-Chrome consistency checker plus a finish gate (`gate.mjs`).

## Project note

This project is a **Rust + egui desktop GUI** for a file-sync client, not a marketing landing page. Taste Skill's page-plan, photograph-dealing, and browser-checker pipeline do not apply here. The relevant transferable rules are its anti-slop discipline and its finish gate concept — apply them to `gui/src/ui/` reviews rather than running the upstream scripts.