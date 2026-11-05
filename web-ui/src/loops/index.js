// Loops page: a tabbed list of loops and channels, plus a detail view for each.
import { html } from '/components/lib/html.js';

/* ===== State ===== */
let loops = [];
let channels = [];
let currentLoop = null;
let activityPage = 1;
let activityLoaded = false;

/* ===== Elements ===== */
const pageShell = document.getElementById('page-shell');
const listView = document.getElementById('list-view');
const loopDetailView = document.getElementById('loop-detail-view');
const channelDetailView = document.getElementById('channel-detail-view');

const tabs = document.getElementById('tabs');
const loopsPanel = document.getElementById('loops-panel');
const channelsPanel = document.getElementById('channels-panel');
const activityPanel = document.getElementById('activity-panel');
const loopsState = document.getElementById('loops-state');
const channelsState = document.getElementById('channels-state');
const activityState = document.getElementById('activity-state');
const loopsErrorMsg = document.getElementById('loops-error-msg');
const channelsErrorMsg = document.getElementById('channels-error-msg');
const activityErrorMsg = document.getElementById('activity-error-msg');
const loopGrid = document.getElementById('loop-grid');
const channelGrid = document.getElementById('channel-grid');
const activityList = document.getElementById('activity-list');
const activityPagination = document.getElementById('activity-pagination');
const promptSection = document.getElementById('loop-detail-prompt');
const promptEditor = document.getElementById('prompt-editor');

// Dark mode sync for CodeMirror
const darkMode = window.matchMedia('(prefers-color-scheme: dark)');
darkMode.addEventListener('change', syncTheme);
syncTheme();

function syncTheme() {
  const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
  const themeLink = document.getElementById('cm-theme');
  if (themeLink) themeLink.disabled = !isDark;
}

/* ===== Data ===== */
async function loadData() {
  const [loopsRes, channelsRes] = await Promise.all([
    fetch('/api/loops'),
    fetch('/api/channels'),
  ]);
  if (!loopsRes.ok) throw new Error(`HTTP ${loopsRes.status}`);
  if (!channelsRes.ok) throw new Error(`HTTP ${channelsRes.status}`);
  loops = (await loopsRes.json()).loops || [];
  channels = (await channelsRes.json()).channels || [];
}

function subscribersOf(id) {
  return loops.filter((loop) => loop.subscribe_channels.includes(id));
}

/* ===== List View ===== */
function showList(activeTab) {
  listView.classList.remove('hidden');
  loopDetailView.classList.add('hidden');
  channelDetailView.classList.add('hidden');
  pageShell.setAttribute('meta-title', 'Loops');
  pageShell.setAttribute('back-href', '/');
  pageShell.setAttribute('back-label', 'Home');
  selectTab(activeTab);
  loadList();
}

const TAB_BASE =
  'rounded-[9px] px-4 py-1.5 text-sm font-medium transition-colors';
const TAB_ACTIVE = `${TAB_BASE} bg-card text-ink shadow-sm`;
const TAB_IDLE = `${TAB_BASE} text-muted hover:text-ink`;

const TAB_PANELS = {
  loops: loopsPanel,
  channels: channelsPanel,
  activity: activityPanel,
};

function selectTab(tab) {
  const active = TAB_PANELS[tab] ? tab : 'activity';

  for (const [name, panel] of Object.entries(TAB_PANELS)) {
    panel.classList.toggle('hidden', name !== active);
    const button = document.getElementById(`tab-${name}`);
    button.className = name === active ? TAB_ACTIVE : TAB_IDLE;
    button.setAttribute('aria-selected', String(name === active));
  }

  if (active === 'activity' && !activityLoaded) loadActivity(activityPage);
}

tabs.addEventListener('click', (event) => {
  const button = event.target.closest('button[data-tab]');
  if (!button) return;
  selectTab(button.dataset.tab);

  const url = new URL(window.location.href);
  if (button.dataset.tab === 'activity') url.searchParams.delete('tab');
  else url.searchParams.set('tab', button.dataset.tab);
  window.history.replaceState({}, '', url);
});

async function loadList() {
  loopsState.setAttribute('state', 'loading');
  channelsState.setAttribute('state', 'loading');
  try {
    await loadData();
    renderLoops();
    renderChannels();
  } catch (err) {
    loopsErrorMsg.textContent = `Failed to load loops: ${err.message}`;
    channelsErrorMsg.textContent = `Failed to load channels: ${err.message}`;
    loopsState.setAttribute('state', 'error');
    channelsState.setAttribute('state', 'error');
  }
}

