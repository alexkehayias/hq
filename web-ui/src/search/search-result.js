/**
 * <hq-search-result> — single note/task/chat result row in the search list.
 *
 * @attr {string} result - JSON string: { id, title?, tags?, is_task, task_status?, type, file_name? }
 * @attr {boolean} selected - highlight the row
 *
 * Renders an <li> with a task-status or chat icon, the title, and #tag chips.
 * Replaces the inline template in search/index.js.
 *
 * @fires result-select - on click, with { result } (bubbles). The page handles
 *   highlighting, URL updates, and opening the note modal.
 */
import { html } from '/components/lib/html.js';

const TASK_ICONS = {
  todo: '⬜',
  next: '⏭️',
  waiting: '⏳',
  canceled: '❌',
  done: '✅',
  someday: '🤷',
};

class HqSearchResult extends HTMLElement {
  static observedAttributes = ['result', 'selected'];

  #result = {};

  #update() {
    try {
      this.#result = JSON.parse(this.getAttribute('result') || '{}');
    } catch {
      this.#result = {};
    }
    this.dataset.noteId = String(this.#result.id);

    const selected = this.hasAttribute('selected');
    const icon = this.#result.is_task
      ? TASK_ICONS[String(this.#result.task_status || '').toLowerCase()] || ''
      : this.#result.type === 'chat'
        ? '💬'
        : '';

    const tags = (this.#result.tags || '')
      .split(',')
      .filter(Boolean)
      .map(
        (t) =>
          html`<span class="bg-gray-200 dark:bg-gray-700 text-gray-700 dark:text-gray-300 text-xs px-2 py-0.5 rounded-full mr-2">#${t}</span>`,
      );

    const markup = html`<li class="group flex justify-between cursor-default select-none items-center rounded-md px-3 py-2 hover:cursor-pointer ${selected ? 'bg-blue-700 text-white' : ''}">
      <div class="flex space-x-2">
        ${icon ? html`<span class="py-0.5 text-gray-800 text-xs rounded-full">${icon}</span>` : null}
        <span class="line-clamp-1">${this.#result.title}</span>
      </div>
      ${tags.length ? html`<div class="flex flex-row">${tags}</div>` : null}
    </li>`;
    this.innerHTML = markup.value;
  }

  connectedCallback() {
    // Custom elements default to inline; block keeps the host a proper list
    // item so container spacing (e.g. divide-y) applies between rows.
    this.style.display = 'block';
    this.addEventListener('click', () => {
      this.dispatchEvent(
        new CustomEvent('result-select', {
          bubbles: true,
          detail: { result: this.#result },
        }),
      );
    });
    this.#update();
  }

  attributeChangedCallback() {
    this.#update();
  }
}

customElements.define('hq-search-result', HqSearchResult);
