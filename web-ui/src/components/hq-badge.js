/**
 * <hq-badge> — colored pill badge.
 *
 * @attr {string} tone - blue|green|yellow|red|gray (default: gray)
 * @slot default - badge text
 *
 * CSS-only: Tailwind classes on the host. Replaces the inline badge() helper
 * in skills/index.js and tag chip markup across pages.
 */
class HqBadge extends HTMLElement {
  static observedAttributes = ['tone'];

  #update() {
    const tone = this.getAttribute('tone') || 'gray';
    const tones = {
      blue: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-300',
      green:
        'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-300',
      yellow:
        'bg-yellow-100 text-yellow-800 dark:bg-yellow-900 dark:text-yellow-300',
      red: 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-300',
      gray: 'bg-gray-100 text-gray-800 dark:bg-gray-700 dark:text-gray-300',
    };
    this.className = [
      'px-2.5 py-0.5 rounded-full text-xs font-medium',
      tones[tone] || tones.gray,
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

customElements.define('hq-badge', HqBadge);