document.getElementById('loops-retry').addEventListener('click', loadList);
document.getElementById('channels-retry').addEventListener('click', loadList);
document
  .getElementById('activity-retry')
  .addEventListener('click', () => loadActivity(activityPage));

activityPagination.addEventListener('page-change', (event) => {
  loadActivity(event.detail.page);
});

function renderLoops() {
  if (loops.length === 0) {
    loopsState.setAttribute('state', 'empty');
    return;
  }
  loopGrid.innerHTML = loops.map(loopCard).map(toValue).join('');
  loopsState.setAttribute('state', 'content');
}

function renderChannels() {
  if (channels.length === 0) {
    channelsState.setAttribute('state', 'empty');
    return;
  }
  channelGrid.innerHTML = channels.map(channelCard).map(toValue).join('');
  channelsState.setAttribute('state', 'content');
}

/* ===== Activity ===== */
const ACTIVITY_LIMIT = 20;

async function loadActivity(page = 1) {
  activityPage = page;
  activityState.setAttribute('state', 'loading');
  try {
    const res = await fetch(
      `/api/loops/runs?page=${page}&limit=${ACTIVITY_LIMIT}`,
    );
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    const runs = data.runs || [];

    if (runs.length === 0) {
      activityState.setAttribute('state', 'empty');
    } else {
      activityList.innerHTML = runs
        .map((run) => runRow(run, true))
        .map(toValue)
        .join('');
      activityPagination.setAttribute('page', String(data.page));
      activityPagination.setAttribute('total-pages', String(data.total_pages));
      activityState.setAttribute('state', 'content');
    }
    activityLoaded = true;
  } catch (err) {
    activityErrorMsg.textContent = `Failed to load activity: ${err.message}`;
    activityState.setAttribute('state', 'error');
  }
}

/* ===== Cards ===== */
function loopCard(loop) {
  return html`
    <a href="/loops/?loop=${loop.id}"
       class="flex flex-col rounded-card border border-line bg-card p-4 transition-shadow hover:shadow-lg/5 md:p-[22px]">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <h3 class="font-semibold text-ink">Loop</h3>
          <p class="mt-0.5 truncate font-mono text-xs text-muted">${loop.id}</p>
        </div>
        ${loop.running ? html`<hq-badge tone="moss">running</hq-badge>` : html`<hq-badge tone="gray">stopped</hq-badge>`}
      </div>
      <div class="mt-3 flex flex-wrap gap-1.5">
        ${
          loop.subscribe_channels.length
            ? loop.subscribe_channels.map(
                (channel) => html`<hq-badge tone="teal">${channel}</hq-badge>`,
              )
            : html`<span class="text-xs text-muted">No channels</span>`
        }
      </div>
    </a>
  `;
}

function channelCard(channel) {
  const count = subscribersOf(channel.id).length;
  return html`
    <a href="/loops/?channel=${channel.id}"
       class="flex flex-col rounded-card border border-line bg-card p-4 transition-shadow hover:shadow-lg/5 md:p-[22px]">
      <div class="flex items-start justify-between gap-3">
        <h3 class="min-w-0 truncate font-mono font-semibold text-ink">${channel.id}</h3>
        <div class="flex shrink-0 items-center gap-1.5">
          ${channel.created_by ? html`<hq-badge tone="amber">agent</hq-badge>` : ''}
          <hq-badge tone="${sourceTone(channel.source)}">${channel.source}</hq-badge>
        </div>
      </div>
      <p class="mt-3 text-sm text-muted">${count} loop${count === 1 ? '' : 's'} subscribed</p>
    </a>
  `;
}

