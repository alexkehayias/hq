/**
 * <hq-stat-card> — KPI stat card with label and value.
 *
 * @attr {string} label - small label text (e.g. "Total Tokens")
 * @attr {string} value - large value text (e.g. "1.2M")
 * @slot default - optional formatted value (overrides `value` attr)
 *
 * CSS-only: renders a rounded card with small label and large value.
 * Replaces the three duplicated stat cards on the metrics page
 * (#totalTokens, #avgTokensPerDay, #estCost).
 */
class HqStatCard extends HTMLElement {
  static observedAttributes = ['label', 'value'];

  #initialized = false;
  #originalBody = '';

  #esc(s) {
    return String(s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
  }

  connectedCallback() {
    if (this.#initialized) return;
    this.#initialized = true;

    // Capture slotted children BEFORE building structure
    const loose = [];
    for (const el of [...this.children]) {
      if (el.getAttribute('slot') !== 'action') loose.push(el);
    }
    this.#originalBody =
      loose.length > 0
        ? loose.map((el) => el.outerHTML).join('')
        : this.#esc(this.getAttribute('value') || '');

    // Clear and build
    this.innerHTML = '';
    const div = document.createElement('div');
    div.className = 'bg-white dark:bg-gray-800 rounded-lg p-4 shadow-sm';
    div.innerHTML = `
      <p class="text-sm text-gray-500 dark:text-gray-400"></p>
      <p class="text-2xl font-semibold text-gray-900 dark:text-white mt-1"></p>
    `;
    this.appendChild(div);
    this.#update();
  }

  attributeChangedCallback() {
    if (this.#initialized) this.#update();
  }

  #update() {
    const label = this.getAttribute('label') || '';
    const valueAttr = this.getAttribute('value');
    const body = this.#originalBody || this.#esc(valueAttr || '');

    const labelP = this.querySelector('p:first-child');
    const valueP = this.querySelector('p:last-child');
    if (labelP) labelP.textContent = label;
    if (valueP) valueP.innerHTML = body;
  }
}

customElements.define('hq-stat-card', HqStatCard);
