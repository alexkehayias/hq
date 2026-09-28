/**
 * <hq-select> — styled dropdown wrapping a native <select>.
 *
 * @attr {string} label - optional visible label text (set once at connect)
 * @attr {string} value - current selected value (reflected on change)
 * @attr {boolean} disabled - disables the select
 * @slot default - <option> elements
 *
 * @fires change - bubbles; dispatched with { detail: { value } } when the
 *   selection changes.
 *
 * Light DOM: renders a real <select> with appearance-none so the native arrow
 * is hidden and replaced by a themed <hq-icon> chevron. Keeping the native
 * <select> preserves keyboard navigation, screen-reader behavior, and form
 * semantics; the chevron is themed for light/dark via Tailwind.
 */
import '/components/hq-icon.js';

class HqSelect extends HTMLElement {
  static observedAttributes = ['value', 'disabled'];

  #initialized = false;

  connectedCallback() {
    if (this.#initialized) return;
    this.#initialized = true;

    const fragment = document.createDocumentFragment();
    while (this.firstChild) fragment.appendChild(this.firstChild);

    this.innerHTML = `
      <label class="block text-sm font-medium text-body">
        <span class="hq-select-label mb-1 block"></span>
        <span class="relative block">
          <select class="w-full appearance-none rounded-[12px] border border-line bg-card px-3 py-2 pr-9 text-sm text-ink focus:outline-none focus:ring-2 focus:ring-teal"></select>
          <span class="pointer-events-none absolute inset-y-0 right-0 flex items-center pr-3 text-muted"><hq-icon name="chevron-down" size="sm"></hq-icon></span>
        </span>
      </label>
    `;

    const label = this.getAttribute('label');
    const labelEl = this.querySelector('.hq-select-label');
    if (label) {
      labelEl.textContent = label;
    } else {
      labelEl.remove();
    }

    const select = this.querySelector('select');
    select.appendChild(fragment);

    select.addEventListener('change', (e) => {
      e.stopPropagation();
      this.setAttribute('value', select.value);
      this.dispatchEvent(
        new CustomEvent('change', {
          detail: { value: select.value },
          bubbles: true,
        }),
      );
    });

    this.#update();
  }

  attributeChangedCallback(name) {
    if (!this.#initialized) return;
    if (name === 'value' || name === 'disabled') this.#update();
  }

  #update() {
    const select = this.querySelector('select');
    if (!select) return;
    const value = this.getAttribute('value');
    if (value !== null) select.value = value;
    select.disabled = this.hasAttribute('disabled');
  }
}

customElements.define('hq-select', HqSelect);
