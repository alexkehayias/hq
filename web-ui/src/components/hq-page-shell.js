/**
 * <hq-page-shell> — page wrapper with gradient background and "Back to" link.
 *
 * @attr {string} meta-title - sets document.title / browser tab (e.g. "Skills - HQ")
 * @attr {string} back-href - URL for the back link (default "/")
 * @attr {string} back-label - destination label, rendered as "Back to {label}" (default "Home")
 * @slot default - page body, placed inside <main>
 *
 * Light DOM: original children are moved into a <main> container on first connect.
 */
class HqPageShell extends HTMLElement {
  static observedAttributes = ['meta-title', 'back-href', 'back-label'];

  #initialized = false;

  connectedCallback() {
    if (this.#initialized) return;
    this.#initialized = true;

    const fragment = document.createDocumentFragment();
    while (this.firstChild) fragment.appendChild(this.firstChild);

    this.innerHTML = `
      <div class="min-h-dvh bg-gradient-to-br from-blue-50 via-white to-purple-50 dark:from-gray-900 dark:via-gray-800 dark:to-gray-900">
        <div class="flex flex-col items-start px-4 sm:px-6 lg:px-8 pt-6 pb-12 max-w-7xl mx-auto">
          <a id="hq-back-link" href="/" class="inline-flex items-center text-blue-600 dark:text-blue-400 hover:text-blue-800 dark:hover:text-blue-300 transition-colors mb-6">
            <svg class="h-5 w-5 mr-2" fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 19.5L8.25 12l7.5-7.5" />
            </svg>
            <span id="hq-back-label">Back to Home</span>
          </a>
          <main class="w-full max-w-6xl mx-auto text-gray-900 dark:text-gray-200"></main>
        </div>
      </div>
    `;

    this.querySelector('main').appendChild(fragment);
    this.#updateAttrs();
  }

  attributeChangedCallback() {
    if (this.#initialized) this.#updateAttrs();
  }

  #updateAttrs() {
    const title = this.getAttribute('meta-title');
    if (title) document.title = `${title} - HQ`;

    const link = this.querySelector('#hq-back-link');
    if (link) {
      link.setAttribute('href', this.getAttribute('back-href') || '/');
      const label = this.querySelector('#hq-back-label');
      if (label) {
        const backLabel = this.getAttribute('back-label') || 'Home';
        label.textContent = `Back to ${backLabel}`;
      }
    }
  }
}

customElements.define('hq-page-shell', HqPageShell);