/* ===== Loop Detail ===== */
async function showLoopDetail(id) {
  listView.classList.add('hidden');
  channelDetailView.classList.add('hidden');
  loopDetailView.classList.remove('hidden');
  pageShell.setAttribute('meta-title', 'Loop');
  pageShell.setAttribute('back-href', '/loops/');
  pageShell.setAttribute('back-label', 'Loops');

  currentLoop = null;
  promptSection.classList.add('hidden');
  document.getElementById('loop-detail-id').textContent = id;
  document.getElementById('loop-detail-status').innerHTML = '';
  const body = document.getElementById('loop-detail-body');
  body.innerHTML = spinner();

  try {
    await loadData();
  } catch (err) {
    body.innerHTML = errorState('Failed to load loop', err.message);
    return;
  }

  let runs = [];
  let runsError = null;
  try {
    const res = await fetch(
      `/api/loops/${encodeURIComponent(id)}/runs?limit=20`,
    );
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    runs = (await res.json()).runs || [];
  } catch (err) {
    runsError = err.message;
  }

  const loop = loops.find((l) => l.id === id);
  if (!loop) {
    body.innerHTML = errorState(
      'Loop not found',
      'No loop with this id exists.',
    );
    return;
  }

  document
    .getElementById('loop-detail-status')
    .appendChild(
      badge(
        loop.running ? 'running' : 'stopped',
        loop.running ? 'moss' : 'gray',
      ),
    );

  body.innerHTML = html`<div class="flex flex-col gap-6">
    ${section(
      'Subscribed channels',
      loop.subscribe_channels.length
        ? html`<div class="flex flex-wrap gap-2">
            ${loop.subscribe_channels.map((channel) => html`<a href="/loops/?channel=${channel}"><hq-badge tone="teal">${channel}</hq-badge></a>`)}
          </div>`
        : html`<p class="text-sm text-muted">This loop is not subscribed to any channels.</p>`,
    )}
    ${section(
      'Emit channels',
      loop.emit_channels.length
        ? html`<div class="flex flex-wrap gap-2">
            ${loop.emit_channels.map((channel) => html`<a href="/loops/?channel=${channel}"><hq-badge tone="amber">${channel}</hq-badge></a>`)}
          </div>`
        : html`<p class="text-sm text-muted">This loop can only emit to channels it creates itself.</p>`,
    )}
    ${section(
      'Tools',
      loop.tools.length
        ? html`<div class="flex flex-wrap gap-2">${loop.tools.map((tool) => html`<hq-badge tone="gray">${tool}</hq-badge>`)}</div>`
        : html`<p class="text-sm text-muted">Using the default tools (bash, notify).</p>`,
    )}
    ${section('Recent runs', runsSection(runs, runsError))}
    ${section('Created', html`<p class="text-sm text-body">${loop.created_at}</p>`)}
  </div>`.value;

  currentLoop = loop;
  promptSection.classList.remove('hidden');
  promptEditor.open('', loop.system_prompt, 'markdown');
}

/* ===== Prompt Save ===== */
promptEditor.addEventListener('save-request', savePrompt);

async function savePrompt() {
  if (!currentLoop) return;

  promptEditor.setSaving();

  try {
    const res = await fetch(
      `/api/loops/${encodeURIComponent(currentLoop.id)}`,
      {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ system_prompt: promptEditor.getValue() }),
      },
    );

    if (!res.ok) {
      const errData = await res.json().catch(() => ({}));
      throw new Error(errData.error || `HTTP ${res.status}`);
    }

    currentLoop.system_prompt = promptEditor.getValue();
    promptEditor.setSaved();
  } catch (err) {
    promptEditor.setSaveError(err.message);
  }
}

