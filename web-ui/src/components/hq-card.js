/**
 * <hq-card> — surface container with optional header.
 *
 * @attr {string} max-width - 2xl|4xl|6xl|full (default: 4xl)
 * @slot header - optional header content
 * @slot default - body
 *
 * Light DOM: renders a <section> with header (if slot provided) and body.
 */
class HqCard extends HTMLElement {
  static observedAttributes = ['max-width'];

  #initialized = false;

  connectedCallback() {
    if (this.#initialized) return;
    this.#initialized = true;

    const fragment = document.createDocumentFragment();
    while (this.firstChild) fragment.appendChild(this.firstChild);

    this.innerHTML = `
      <section class="max-w-4xl mx-auto bg-white dark:bg-gray-800 rounded-2xl shadow-xl overflow-hidden">
        <div data-hq-header class="hidden px-6 py-8 border-b border-gray-100 dark:border-gray-700"></div>
        <div data-hq-body class="px-6 py-8"></div>
      </section>
    `;

    const header = this.querySelector('[data-hq-header]');
    const body = this.querySelector('[data-hq-body]');

    for (const node of [...fragment.childNodes]) {
      if (node.nodeType === 1 && node.getAttribute('slot') === 'header') {
        header.appendChild(node);
        header.classList.remove('hidden');
      } else {
        body.appendChild(node);
      }
    }

    this.#updateMaxWidth();
  }

  attributeChangedCallback() {
    if (this.#initialized) this.#updateMaxWidth();
  }

  #updateMaxWidth() {
    const mw = this.getAttribute('max-width') || '4xl';
    const section = this.querySelector('section');
    if (section) {
      const widths = {
        '2xl': 'max-w-2xl',
        '4xl': 'max-w-4xl',
        '6xl': 'max-w-6xl',
        full: 'max-w-full',
      };
      section.className = `${widths[mw] || widths['4xl']} mx-auto bg-white dark:bg-gray-800 rounded-2xl shadow-xl overflow-hidden`;
    }
  }
}

customElements.define('hq-card', HqCard);
