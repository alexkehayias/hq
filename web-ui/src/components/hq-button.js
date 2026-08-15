/**
 * <hq-button> — styled button with variants.
 *
 * @attr {string} variant - primary|secondary|ghost|danger (default: primary)
 * @attr {boolean} disabled - renders disabled state
 * @slot default - button label
 *
 * Light DOM: renders a real <button> so it is focusable, keyboard-activatable
 * (Enter/Space), and announced correctly by assistive tech. The host keeps
 * inline-flex so layout utility classes (e.g. mt-4) still apply around it.
 */
class HqButton extends HTMLElement {
  static observedAttributes = ['variant', 'disabled'];

  #initialized = false;

  connectedCallback() {
    if (this.#initialized) return;
    this.#initialized = true;

    const fragment = document.createDocumentFragment();
    while (this.firstChild) fragment.appendChild(this.firstChild);

    this.style.display = 'inline-flex';
    this.innerHTML = '<button></button>';
    const btn = this.querySelector('button');
    btn.type = 'button';
    btn.appendChild(fragment);
    this.#update();
  }

  attributeChangedCallback() {
    if (this.#initialized) this.#update();
  }

  #update() {
    const btn = this.querySelector('button');
    if (!btn) return;
    const variant = this.getAttribute('variant') || 'primary';
    const disabled = this.hasAttribute('disabled');
    btn.disabled = disabled;
    const variants = {
      primary:
        'bg-blue-600 text-white hover:bg-blue-700 dark:bg-blue-500 dark:hover:bg-blue-600',
      secondary:
        'bg-gray-200 text-gray-900 hover:bg-gray-300 dark:bg-gray-700 dark:text-gray-100 dark:hover:bg-gray-600',
      ghost:
        'bg-transparent text-blue-600 hover:bg-blue-50 dark:text-blue-400 dark:hover:bg-gray-700',
      danger:
        'bg-red-600 text-white hover:bg-red-700 dark:bg-red-500 dark:hover:bg-red-600',
    };
    btn.className = [
      'inline-flex items-center justify-center rounded-md px-4 py-2 text-sm font-medium transition-colors',
      variants[variant] || variants.primary,
      disabled ? 'opacity-50 cursor-not-allowed' : '',
    ]
      .filter(Boolean)
      .join(' ');
  }
}

customElements.define('hq-button', HqButton);
