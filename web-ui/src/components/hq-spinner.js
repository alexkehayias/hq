/**
 * <hq-spinner> — loading spinner (CSS animation).
 *
 * @attr {string} size - sm|md|lg (default: md)
 * @attr {string} tone - optional color class (e.g. "text-blue-600"); inherits currentColor if unset
 *
 * CSS-only: Tailwind animate-spin. Used by <hq-state-view> default loading slot.
 */
class HqSpinner extends HTMLElement {
  static observedAttributes = ['size', 'tone'];

  #update() {
    const size = this.getAttribute('size') || 'md';
    const tone = this.getAttribute('tone');
    const sizes = {
      sm: 'h-4 w-4 border',
      md: 'h-8 w-8 border-b-2',
      lg: 'h-12 w-12 border-b-4',
    };
    this.className = [
      'inline-block animate-spin rounded-full border-blue-600 dark:border-blue-400',
      sizes[size] || sizes.md,
      tone || '',
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

customElements.define('hq-spinner', HqSpinner);
