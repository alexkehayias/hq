/**
 * <hq-terminal> — terminal-style command input with a scrolling output log.
 *
 * @attr {string} prompt - prompt string shown before the input (default: "$")
 * @attr {string} welcome - optional muted line printed when the element connects
 * @attr {string} height - CSS height for the whole terminal (default: "320px")
 * @attr {number} max-lines - maximum output rows kept in the DOM; older rows
 *   are removed (default: unlimited)
 * @attr {boolean} disabled - disables the input
 *
 * @method print(text, tone?) - append output; `text` may contain newlines.
 *   tone: default|muted|error|success (default: default)
 * @method clear() - remove all output lines
 * @method focus() - focus the input
 * @method setPrompt(text) - update the prompt string
 *
 * @fires command - bubbles; dispatched on Enter with { detail: { line } }
 *   after the line has been echoed to the output.
 *
 * Light DOM: a monospace scrollback log above a prompt row. The component
 * knows nothing about what a command means — the host page listens for
 * `command`, does the work, and calls print() with the results.
 */
const TONES = {
  default: 'text-[#cdd8d1]',
  muted: 'text-[#7d9288]',
  error: 'text-[#e59a78]',
  success: 'text-[#9fc57a]',
};

class HqTerminal extends HTMLElement {
  static observedAttributes = ['prompt', 'height', 'max-lines', 'disabled'];

  #initialized = false;
  #root = null;
  #output = null;
  #input = null;
  #promptEl = null;
  #history = [];
  #historyIndex = 0;

  connectedCallback() {
    if (this.#initialized) return;
    this.#initialized = true;

    // Custom elements are inline by default, so a height would be ignored.
    this.style.display = 'block';
    this.#applyHeight();

    this.innerHTML = `
      <div data-hq-root class="flex h-full flex-col overflow-hidden rounded-card border border-line bg-code font-mono text-[13px] leading-tight shadow-sm">
        <div data-hq-output class="min-h-0 flex-1 overflow-y-auto px-4 py-2" role="log" aria-live="polite" aria-label="Terminal output"></div>
        <form data-hq-form class="m-0 flex items-center gap-2 border-t border-white/10 px-4 py-2 text-[16px] xl:text-[13px]">
          <span data-hq-prompt class="shrink-0 select-none font-normal text-[var(--color-code-fg)]"></span>
          <input data-hq-input class="min-w-0 flex-1 bg-transparent text-[#e8efe9] caret-teal-bright outline-none placeholder:text-[#5d7167]" type="text" autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false" aria-label="Terminal input">
        </form>
      </div>
    `;

    this.#root = this.querySelector('[data-hq-root]');
    this.#output = this.querySelector('[data-hq-output]');
    this.#input = this.querySelector('[data-hq-input]');
    this.#promptEl = this.querySelector('[data-hq-prompt]');
    const form = this.querySelector('[data-hq-form]');

    this.#updatePrompt();
    this.#updateDisabled();

    form.addEventListener('submit', (e) => {
      e.preventDefault();
      this.#submit();
    });

    this.#input.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        this.#navigateHistory(-1);
      } else if (e.key === 'ArrowDown') {
        e.preventDefault();
        this.#navigateHistory(1);
      } else if (e.key.toLowerCase() === 'l' && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        this.clear();
      }
    });

    // Clicking anywhere in the terminal focuses the input, unless the user is
    // selecting text.
    this.#root.addEventListener('click', () => {
      if (window.getSelection()?.toString()) return;
      this.#input.focus();
    });

    const welcome = this.getAttribute('welcome');
    if (welcome) this.print(welcome, 'muted');
  }

  attributeChangedCallback(name) {
    if (!this.#initialized) return;
    if (name === 'prompt') this.#updatePrompt();
    if (name === 'height') this.#applyHeight();
    if (name === 'max-lines') this.#trim();
    if (name === 'disabled') this.#updateDisabled();
  }

  print(text, tone = 'default') {
    for (const line of String(text).split('\n')) {
      this.#appendLine(line, tone);
    }
  }

  clear() {
    this.#output.replaceChildren();
  }

  focus() {
    this.#input?.focus();
  }

  setPrompt(text) {
    this.setAttribute('prompt', text);
  }

  #submit() {
    const line = this.#input.value;
    if (!line.trim()) return;

    this.#input.value = '';
    this.#history.push(line);
    this.#historyIndex = this.#history.length;
    this.#echo(line);
    this.dispatchEvent(
      new CustomEvent('command', { detail: { line }, bubbles: true }),
    );
  }

  #echo(line) {
    const row = document.createElement('div');
    row.className = 'flex gap-2';

    const prompt = document.createElement('span');
    prompt.className =
      'shrink-0 select-none font-normal text-[var(--color-code-fg)]';
    prompt.textContent = this.#promptText();

    const text = document.createElement('span');
    text.className = 'break-words whitespace-pre-wrap text-[#e8efe9]';
    text.textContent = line;

    row.append(prompt, text);
    this.#output.appendChild(row);
    this.#trim();
    this.#scrollToBottom();
  }

  #appendLine(text, tone) {
    const line = document.createElement('div');
    line.className = `min-h-[1.25em] break-words whitespace-pre-wrap ${TONES[tone] || TONES.default}`;
    line.textContent = text;
    this.#output.appendChild(line);
    this.#trim();
    this.#scrollToBottom();
  }

  // Bound the DOM for long-running shells; drop the oldest rows first.
  #trim() {
    const max = Number.parseInt(this.getAttribute('max-lines'), 10);
    if (!Number.isFinite(max) || max <= 0) return;
    while (this.#output.childElementCount > max) {
      this.#output.firstElementChild.remove();
    }
  }

  #navigateHistory(delta) {
    if (this.#history.length === 0) return;

    this.#historyIndex = Math.min(
      Math.max(this.#historyIndex + delta, 0),
      this.#history.length,
    );
    this.#input.value = this.#history[this.#historyIndex] ?? '';
    this.#input.setSelectionRange(
      this.#input.value.length,
      this.#input.value.length,
    );
  }

  #scrollToBottom() {
    this.#output.scrollTop = this.#output.scrollHeight;
  }

  #promptText() {
    return this.getAttribute('prompt') ?? '$';
  }

  #updatePrompt() {
    this.#promptEl.textContent = this.#promptText();
  }

  #updateDisabled() {
    this.#input.disabled = this.hasAttribute('disabled');
  }

  #applyHeight() {
    const height = this.getAttribute('height');
    if (height) this.style.height = height;
    else if (!this.style.height) this.style.height = '320px';
  }
}

customElements.define('hq-terminal', HqTerminal);
