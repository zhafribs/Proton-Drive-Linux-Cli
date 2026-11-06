# Proton Drive Linux — Project Clarity

**Status:** Phase 1 shipped in 3.20.26 — shell, Home, Activity  
**Scope:** whole application, not a theme refresh  
**Foundation:** GNOME / libadwaita behavior, Proton brand, Material and Fluent discipline

---

## 1. Why this exists

The app currently works, but its interface carries the fingerprints of an
internal tool: a wall of cards, instructional paragraphs, mixed save behavior,
emoji used as icons, a decorative aurora backdrop, and screens that explain
themselves instead of simply working.

Project Clarity is the rebuild. The goal is not to imitate Google or Microsoft.
The goal is to adopt the operating principles that make their software feel
inevitable:

- one clear job per screen;
- one visual and interaction system;
- native platform behavior;
- calm, readable surfaces;
- accessible by default;
- trust in every destructive or irreversible action.

The app should feel like it belongs on Linux and belongs to Proton.

---

## 2. Design principles

### 2.1 Clarity before decoration
Every element must earn its place. If a sentence only explains the layout, the
layout is wrong. Prefer a clear label, a sensible default, and progressive
disclosure over a permanent paragraph.

### 2.2 One system, not a collection of screens
Colors, spacing, type, radii, elevation, motion, and controls come from tokens.
No screen improvises its own card, button, or spacing.

### 2.3 Native by default
Use Linux conventions: a sidebar and header bar, standard keyboard navigation,
system notifications, a tray where available, predictable window behavior, and
light/dark themes designed together. Do not fight the desktop.

### 2.4 Calm productivity
Chrome recedes; content leads. Restrained accent color, solid neutral surfaces,
subtle depth, and motion that explains change. No drifting background by
default, no glow for its own sake.

### 2.5 Trust is a feature
Sync state is always visible. Deletions are recoverable or confirmed. A failure
names the file, the reason, and the next action. Nothing is silently skipped,
silently duplicated, or silently deleted.

### 2.6 Accessible by default
WCAG 2.2 AA contrast, visible focus, keyboard reachability, screen-reader names,
reduced-motion support, and 44 px minimum targets on touch-capable devices.
Hover may add context; it may never be the only route to critical information.

### 2.7 Performance is design
Immediate feedback, stable layout, no blank waits. Skeletons for unknown data,
progress for long work, and a UI that stays responsive while the engine is busy.

### 2.8 Content is interface
Plain language, active voice, sentence case, no jargon. Errors say what
happened, what was preserved, and what the user can do next.

---

## 3. Visual language

### 3.1 Surfaces
Three levels, used consistently:

| Level | Role | Light | Dark |
|---|---|---|---|
| Page | app background | `#F7F7FB` | `#0E0E16` |
| Surface | cards, tables, panels | `#FFFFFF` | `#171722` |
| Raised | menus, dialogs, popovers | `#FFFFFF` | `#1F1F2E` |

Borders are 1 px hairline. Shadows are soft and rare; elevation communicates
layering, not decoration. Cards are for grouping a task, not for wrapping every
paragraph.

### 3.2 Brand
Proton violet is the single accent:

- accent: `#6D4AFF` (light) / `#A78BFF` (dark)
- accent soft: `#EEE9FF` / `#2A2350`
- accent text: `#5B3FD0` / `#C4B5FF`

Accent is reserved for primary actions, selection, focus, and active navigation.
It is not a background wash.

### 3.3 Semantic color
Success `#16803C`, warning `#B25E00`, danger `#C62828`, info `#2563EB`.
Status is always paired with a label or icon, never color alone.

### 3.4 Typography
- UI: Inter / system UI. Bundle Inter for cross-distro consistency.
- Mono: JetBrains Mono / system mono, only for logs and technical values.
- Scale: 12 / 13 / 14 / 16 / 20 / 28 / 34.
- Body line height 1.45–1.55. Headings 1.2.
- Weights: 400 regular, 500 medium, 600 semibold. No light weights for UI.

### 3.5 Spacing and shape
- 8-point grid: 4, 8, 12, 16, 24, 32, 40, 48.
- Radii: 8 (inputs), 12 (cards), 16 (dialogs), full (pills).
- Icons: one Lucide-style set, 16/20/24 px, 1.75 px stroke. No emoji as icons.
- Content column: max 1120 px, 32 px page padding, 24 px card padding.

### 3.6 Motion
- 150–200 ms for state changes, 220–260 ms for overlays.
- Ease-out for entrance, ease-in for exit.
- Animate opacity/transform, not layout.
- Respect `prefers-reduced-motion` and the existing motion preference.
- No continuous background animation by default.

---

## 4. App shell

```
┌───────────────┬──────────────────────────────────────────────┐
│ Proton Drive  │  Page title              Search   + New      │
│               ├──────────────────────────────────────────────┤
│  Home         │                                              │
│  Files        │              content column                  │
│  Photos       │                                              │
│  Activity     │                                              │
│               │                                              │
│  Settings     │                                              │
│               │                                              │
│  ● Synced     │                                              │
│  Account      │                                              │
└───────────────┴──────────────────────────────────────────────┘
```

