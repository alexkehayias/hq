/**
 * <hq-loop-run> — one loop run row.
 *
 * @attr {string} run - JSON string of a loop run: { status, loop_id,
 *   channel_id, started_at, duration_ms, error?, response?, event,
 *   session_id, has_transcript }
 * @attr {boolean} show-loop - show the id of the loop that produced the run
 *
 * Renders the status icon, loop/channel, start time, duration, and a preview
 * of the run's error, response, or event. Links to the run's transcript when
 * one still exists. Used by the loop detail view and the Activity tab.
 */
import { html } from '/components/lib/html.js';

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

class HqLoopRun extends HTMLElement {
  static observedAttributes = ['run', 'show-loop'];

  #render(run, showLoop) {
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

  #update() {
    let run;
    try {
      run = JSON.parse(this.getAttribute('run') || '{}');
    } catch {
      run = {};
    }
    this.innerHTML = this.#render(run, this.hasAttribute('show-loop')).value;
  }

  connectedCallback() {
    // Custom elements default to inline; block lets the host participate in
    // the parent's flex/space-y layout like a normal row.
    this.style.display = 'block';
    this.#update();
  }

  attributeChangedCallback() {
    this.#update();
  }
}

customElements.define('hq-loop-run', HqLoopRun);
