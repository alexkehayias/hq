/**
 * <hq-modal> — full-screen overlay modal.
 *
 * @attr {boolean} open - when present, the modal is visible
 * @slot default - modal body content
 *
 * @fires close - dispatched on Escape key or backdrop click (bubbles)
 *
 * Light DOM: renders an overlay with a centered card. The page sets `open`
 * to show and listens for `close` to hide.
 */
class HqModal extends HTMLElement {
  static observedAttributes = ['open'];

  #initialized = false;
  #abort = null;

  connectedCallback() {
    if (this.#initialized) return;
    this.#initialized = true;

    const fragment = document.createDocumentFragment();
    while (this.firstChild) fragment.appendChild(this.firstChild);

    this.innerHTML = `
      <div data-hq-backdrop class="fixed inset-0 bg-black/85 z-[10000] flex items-start sm:items-center justify-center p-4 sm:p-6 overflow-y-auto" style="display:none">
        <div data-hq-card class="max-w-2xl w-full bg-white dark:bg-gray-800 rounded-xl shadow-2xl p-5 my-8">
        </div>
      </div>
    `;

    const card = this.querySelector('[data-hq-card]');
    card.appendChild(fragment);

    this.querySelector('[data-hq-backdrop]').addEventListener('click', (e) => {
      if (e.target === e.currentTarget) this.#close();
    });

    this.#abort = new AbortController();
    document.addEventListener(
      'keydown',
      (e) => {
        if (e.key === 'Escape' && this.hasAttribute('open')) this.#close();
      },
      { signal: this.#abort.signal },
    );

    this.#updateDisplay();
  }

  disconnectedCallback() {
    this.#abort?.abort();
  }

  attributeChangedCallback() {
    if (this.#initialized) this.#updateDisplay();
  }

  #close() {
    this.removeAttribute('open');
    this.dispatchEvent(new CustomEvent('close', { bubbles: true }));
  }

  #updateDisplay() {
    const backdrop = this.querySelector('[data-hq-backdrop]');
    if (backdrop) {
      backdrop.style.display = this.hasAttribute('open') ? '' : 'none';
    }
  }
}

customElements.define('hq-modal', HqModal);
