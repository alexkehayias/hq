const escapeHtml = (value) =>
  String(value).replace(
    /[&<>"']/g,
    (c) =>
      ({
        '&': '&amp;',
        '<': '&lt;',
        '>': '&gt;',
        '"': '&quot;',
        "'": '&#39;',
      })[c],
  );

class AttachmentBubble extends HTMLElement {
  constructor() {
    super();
    this.files = [];
  }

  static get observedAttributes() {
    return ['files'];
  }

  attributeChangedCallback(name, oldValue, newValue) {
    if (name === 'files' && oldValue !== newValue) {
      try {
        this.files = JSON.parse(newValue || '[]');
      } catch (e) {
        console.error('Invalid files attribute:', e);
        this.files = [];
      }
      this.render();
    }
  }

  connectedCallback() {
    this.render();
  }

  render() {
    if (this.files.length === 0) {
      this.innerHTML = '';
      return;
    }

    this.innerHTML = `
      <div class="flex justify-end mb-4">
        <div class="flex flex-col items-end gap-2 max-w-full">
          ${this.files.map((file) => this.renderFile(file)).join('')}
        </div>
      </div>
    `;
  }

  renderFile(file) {
    const filename = escapeHtml(file.filename);

    if (file.content_type?.startsWith('image/')) {
      return `<img src="${file.objectUrl}" alt="${filename}" class="max-h-64 max-w-full rounded-xl border border-blue-200 dark:border-blue-700">`;
    }

    return `
      <div class="flex items-center gap-2 py-2 px-4 bg-blue-50 dark:bg-blue-900/30 border border-blue-200 dark:border-blue-700 rounded-xl">
        <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor" class="w-5 h-5 shrink-0 text-gray-500 dark:text-gray-400">
          <path stroke-linecap="round" stroke-linejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H8.25m2.25 0H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z" />
        </svg>
        <span class="text-sm text-gray-900 dark:text-gray-100 truncate">${filename}</span>
      </div>
    `;
  }
}

customElements.define('attachment-bubble', AttachmentBubble);

export default AttachmentBubble;