- Sidebar: 248 px, collapsible at narrow widths.
- Header: 64 px; page title, contextual actions, search/command.
- Status: a compact sync pill in the sidebar footer; opens Activity on click.
- Account: identity row at the bottom, menu for sign out, settings, about.
- No top-level tabs. Settings uses a secondary in-page navigation.

---

## 5. Page blueprints

### Home
1. Page title and sync status.
2. Hero status panel: state, last sync, next sync, primary **Sync now**.
3. Three metrics: files, photos, storage.
4. Recent activity list (max 5), with a link to Activity.
5. Sync details: next check, app build, remote folder, interval.

No card wall. The hero is the only elevated surface; metrics are a single row.

### Files
1. Breadcrumb + current folder title.
2. Toolbar: search, sort, filter, view toggle, **Upload**.
3. Table/list: name, size, modified, status; multi-select; context menu.
4. Drag-and-drop target with a clear overlay.
5. Empty, loading, and error states with one action each.

### Photos
1. Header: title, count, storage, **Sync photos** / **Upload**.
2. Import progress banner when a pass is running.
3. Sticky date sections; responsive grid; hover checkbox; selection toolbar.
4. Skipped/failed files as a compact, named notice with actions.
5. Download missing / delete selected only when relevant.

### Activity
1. Filter chips: All, Errors, Conflicts, Transfers.
2. Timeline rows: time, action, file, result, expandable detail.
3. Empty state: “Nothing has happened yet.”
4. No raw log by default; a “Copy log command” action for support.

### Settings
1. Search settings.
2. In-page sections: General, Sync, Photos, Notifications, Appearance, Advanced, Updates.
3. One save model: either everything applies immediately with undo, or one
   sticky Save bar for all tabs. Never a mix.
4. Controls with visible labels; helper text only when it prevents a mistake.
5. Destructive actions separated and confirmed.

### Dialogs
- Standard widths: 420 (confirm), 560 (form), 720 (picker).
- Title, one-sentence body, primary action on the right, secondary on the left.
- Destructive primary is red and names the object.
- Escape cancels; Enter confirms the safe primary; focus starts on the safe action.

---

## 6. Component rules

| Component | Rule |
|---|---|
| Button | One primary per view. Secondary = surface + border. Ghost = text only. Danger = red, confirmed. |
| Input | Visible label above; placeholder is an example, not a label. Error below, next to the field. |
| Toggle | Label on the left, switch on the right, optional hover explanation. No permanent helper line. |
| Card | Group one task. Title optional. No nested cards. |
| Banner | Inline status with one action; never used for marketing copy. |
| Status pill | Dot + text. Color is reinforcement, not the message. |
| Empty state | Icon, one sentence, one action. No illustrations that delay the action. |
| Table | Sticky header, zebra-free, 40 px rows, hover and selected states. |
| Toast | 4 s, one line, undo when destructive. Never the only record of a failure. |
| Tooltip | Supplementary only; never the only route to critical information. |

---

## 7. Content style

- Sentence case for labels and headings.
- Buttons are verbs: **Sync now**, **Upload files**, **Save changes**.
- No “Please,” no “simply,” no “just.”
- Errors: what happened + what was preserved + next action.
- No permanent instructional paragraphs. If a control needs a paragraph, redesign
  the control or move the detail into a tooltip/popover.
- Numbers use the user’s locale; sizes use binary units with a short label.

---

## 8. Accessibility and quality gates

Every phase ships only if:

- keyboard-only navigation reaches every action;
- focus is visible on every interactive element;
- screen-reader names exist for icon-only controls;
- contrast is AA at the actual text size;
- reduced motion is respected;
- destructive actions are confirmed or undoable;
- no critical information is hover-only;
- the 1024×720 and 1440×900 layouts both work;
- `cargo test`, release clippy, the release gate, and a smoke run pass.

---

## 9. Rollout

| Phase | Deliverable | User-visible? |
|---|---|---|
| 0 | This constitution, tokens, component inventory, visual prototype | No |
| 1 | New shell, sidebar, header, Home | Yes |
| 2 | Files table + Activity timeline | Yes |
| 3 | Photos grid + import progress + notices | Yes |
| 4 | Settings rebuild + unified save model + dialogs | Yes |
| 5 | Onboarding, accessibility audit, motion/performance polish | Yes |

Each phase gets a three-seat review (design, engineering, QA), a version bump,
and a signed release. No phase is a big-bang rewrite.

---

## 10. What we will not do

- Copy another company’s visual identity or trademarks.
- Keep the aurora/glass look as the default just because it exists.
- Hide critical status behind hover.
- Add decoration that does not communicate hierarchy or state.
- Replace native Linux behavior with web-app mannerisms.
- Ship a redesign without screenshots, keyboard checks, and a rollback path.
