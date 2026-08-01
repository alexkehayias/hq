/**
 * <hq-pagination> — page navigation with a 5-page window.
 *
 * @attr {number} page - current page (1-indexed)
 * @attr {number} total-pages - total number of pages
 *
 * @fires page-change - dispatched with { detail: { page } } when a page is clicked (bubbles)
 *
 * Renders prev button, 5-page window around current page, next button.
 * Active page styled as bg-blue-500 text-white. Replaces the inline
 * onclick="loadSessions(N)" globals in chat/sessions/index.js.
 */
class HqPagination extends HTMLElement {
  static observedAttributes = ['page', 'total-pages'];

  #update() {
    const page = parseInt(this.getAttribute('page') || '1', 10);
    const totalPages = parseInt(this.getAttribute('total-pages') || '1', 10);

    if (totalPages <= 1) {
      this.innerHTML = '';
      return;
    }

    const maxVisible = 5;
    let start = Math.max(1, page - Math.floor(maxVisible / 2));
    const end = Math.min(totalPages, start + maxVisible - 1);
    if (end - start + 1 < maxVisible) {
      start = Math.max(1, end - maxVisible + 1);
    }

    let html = '<nav class="flex justify-center items-center space-x-2 mt-4">';

    if (page > 1) {
      html += `<button data-page="${page - 1}" class="px-3 py-1 border rounded hover:bg-gray-100 dark:hover:bg-gray-700 text-sm">Previous</button>`;
    }

    for (let i = start; i <= end; i++) {
      if (i === page) {
        html += `<span class="px-3 py-1 border rounded bg-blue-500 text-white text-sm">${i}</span>`;
      } else {
        html += `<button data-page="${i}" class="px-3 py-1 border rounded hover:bg-gray-100 dark:hover:bg-gray-700 text-sm">${i}</button>`;
      }
    }

    if (page < totalPages) {
      html += `<button data-page="${page + 1}" class="px-3 py-1 border rounded hover:bg-gray-100 dark:hover:bg-gray-700 text-sm">Next</button>`;
    }

    html += '</nav>';
    this.innerHTML = html;
  }

  connectedCallback() {
    this.#update();
    if (!this.#listenerAttached) {
      this.addEventListener('click', (e) => {
        const btn = e.target.closest('[data-page]');
        if (!btn) return;
        const newPage = parseInt(btn.dataset.page, 10);
        this.setAttribute('page', String(newPage));
        this.dispatchEvent(
          new CustomEvent('page-change', {
            detail: { page: newPage },
            bubbles: true,
          }),
        );
      });
      this.#listenerAttached = true;
    }
  }

  attributeChangedCallback() {
    this.#update();
  }

  #listenerAttached = false;
}

customElements.define('hq-pagination', HqPagination);
