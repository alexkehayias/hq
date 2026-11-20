/**
 * <hq-loop-run> — one loop run row.
 *
 * @attr {string} run - JSON string of a loop run: { status, loop_id,
 *   loop_title, channel_id, started_at, duration_ms, error?, response?,
 *   event, session_id, has_transcript }
 * @attr {boolean} show-loop - show the title of the loop that produced the run
 *
 * The row is a disclosure with two separate affordances:
 *
 * - Clicking the heading expands the run's full event, error, and response
 *   output. Collapsed rows show the heading plus a one-line gist of the run's
 *   output.
 * - The trailing "Continue" link opens the run's chat transcript, when one
 *   still exists.
 *
 * Used by the loop detail view and the Activity tab.
 */
import { html } from '/components/lib/html.js';

const STATUS_ICON = {
  success: { name: 'check', tone: 'text-moss' },
  error: { name: 'close', tone: 'text-rust' },
  interrupted: { name: 'alert', tone: 'text-amber' },
  running: { name: 'spinner', tone: 'text-teal animate-spin' },
};

/** Wash and text color per detail level. */
const SEVERITY = {
  event: { wash: 'bg-stone-100', text: 'text-body', label: 'text-muted' },
  warn: {
    wash: 'bg-amber-tint',
    text: 'text-amber-ink',
    label: 'text-amber-ink',
  },
  error: { wash: 'bg-rust-tint', text: 'text-rust', label: 'text-rust' },
};

const LEVEL_LABEL = { event: 'Event', warn: 'Warn', error: 'Error' };

let nextDetailsId = 0;

function statusIcon(status) {
  const { name, tone } = STATUS_ICON[status] || STATUS_ICON.running;
  return html`<span class="inline-flex shrink-0 ${tone}" role="img" aria-label="${status}" title="${status}"><hq-icon name="${name}" size="sm"></hq-icon></span>`;
}

function formatTime(iso) {
  if (!iso) return '';
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString();
}

