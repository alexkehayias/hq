/**
 * <hq-empty-state> — centered empty/placeholder message with optional action.
 *
 * @attr {string} icon - optional hq-icon name (e.g. "search", "alert")
 * @attr {string} title - heading text
 * @slot default - description text (optional)
 * @slot action - optional button or link
 *
 * CSS-only: renders a centered column. Used inside <hq-state-view slot="empty">.
 */
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

    const parts = ['<div class="text-center py-8">'];
    if (icon) {
      parts.push(
        '<div class="flex justify-center mb-4 text-gray-400"><hq-icon name="' +
          icon +
          '" size="lg"></hq-icon></div>',
      );
    }
    if (title) {
      parts.push(
        '<h3 class="text-lg font-semibold text-gray-900 dark:text-white mb-2">' +
          title +
          '</h3>',
      );
    }
    if (this.#loose.length) {
      parts.push(
        '<p class="text-sm text-gray-600 dark:text-gray-400 mb-4">' +
          this.#loose.map((el) => el.textContent || '').join(' ') +
          '</p>',
      );
    }
    if (this.#action) {
      parts.push(`<div class="mt-4">${this.#action.outerHTML}</div>`);
    }
    parts.push('</div>');
    this.innerHTML = parts.join('');
  }
}

customElements.define('hq-empty-state', HqEmptyState);
