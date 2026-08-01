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

  #update() {
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
    const loose = [];
    const namedAction = [];
    for (const el of [...this.children]) {
      if (el.getAttribute('slot') === 'action') namedAction.push(el);
      else loose.push(el);
    }
    if (loose.length) {
      parts.push(
        '<p class="text-sm text-gray-600 dark:text-gray-400 mb-4">' +
          loose.map((el) => el.textContent || '').join(' ') +
          '</p>',
      );
    }
    if (namedAction.length) {
      parts.push('<div class="mt-4">' + namedAction[0].outerHTML + '</div>');
    }
    parts.push('</div>');
    this.innerHTML = parts.join('');
  }

  connectedCallback() {
    this.#update();
  }

  attributeChangedCallback() {
    this.#update();
  }
}

customElements.define('hq-empty-state', HqEmptyState);
