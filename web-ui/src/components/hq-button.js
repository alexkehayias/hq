/**
 * <hq-button> — styled button with variants.
 *
 * @attr {string} variant - primary|secondary|ghost|danger (default: primary)
 * @attr {boolean} disabled - renders disabled state
 * @slot default - button label
 *
 * CSS-only: no HTM, no shadow DOM. Tailwind classes on the host element.
 */
class HqButton extends HTMLElement {
  static observedAttributes = ['variant', 'disabled'];

  #update() {
    const variant = this.getAttribute('variant') || 'primary';
    const disabled = this.hasAttribute('disabled');
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
    this.className = [
      'inline-flex items-center justify-center rounded-md px-4 py-2 text-sm font-medium transition-colors',
      variants[variant] || variants.primary,
      disabled ? 'opacity-50 cursor-not-allowed' : '',
    ]
      .filter(Boolean)
      .join(' ');
  }

  connectedCallback() {
    this.#update();
  }

  attributeChangedCallback() {
    this.#update();
  }
}

customElements.define('hq-button', HqButton);
