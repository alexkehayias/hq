/**
 * <hq-code-editor> — CodeMirror-backed file editor with a save toolbar.
 *
 * Page-specific to the skills page; CodeMirror is a global loaded via <script>
 * tags by the page (see skills/index.html), so this module only imports hq-button.
 *
 * @method open(filename, value) - load a file; resets dirty state
 * @method getValue() - current editor content
 * @method setSaving() - disable Save + show "Saving..." (during network save)
 * @method setSaved() - mark content saved, show "Saved!"; restores after 2s
 * @method setSaveError(msg) - show error, re-enable Save; restores after 3s
 *
 * @fires save-request - Save button click or Ctrl/Cmd-S when dirty (bubbles)
 * @fires change - { dirty } when dirty state flips (bubbles)
 *
 * Light DOM: renders the toolbar (filename + Save button) and the editor area.
 * The page listens for save-request, performs the API call, then calls
 * setSaving()/setSaved()/setSaveError() to reflect the result.
 */
import '/components/hq-button.js';

class HqCodeEditor extends HTMLElement {
  #editor = null;
  #dirty = false;
  #savedValue = '';
  #initialized = false;
  #restoreTimer = null;
  #container = null;
  #saveBtn = null;
  #saveLabel = null;
  #filenameEl = null;

  connectedCallback() {
    if (this.#initialized) return;
    this.#initialized = true;

    this.style.display = 'flex';
    this.style.flexDirection = 'column';
    this.style.flex = '1';
    this.style.minHeight = '0';

    this.innerHTML = `
      <div class="flex items-center justify-between px-4 py-2 bg-gray-50 dark:bg-gray-700 border-b border-gray-200 dark:border-gray-600">
        <span data-hq-filename class="text-sm font-medium text-gray-700 dark:text-gray-300 truncate"></span>
        <hq-button data-hq-save variant="primary" disabled><span data-hq-save-label>Save</span></hq-button>
      </div>
      <div data-hq-container class="flex-1 min-h-[400px]"></div>
    `;

    this.#filenameEl = this.querySelector('[data-hq-filename]');
    this.#container = this.querySelector('[data-hq-container]');
    this.#saveBtn = this.querySelector('[data-hq-save]');
    this.#saveLabel = this.querySelector('[data-hq-save-label]');

    this.#saveBtn.addEventListener('click', () => this.#requestSave());
    window.addEventListener('resize', () => this.#resize());
  }

  disconnectedCallback() {
    if (this.#restoreTimer) clearTimeout(this.#restoreTimer);
  }

  open(filename, value) {
    this.#filenameEl.textContent = filename;
    this.#ensureEditor();
    this.#editor.setOption('mode', modeForPath(filename));
    this.#savedValue = value || '';
    this.#editor.setValue(value || '');
    this.#dirty = false;
    this.#updateButton();
    this.#resize();
  }

  getValue() {
    return this.#editor ? this.#editor.getValue() : '';
  }

  setSaving() {
    this.#saveBtn.setAttribute('disabled', '');
    this.#saveLabel.textContent = 'Saving...';
  }

  setSaved() {
    this.#savedValue = this.#editor.getValue();
    this.#setDirty(false);
    this.#saveLabel.textContent = 'Saved!';
    this.#scheduleRestore(2000);
  }

  setSaveError(msg) {
    this.#saveBtn.removeAttribute('disabled');
    this.#saveLabel.textContent = `Error: ${msg}`;
    this.#scheduleRestore(3000);
  }

  #ensureEditor() {
    if (this.#editor) return;
    const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    this.#editor = CodeMirror(this.#container, {
      value: '',
      mode: modeForPath(this.#filenameEl.textContent),
      theme: isDark ? 'dracula' : 'default',
      lineNumbers: false,
      indentUnit: 2,
      tabSize: 2,
      lineWrapping: true,
      extraKeys: {
        'Ctrl-S': () => this.#requestSave(),
        'Cmd-S': () => this.#requestSave(),
      },
    });
    this.#editor.on('change', () => {
      const dirty = this.#editor.getValue() !== this.#savedValue;
      this.#setDirty(dirty);
    });
  }

  #requestSave() {
    if (!this.#dirty) return;
    this.dispatchEvent(new CustomEvent('save-request', { bubbles: true }));
  }

  #setDirty(dirty) {
    if (this.#dirty === dirty) return;
    this.#dirty = dirty;
    this.#updateButton();
    this.dispatchEvent(
      new CustomEvent('change', { detail: { dirty }, bubbles: true }),
    );
  }

  #updateButton() {
    this.#saveBtn.setAttribute('disabled', '');
    this.#saveLabel.textContent = 'Save';
    if (this.#dirty) this.#saveBtn.removeAttribute('disabled');
  }

  #scheduleRestore(ms) {
    if (this.#restoreTimer) clearTimeout(this.#restoreTimer);
    this.#restoreTimer = setTimeout(() => this.#updateButton(), ms);
  }

  #resize() {
    if (!this.#editor) return;
    const rect = this.#container.getBoundingClientRect();
    const height = window.innerHeight - rect.top - 16;
    this.#container.style.height = `${height}px`;
    this.#editor.setSize(null, height);
  }
}

customElements.define('hq-code-editor', HqCodeEditor);

function modeForPath(path) {
  const ext = path.split('.').pop().toLowerCase();
  const name = path.toLowerCase();
  if (name.endsWith('.md') || name.endsWith('.markdown')) return 'markdown';
  if (name.endsWith('.py')) return 'python';
  if (name.endsWith('.js') || name.endsWith('.mjs') || name.endsWith('.cjs'))
    return 'javascript';
  if (name.endsWith('.ts') || name.endsWith('.tsx')) return 'javascript';
  if (name.endsWith('.sh') || name.endsWith('.bash') || name.endsWith('.zsh'))
    return 'shell';
  if (name.endsWith('.yaml') || name.endsWith('.yml')) return 'yaml';
  if (name.endsWith('.html') || name.endsWith('.xml') || name.endsWith('.svg'))
    return 'xml';
  if (ext === 'css' || ext === 'json') return ext;
  return null;
}