function formatRelativeTime(iso) {
  if (!iso) return '';
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;

  const seconds = Math.floor((Date.now() - date.getTime()) / 1000);
  if (seconds < 45) return 'just now';
  if (seconds < 3600) return `${Math.max(1, Math.floor(seconds / 60))}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  if (seconds < 604800) return `${Math.floor(seconds / 86400)}d ago`;
  return date.toLocaleDateString();
}

/** Milliseconds under a second, otherwise seconds with one decimal. */
function formatDuration(ms) {
  if (ms == null) return '—';
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

/** First sentence of `text`, with runs of whitespace collapsed. */
function firstSentence(text) {
  const collapsed = String(text || '')
    .replace(/\s+/g, ' ')
    .trim();
  if (!collapsed) return '';
  const match = collapsed.match(/^.*?[.!?](?=\s|$)/);
  return match ? match[0] : collapsed;
}

/** A timer poll that produced no output is a quiet poll. */
function isQuietPoll(run) {
  return (
    /^\s*tick\b/.test(run.event || '') ||
    String(run.channel_id || '').startsWith('timer-')
  );
}

/** One-line summary shown under a collapsed heading. */
function runGist(run) {
  const output = String(run.error || run.response || '').trim();
  if (output) return firstSentence(output);
  if (isQuietPoll(run)) return 'quiet poll';
  return firstSentence(run.event || '');
}

/** Classify a channel message by its log level, if it carries one. */
function levelOf(text) {
  const firstLine = String(text || '').split('\n', 1)[0];
  if (/\bERROR\b/.test(firstLine)) return 'error';
  if (/\bWARN(?:ING)?\b/.test(firstLine)) return 'warn';
  return 'event';
}

function detailBlock(label, text, severity = 'event') {
  const theme = SEVERITY[severity];
  return html`
    <section>
      <h4 class="text-[11px] font-semibold uppercase tracking-wider ${theme.label}">${label}</h4>
      <pre class="mt-1 whitespace-pre-wrap break-words rounded-[10px] p-2.5 font-mono text-xs ${theme.wash} ${theme.text}">${text}</pre>
    </section>
  `;
}

class HqLoopRun extends HTMLElement {
  static observedAttributes = ['run', 'show-loop'];

  #expanded = false;
  #detailsId = `loop-run-details-${++nextDetailsId}`;

  #render(run, showLoop) {
    const expanded = this.#expanded;
    const level = levelOf(run.event);
    const gist = runGist(run);

    return html`
      <div class="rounded-[14px] border border-line bg-card">
        <div class="flex items-start gap-1 px-2 py-1.5">
          <button type="button" data-toggle aria-expanded="${String(expanded)}" aria-controls="${this.#detailsId}"
            class="flex min-w-0 flex-1 cursor-pointer items-start gap-2 rounded-[10px] px-1.5 py-1 text-left transition-colors hover:bg-mist focus:outline-none focus-visible:ring-2 focus-visible:ring-teal">
            <span class="mt-0.5 shrink-0">${statusIcon(run.status)}</span>
            <span class="flex min-w-0 flex-1 flex-col gap-0.5 md:flex-row md:items-center md:gap-2">
              <span class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                ${showLoop ? html`<span class="text-sm font-medium text-ink">${run.loop_title}</span>` : ''}
                <span class="font-mono text-xs text-muted" title="${formatTime(run.started_at)}">${formatRelativeTime(run.started_at)}</span>
              </span>
              ${
                gist
                  ? html`<span data-gist class="flex min-w-0 items-center gap-2 md:flex-1">
                      <span class="hidden select-none text-muted md:inline" aria-hidden="true">·</span>
                      <span class="min-w-0 flex-1 truncate font-mono text-xs text-muted">${gist}</span>
                    </span>`
                  : ''
              }
            </span>
          </button>
          ${
            run.has_transcript
              ? html`<a href="/chat/index.html?session_id=${encodeURIComponent(run.session_id)}"
                       aria-label="Continue this session" title="Continue this session"
                       class="inline-flex shrink-0 items-center justify-center rounded-[9px] p-1.5 text-muted transition-colors hover:bg-teal-tint hover:text-teal-strong focus:outline-none focus:ring-2 focus:ring-teal">
                  <hq-icon name="chevron-right" size="sm"></hq-icon>
                </a>`
              : ''
          }
        </div>

        <div data-details id="${this.#detailsId}" class="${expanded ? '' : 'hidden'} max-h-[60vh] overflow-auto">
          <div class="flex flex-col gap-3 pb-3 pl-[38px] pr-3 pt-0.5">
            <dl class="flex flex-wrap gap-x-6 gap-y-1 font-mono text-xs">
              <div class="flex items-baseline gap-2">
                <dt class="text-[11px] font-semibold uppercase tracking-wider text-muted">Channel</dt>
                <dd class="text-body">${run.channel_id}</dd>
              </div>
              <div class="flex items-baseline gap-2">
                <dt class="text-[11px] font-semibold uppercase tracking-wider text-muted">Duration</dt>
                <dd class="text-body">${formatDuration(run.duration_ms)}</dd>
              </div>
            </dl>
            ${run.event ? detailBlock(LEVEL_LABEL[level], run.event, level) : ''}
            ${run.error ? detailBlock('Run error', run.error, 'error') : ''}
            ${run.response ? detailBlock('Response', run.response, 'event') : ''}
          </div>
        </div>
      </div>
    `;
  }

  #update() {
    let run;
    try {
      run = JSON.parse(this.getAttribute('run') || '{}');
    } catch {
      run = {};
    }
    this.innerHTML = this.#render(run, this.hasAttribute('show-loop')).value;
  }

  #setExpanded(expanded) {
    this.#expanded = expanded;
    this.querySelector('[data-toggle]')?.setAttribute(
      'aria-expanded',
      String(expanded),
    );
    this.querySelector('[data-details]')?.classList.toggle('hidden', !expanded);
  }

  connectedCallback() {
    // Custom elements default to inline; block lets the host participate in
    // the parent's flex/space-y layout like a normal row.
    this.style.display = 'block';
    this.#update();
  }

  constructor() {
    super();
    // Delegated so the listener survives the innerHTML re-renders in #update.
    this.addEventListener('click', (event) => {
      const toggle = event.target.closest('[data-toggle]');
      if (!toggle || !this.contains(toggle)) return;
      this.#setExpanded(!this.#expanded);
    });
  }

  attributeChangedCallback() {
    this.#update();
  }
}

customElements.define('hq-loop-run', HqLoopRun);
