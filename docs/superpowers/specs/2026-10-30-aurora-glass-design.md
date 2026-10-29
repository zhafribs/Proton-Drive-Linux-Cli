# Aurora Glass — GUI redesign

2026-10-30 · approved direction: **A — Aurora Glass** (dark-first, animated).

## Intent

Make the app's GUI something the owner wants to look at every day: a living,
deep-space interface around the Proton Drive purple — slowly drifting aurora
light, glass panels, glowing status orb, gradient call-to-action buttons —
without losing any existing feature or spending meaningful RAM/CPU.

## Goals

- A distinctive, motion-rich visual identity (animated ambient background,
  glass cards, glowing orb, gradient CTAs, hover micro-interactions).
- Keep every existing screen and control working identically.
- Performance budget: no per-frame heap allocations, no image assets, no
  shaders. Motion stops when the window is hidden. A
  `PROTON_UI_MOTION=0` env switch freezes all animation.
- Restyle the light theme as a "frosted daylight" variant of the same layout.

## Non-goals / anti-goals

- No bitmap backgrounds, blur passes or external icon sets.
- No feature removal; features may be rearranged for hierarchy.
- Anti-goal: janky or memory-hungry animation, and a light theme that looks
  like an afterthought.

## Constraints

- egui 0.37 immediate mode; painting is GPU-cheap but re-runs every frame.
- The app already repaints on input and every ~900 ms; the aurora asks for a
  continuous repaint while visible only.
- Dark becomes the default theme; users keep the existing light/dark/system
  switcher.
- Layout and copy are in scope: the builder may move sections and reword
  user-facing strings toward a calmer, professional tone (features stay).

## Approaches considered

1. **Aurora Glass (chosen)** — dark navy, animated violet/cyan glow, glass.
2. Sunlit Ceramic — light, calm; rejected as less distinctive.
3. Neon Tokyo — bold outlines; rejected: Proton purple loses identity.

## How it is built

- **Aurora background**: five large translucent discs (violet, cyan, magenta,
  accent) whose centres drift on slow sine paths. Painted per frame with
  `circle_filled` — zero allocations, no textures. When hidden or
  `PROTON_UI_MOTION=0`, positions are frozen to a static pleasing layout.
- **Glass surfaces**: surface fills become translucent tints over the aurora,
  with a 1px hairline stroke and a white top sheen on the upper edge. Corner
  radius 18. Cards, sidebar, inputs and popups share the treatment.
- **Gradient CTA**: vertical two-stop gradient (violet → cyan) drawn as a
  rounded mesh once per button; hover brightens, press darkens.
- **Status orb**: larger neon ring with a two-tone comet and a soft outer
  aura that pulses with the aurora palette.
- **Navigation**: glass pills; the active row wears a gradient-tinted fill
  with a glowing edge; hover shows a translucent sheen.
- **Typography/spacing**: same type scale, slightly larger radii and gaps;
  titles keep their sizes.
- **Light variant**: warm white glass ("frosted daylight") with the same
  radii, sheens and layout; no separate code paths beyond palette values.
- **Motion values** (builder's call): blob drift ≈ 0.02–0.05 rad/s, colours
  at 4–7% alpha, halo pulse ≈ 2.5 s.

## Left to the builder

Exact hex values, blob radii/speeds, sheen opacities, and the dark palette;
these follow the constraints above and the existing theme-token structure in
`gui/src/theme.rs` (which stays the single source of truth).
