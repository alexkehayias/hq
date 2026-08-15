# hq Web Components

A composable design system using web components (light DOM), HTM for templating, and Tailwind v3. No bundler — each component is one ES module loaded via `<script type="module">`.

## How to use

1. Load all components: `<script type="module" src="/components/index.js"></script>`
2. Write declarative HTML using the custom elements
3. Listen for events via `addEventListener` (inline handlers like `onClick` are not supported)

For visual review, see `/components-gallery/`.

## Components

### Tier 1 — Primitives

#### `<hq-page-shell>`
- **File:** `./hq-page-shell.js`
- **Attrs:** `title` (sets document.title), `back-href` (URL, default `/`), `back-label` (default `Home`)
- **Slots:** default (page body, placed in `<main>`)
- Wraps gradient background + safe-area padding + "Back to" link. Used by every page.

#### `<hq-card>`
- **File:** `./hq-card.js`
- **Attrs:** `max-width` (`2xl|4xl|6xl|full`, default `4xl`)
- **Slots:** `header` (optional), default (body)
- Surface container with rounded corners + shadow. Optional header section.

#### `<hq-button>`
- **File:** `./hq-button.js`
- **Attrs:** `variant` (`primary|secondary|ghost|danger`, default `primary`), `disabled`
- **Slots:** default (label)
- Styled button. Renders a real `<button>` in light DOM (focusable, keyboard-activatable).

#### `<hq-badge>`
- **File:** `./hq-badge.js`
- **Attrs:** `tone` (`blue|green|yellow|red|gray`, default `gray`)
- **Slots:** default (text)
- Colored pill badge. CSS-only.

#### `<hq-spinner>`
- **File:** `./hq-spinner.js`
- **Attrs:** `size` (`sm|md|lg`, default `md`), `tone` (optional color class)
- Loading spinner. CSS-only.

#### `<hq-icon>`
- **File:** `./hq-icon.js`
- **Attrs:** `name` (required), `size` (`sm|md|lg`, default `md`), `tone` (optional color class, e.g. `text-red-400`)
- Inline SVG icon library. Names: `chevron-left`, `chevron-right`, `search`, `chat`, `sessions`, `skills`, `metrics`, `close`, `alert`, `plus`. Uses HTM.

### Tier 2 — Composites

#### `<hq-state-view>`
- **File:** `./hq-state-view.js`
- **Attrs:** `state` (`loading|error|empty|content`, default `content`)
- **Slots:** `loading`, `error`, `empty`, `content`
- State-driven slot switcher. Non-active slots get `display:none`. Default loading slot injects `<hq-spinner>` if not provided.

#### `<hq-empty-state>`
- **File:** `./hq-empty-state.js`
- **Attrs:** `icon` (optional hq-icon name), `title`
- **Slots:** default (description), `action` (optional button)
- Centered empty/placeholder message. CSS-only.

#### `<hq-modal>`
- **File:** `./hq-modal.js`
- **Attrs:** `open` (boolean)
- **Events:** `close` (bubbles — fired on Escape key or backdrop click)
- Full-screen overlay modal with centered card.

#### `<hq-pagination>`
- **File:** `./hq-pagination.js`
- **Attrs:** `page` (number, 1-indexed), `total-pages` (number)
- **Events:** `page-change` with `{ detail: { page } }`
- Page navigation with 5-page window. Replaces inline `onclick=` globals.

### Tier 3 — Page-Specific

#### `<hq-file-tree>`
- **File:** `./hq-file-tree.js`
- **Attrs:** `files` (JSON: `[{path, is_directory}]`), `selected`
- **Events:** `file-select` with `{ detail: { path } }`
- File list sidebar with directory expansion. Sorts SKILL.md first, then dirs before files. Uses HTM.

#### `<hq-stat-card>`
- **File:** `./hq-stat-card.js`
- **Attrs:** `label`, `value`
- **Slots:** default (optional formatted value, overrides `value` attr)
- KPI stat card with label and large value. CSS-only.

#### `<hq-session-item>`
- **File:** `./hq-session-item.js`
- **Attrs:** `session` (JSON: `{ id, title?, summary?, tags? }`)
- Chat session list row with title (falls back to `Session {id}`), optional tag chips, summary, and "View »" link. Uses HTM.

## Conventions

- **Light DOM** — no Shadow DOM. Tailwind classes work directly.
- **HTM for data-driven HTML** — `import { html } from '/components/lib/html.js'`. CSS-only components (badge, spinner) don't need HTM.
- **Event handlers via `addEventListener`** — inline `onClick` props are dropped by the serializer.
- **Dark mode** — every page must load `/theme-init.js` in `<head>` (before the body renders) so the `.dark` class is set without a flash. Without it, a page silently loses dark mode.
- **Reference:** `web-ui/src/chat/message-bubble.js` (the existing web component pattern, not migrated).

## File layout

```
web-ui/src/
  vendor/htm.js                    ← HTM library (~1KB ESM)
  theme-init.js                    ← pre-paint dark-mode init (loaded in <head>)
  theme.js                         ← exported helpers (isDark/setDark/toggleDark)
  components/
    lib/html.js                     ← serializer (SafeHtml + esc/escAttr)
    index.js                        ← imports all components
    README.md                       ← this file
    hq-page-shell.js, hq-card.js, ...
  components-gallery/
    index.html, index.js            ← visual reference page
```