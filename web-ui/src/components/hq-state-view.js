/**
 * <hq-state-view> — state-driven slot switcher (loading/error/empty/content).
 *
 * @attr {string} state - loading|error|empty|content (default: content)
 * @slot loading - shown when state="loading" (defaults to <hq-spinner> if not provided)
 * @slot error - shown when state="error"
 * @slot empty - shown when state="empty"
 * @slot content - shown when state="content" (also: any unnamed child)
 *
 * Light DOM: original children are sorted into named slots on connect.
 * Non-active slots get display:none. The page toggles the `state` attr;
 * no events emitted.
 */
class HqStateView extends HTMLElement {
  static observedAttributes = ['state'];

  #initialized = false;

  connectedCallback() {
    if (this.#initialized) return;
    this.#initialized = true;

    const loose = [];
    const named = {};
    for (const el of [...this.children]) {
      const slot = el.getAttribute('slot');
      if (slot) {
        named[slot] = el;
      } else {
        loose.push(el);
      }
    }

    if (loose.length && !named.content) {
      const wrap = document.createElement('div');
      wrap.setAttribute('slot', 'content');
      for (const el of loose) wrap.appendChild(el);
      named.content = wrap;
      this.appendChild(wrap);
    }

    if (!named.loading) {
      const l = document.createElement('div');
      l.setAttribute('slot', 'loading');
      l.className = 'flex justify-center py-12';
      l.innerHTML =
        '<hq-spinner></hq-spinner><p class="ml-3 text-gray-600 dark:text-gray-400">Loading...</p>';
      this.appendChild(l);
    }

    this.#render();
  }

  attributeChangedCallback() {
    if (this.#initialized) this.#render();
  }

  #render() {
    const state = this.getAttribute('state') || 'content';
    for (const el of this.children) {
      const slot = el.getAttribute('slot') || 'content';
      el.style.display = slot === state ? '' : 'none';
    }
  }
}

customElements.define('hq-state-view', HqStateView);
