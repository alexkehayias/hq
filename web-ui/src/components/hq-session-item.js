/**
 * <hq-session-item> — chat session list row.
 *
 * @attr {string} session - JSON string: { id, title?, summary?, tags? }
 *
 * Renders an article with title (falls back to "Session {id}"), optional
 * tag chips (<hq-badge tone="blue">), summary (falls back to italic
 * "Summary not available."), and a "View »" link. Replaces the inline
 * template in chat/sessions/index.js.
 */
import { html, SafeHtml } from '/components/lib/html.js';

class HqSessionItem extends HTMLElement {
  static observedAttributes = ['session'];

  #update() {
    let session;
    try {
      session = JSON.parse(this.getAttribute('session') || '{}');
    } catch {
      session = {};
    }
    const title = session.title || `Session ${session.id}`;
    const id = String(session.id);
    const summary = session.summary
      ? session.summary
      : new SafeHtml('<i>Summary not available.</i>');
    const tags = (session.tags || []).map(
      (t) => html`<hq-badge tone="blue">${t}</hq-badge>`,
    );
    const result = html`<article class="border border-gray-200 dark:border-gray-700 rounded-lg p-4 bg-white dark:bg-gray-800">
      <h2 class="font-semibold text-gray-900 dark:text-white">${title}</h2>
      ${tags.length ? html`<div class="flex flex-wrap gap-2 mt-2">${tags}</div>` : null}
      <p class="text-sm text-gray-600 dark:text-gray-400 mt-2">${summary}</p>
      <a href="/chat/index.html?session_id=${id}" class="text-sm text-blue-500 hover:underline mt-2 inline-block">View »</a>
    </article>`;
    this.innerHTML = result.value;
  }

  connectedCallback() {
    this.#update();
  }

  attributeChangedCallback() {
    this.#update();
  }
}

customElements.define('hq-session-item', HqSessionItem);
