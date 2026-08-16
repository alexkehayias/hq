/**
 * <hq-empty-state> — centered empty/placeholder message with optional action.
 *
 * @attr {string} icon - optional hq-icon name (e.g. "search", "alert")
 * @attr {string} title - heading text
 * @slot default - description text (optional)
 * @slot action - optional button or link
 *
 * CSS-only: renders a centered column. Used inside <hq-state-view slot="empty">.
 * The action slot element is moved (not cloned) on each render so any event
 * listeners attached to it keep working.
 */
import { html } from '/components/lib/html.js';

class HqEmptyState extends HTMLElement {
  static observedAttributes = ['icon', 'title'];

  #initialized = false;
  #loose = [];
  #action = null;

  connectedCallback() {
    if (this.#initialized) return;
    this.#initialized = true;

    // Capture original slot children once — #render() only rebuilds the shell
    for (const el of [...this.children]) {
      if (el.getAttribute('slot') === 'action') {
        this.#action = el;
      } else {
        this.#loose.push(el);
      }
    }

    this.#render();
  }

  attributeChangedCallback() {
    if (this.#initialized) this.#render();
  }

  #render() {
    const icon = this.getAttribute('icon');
    const title = this.getAttribute('title');

    const result = html`<div class="text-center py-8">
      ${
        icon
          ? html`<div class="flex justify-center mb-4 text-gray-400"><hq-icon name="${icon}" size="lg"></hq-icon></div>`
          : null
      }
      ${
        title
          ? html`<h3 class="text-lg font-semibold text-gray-900 dark:text-white mb-2">${title}</h3>`
          : null
      }
      ${
        this.#loose.length
          ? html`<p class="text-sm text-gray-600 dark:text-gray-400 mb-4">${this.#loose.map((el) => el.textContent || '').join(' ')}</p>`
          : null
      }
      ${this.#action ? html`<div data-hq-action class="mt-4"></div>` : null}
    </div>`;
    this.innerHTML = result.value;

    // Move the real action element in so its listeners survive re-renders.
    if (this.#action) {
      this.querySelector('[data-hq-action]').appendChild(this.#action);
    }
  }
}

customElements.define('hq-empty-state', HqEmptyState);