/* ===== Channel Detail ===== */
async function showChannelDetail(id) {
  listView.classList.add('hidden');
  loopDetailView.classList.add('hidden');
  channelDetailView.classList.remove('hidden');
  pageShell.setAttribute('meta-title', id);
  pageShell.setAttribute('back-href', '/loops/?tab=channels');
  pageShell.setAttribute('back-label', 'Channels');

  document.getElementById('channel-detail-id').textContent = id;
  document.getElementById('channel-detail-badges').innerHTML = '';
  const body = document.getElementById('channel-detail-body');
  body.innerHTML = spinner();

  try {
    await loadData();
  } catch (err) {
    body.innerHTML = errorState('Failed to load channel', err.message);
    return;
  }

  const channel = channels.find((c) => c.id === id);
  if (!channel) {
    body.innerHTML = errorState(
      'Channel not found',
      'No channel with this id exists.',
    );
    return;
  }

  const badges = document.getElementById('channel-detail-badges');
  if (channel.created_by) badges.appendChild(badge('agent', 'amber'));
  badges.appendChild(badge(channel.source, sourceTone(channel.source)));

  const subscribers = subscribersOf(channel.id);
  body.innerHTML = html`<div class="flex flex-col gap-6">
    ${section(
      'Details',
      html`<dl class="grid grid-cols-[7rem_1fr] gap-y-2 text-sm">
        <dt class="text-muted">Source</dt><dd class="text-body">${channel.source}</dd>
        ${
          channel.created_by
            ? html`<dt class="text-muted">Owner</dt><dd class="font-mono text-body">${channel.created_by}</dd>`
            : ''
        }
        <dt class="text-muted">Debounce</dt><dd class="text-body">${channel.debounce_ms} ms</dd>
        <dt class="text-muted">Created</dt><dd class="text-body">${channel.created_at}</dd>
      </dl>`,
    )}
    ${section(
      `Subscribed loops (${subscribers.length})`,
      subscribers.length
        ? html`<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">${subscribers.map(loopCard)}</div>`
        : html`<p class="text-sm text-muted">No loops subscribe to this channel.</p>`,
    )}
  </div>`.value;
}

/* ===== Runs ===== */
function runsSection(runs, error) {
  if (error) {
    return html`<p class="text-sm text-rust">Failed to load runs: ${error}</p>`;
  }
  if (runs.length === 0) {
    return html`<p class="text-sm text-muted">No runs yet. Runs appear when the loop receives an event.</p>`;
  }
  return html`<div class="flex flex-col gap-2">${runs.map(runRow)}</div>`;
}

function runRow(run, showLoop = false) {
  const detail = run.error || run.response || run.event;
  const content = html`
    <div class="flex items-center justify-between gap-2">
      <div class="flex flex-wrap items-center gap-2">
        ${statusIcon(run.status)}
        ${showLoop ? html`<span class="font-mono text-xs text-body">${run.loop_id}</span>` : ''}
        <span class="font-mono text-xs text-muted">${run.channel_id}</span>
        <span class="text-xs text-muted">${formatTime(run.started_at)}</span>
        ${run.duration_ms != null ? html`<span class="text-xs text-muted">${formatDuration(run.duration_ms)}</span>` : ''}
      </div>
      ${
        run.has_transcript
          ? html`<span class="shrink-0 text-muted transition-colors group-hover:text-teal-strong"><hq-icon name="chevron-right" size="sm"></hq-icon></span>`
          : ''
      }
    </div>
    <p class="mt-2 max-h-24 overflow-hidden whitespace-pre-wrap break-words font-mono text-xs text-body">${detail}</p>
  `;

  if (!run.has_transcript) {
    return html`<div class="rounded-[14px] border border-line bg-card p-3">${content}</div>`;
  }

  return html`<a href="/chat/index.html?session_id=${encodeURIComponent(run.session_id)}"
               class="group block rounded-[14px] border border-line bg-card p-3 transition-shadow hover:shadow-lg/5 focus:outline-none focus:ring-2 focus:ring-teal">${content}</a>`;
}

const STATUS_ICON = {
  success: { name: 'check', tone: 'text-moss' },
  error: { name: 'alert', tone: 'text-rust' },
  interrupted: { name: 'close', tone: 'text-muted' },
  running: { name: 'spinner', tone: 'text-teal animate-spin' },
};

function statusIcon(status) {
  const { name, tone } = STATUS_ICON[status] || STATUS_ICON.running;
  return html`<span class="shrink-0 ${tone}" role="img" aria-label="${status}" title="${status}"><hq-icon name="${name}" size="sm"></hq-icon></span>`;
}

function formatTime(iso) {
  if (!iso) return '';
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString();
}

function formatDuration(ms) {
  if (ms < 1000) return `${ms} ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)} s`;
  const minutes = Math.floor(seconds / 60);
  const remaining = Math.round(seconds % 60);
  return `${minutes}m ${remaining}s`;
}

/* ===== Helpers ===== */
function section(title, content) {
  return html`
    <section>
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-body">${title}</h2>
      ${content}
    </section>
  `;
}

function sourceTone(source) {
  if (source === 'server') return 'mauve';
  if (source === 'webhook') return 'rust';
  return 'amber';
}

function badge(text, tone) {
  const el = document.createElement('hq-badge');
  el.setAttribute('tone', tone);
  el.textContent = text;
  return el;
}

function spinner() {
  return '<div class="flex justify-center py-8"><hq-spinner></hq-spinner></div>';
}

function errorState(title, message) {
  return html`<hq-empty-state icon="alert" title="${title}">
    <span>${message}</span>
  </hq-empty-state>`.value;
}

function toValue(result) {
  return result.value;
}

/* ===== Init ===== */
const params = new URLSearchParams(window.location.search);
const loopId = params.get('loop');
const channelId = params.get('channel');

if (loopId) {
  showLoopDetail(loopId);
} else if (channelId) {
  showChannelDetail(channelId);
} else {
  const tab = params.get('tab');
  showList(TAB_PANELS[tab] ? tab : 'activity');
}
