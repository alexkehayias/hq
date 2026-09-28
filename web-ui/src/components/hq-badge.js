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
    const teal = 'bg-teal-tint text-teal-strong';
    const moss = 'bg-moss-tint text-moss';
    const amber = 'bg-amber-tint text-amber-ink';
    const rust = 'bg-rust-tint text-rust';
    const mauve = 'bg-mauve-tint text-mauve';
    const gray = 'bg-mist text-muted';
    // Legacy tone names map onto the Riverstone accents so existing callers
    // keep working.
    const tones = {
      teal,
      moss,
      amber,
      rust,
      mauve,
      blue: teal,
      green: moss,
      yellow: amber,
      red: rust,
      gray,
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
