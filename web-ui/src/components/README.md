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
- Inline SVG icon library. Names: `chevron-left`, `chevron-right`, `chevron-down`, `search`, `chat`, `sessions`, `skills`, `metrics`, `close`, `alert`, `plus`, `folder`, `file`. Uses HTM.

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

#### `<hq-select>`
- **File:** `./hq-select.js`
- **Attrs:** `label`, `value`, `disabled`
- **Slots:** default (native `<option>` elements)
- **Events:** `change` with `{ detail: { value } }` (bubbles)
- Styled dropdown wrapping a native `<select>` (appearance-none) with a themed chevron. Handles dark mode correctly.

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

## Composing components

Components nest like regular HTML and talk to each other **only through events**.
Every page that uses them must load two things in `<head>`:

```html
<link href="/output.css" rel="stylesheet">
<script src="/theme-init.js"></script>          <!-- dark mode, before first paint -->
<script type="module" src="/components/index.js"></script>  <!-- all components -->
```

### Rules

1. **One page = one `<hq-page-shell>`.** It owns the gradient background, safe-area
   padding, and the "Back to" link. Everything else goes in its default slot (`<main>`).
2. **Nest freely** — components are light-DOM custom elements, so `hq-state-view`
   can wrap `hq-empty-state`, which can contain `hq-button`, and so on.
3. **Wire with `addEventListener`**, never inline `onClick`. Each component fires a
   bubbled custom event carrying the data you need:
   - `hq-select` → `change` with `detail.value`
   - `hq-pagination` → `page-change` with `detail.page`
   - `hq-file-tree` → `file-select` with `detail.path`
   - `hq-modal` → `close`
4. **Read state back from attributes** (e.g. `filter.getAttribute('value')`) and
   **drive state by setting attributes** (e.g. `view.setAttribute('state', 'loading')`).
5. **Use the `html` serializer from `/components/lib/html.js`** for any data-driven
   markup you build in JS — it auto-escapes strings and drops `on*` handlers.

### Example — a "Tasks" page

```html
<body>
  <hq-page-shell title="Tasks" back-href="/" back-label="Home">

    <div class="flex items-end justify-between gap-4 mb-6 flex-wrap">
      <hq-select id="task-filter" label="Status">
        <option value="all" selected>All</option>
        <option value="open">Open</option>
        <option value="done">Done</option>
      </hq-select>
      <hq-button id="new-task">+ New task</hq-button>
    </div>

    <hq-state-view id="task-view" state="loading">
      <div slot="empty">
        <hq-empty-state icon="search" title="No tasks match">
          Try a different filter.
        </hq-empty-state>
      </div>
      <div slot="content">
        <div id="task-list" class="space-y-4"></div>
        <hq-pagination id="task-pagination" page="1" total-pages="5"></hq-pagination>
      </div>
    </hq-state-view>

  </hq-page-shell>
</body>
```

```js
import { html } from '/components/lib/html.js';

const view = document.getElementById('task-view');
const filter = document.getElementById('task-filter');
const pagination = document.getElementById('task-pagination');
const list = document.getElementById('task-list');

filter.addEventListener('change', async (e) => {
  view.setAttribute('state', 'loading');
  await loadTasks(e.detail.value, 1);
});

pagination.addEventListener('page-change', (e) => {
  view.setAttribute('state', 'loading');
  loadTasks(filter.getAttribute('value'), e.detail.page);
});

async function loadTasks(status, page) {
  const res = await fetch(`/api/tasks?status=${status}&page=${page}`);
  const tasks = await res.json();

  if (!tasks.length) {
    view.setAttribute('state', 'empty');
    return;
  }

  list.innerHTML = tasks
    .map((t) => html`
      <hq-card max-width="full">
        <div slot="header">
          <div class="flex items-center justify-between">
            <h3 class="text-lg font-semibold text-gray-900 dark:text-white">${t.title}</h3>
            <hq-badge tone="${t.status === 'done' ? 'green' : 'yellow'}">${t.status}</hq-badge>
          </div>
        </div>
        <p class="text-sm text-gray-600 dark:text-gray-400">${t.summary}</p>
      </hq-card>
    `)
    .map((r) => r.value)
    .join('');

  pagination.setAttribute('total-pages', String(tasks.totalPages));
  view.setAttribute('state', 'content');
}
```

## Conventions

- **Light DOM** — no Shadow DOM. Tailwind classes work directly.
- **HTM for data-driven HTML** — `import { html } from '/components/lib/html.js'`. CSS-only components (badge, spinner) don't need HTM.
- **Event handlers via `addEventListener`** — inline `onClick` props are dropped by the serializer.
- **Dark mode** — every page must load `/theme-init.js` in `<head>` (before the body renders) so the `.dark` class is set without a flash. Without it, a page silently loses dark mode.
- **Page-specific components colocate with their page** — `components/` holds only reusable, cross-page components. A component used by a single page lives next to that page (e.g. `chat/sessions/session-item.js`) and is imported only by that page's module. When a second page needs it, move it into `components/`, add it to `index.js`, and update both importers. (Legacy exceptions still in `components/`: `hq-file-tree`, `hq-stat-card`.)
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