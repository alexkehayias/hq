/**
 * <hq-file-tree> — file list sidebar with directory expansion.
 *
 * @attr {string} files - JSON string: [{path, is_directory}]
 * @attr {string} selected - file path currently highlighted
 *
 * @fires file-select - dispatched with { detail: { path } } when a file is clicked (bubbles)
 *
 * Builds a tree from a flat file list. Sorts with SKILL.md first, then
 * directories before files. Directory expansion state is internal.
 */
import { html } from '/components/lib/html.js';

function buildTree(files) {
  const root = { name: '', path: '', is_dir: true, children: [] };
  for (const f of files) {
    const parts = f.path.split('/').filter(Boolean);
    let node = root;
    for (let i = 0; i < parts.length; i++) {
      const isLast = i === parts.length - 1;
      let child = node.children.find((c) => c.name === parts[i]);
      if (!child) {
        child = {
          name: parts[i],
          path: parts.slice(0, i + 1).join('/'),
          is_dir: !isLast ? true : !!f.is_directory,
          children: [],
        };
        node.children.push(child);
      }
      node = child;
    }
  }
  return root;
}

function sortChildren(children) {
  return [...children].sort((a, b) => {
    if (a.name === 'SKILL.md') return -1;
    if (b.name === 'SKILL.md') return 1;
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

function collectDirs(node, acc = []) {
  for (const c of node.children || []) {
    if (c.is_dir) acc.push(c);
    collectDirs(c, acc);
  }
  return acc;
}

function renderNode(node, openPaths, selectedPath) {
  const isSelected = !node.is_dir && node.path === selectedPath;
  if (node.is_dir) {
    const isOpen = openPaths.has(node.path);
    return html`<li class="block">
      <button
        class="flex items-center gap-1 w-full px-2 py-1 text-sm rounded hover:bg-gray-100 dark:hover:bg-gray-700 text-left ${isOpen ? 'font-semibold' : ''}"
        data-path=${node.path}
        data-kind="dir"
      >
        <span class="text-xs w-3">${isOpen ? '▼' : '▶'}</span>
        <span>📁</span>
        <span>${node.name}</span>
      </button>
      ${
        isOpen && node.children.length
          ? html`<ul class="ml-3 border-l border-gray-200 dark:border-gray-700">${sortChildren(node.children).map((c) => renderNode(c, openPaths, selectedPath))}</ul>`
          : null
      }
    </li>`;
  }
  return html`<li class="block">
    <button
      class="flex items-center gap-1 w-full px-2 py-1 text-sm rounded ${isSelected ? 'bg-blue-50 dark:bg-blue-900/30 text-blue-700 dark:text-blue-300' : 'hover:bg-gray-100 dark:hover:bg-gray-700'} text-left"
      data-path=${node.path}
      data-kind="file"
    >
      <span class="text-xs w-3"></span>
      <span>📄</span>
      <span>${node.name}</span>
    </button>
  </li>`;
}

class HqFileTree extends HTMLElement {
  static observedAttributes = ['files', 'selected'];

  #openPaths = new Set();
  #tree = null;
  #listenerAttached = false;

  attributeChangedCallback(name) {
    if (name === 'files') this.#tree = null;
    this.render();
  }

  connectedCallback() {
    if (!this.#listenerAttached) {
      this.addEventListener('click', (e) => {
        const btn = e.target.closest('[data-path]');
        if (!btn) return;
        const { path, kind } = btn.dataset;
        if (kind === 'dir') {
          this.#openPaths.has(path)
            ? this.#openPaths.delete(path)
            : this.#openPaths.add(path);
          this.render();
        } else {
          this.setAttribute('selected', path);
          this.dispatchEvent(
            new CustomEvent('file-select', {
              detail: { path },
              bubbles: true,
            }),
          );
        }
      });
      this.#listenerAttached = true;
    }
    const tree = this.#getTree();
    if (this.#openPaths.size === 0) {
      for (const d of collectDirs(tree)) this.#openPaths.add(d.path);
    }
    this.render();
  }

  #getTree() {
    if (this.#tree) return this.#tree;
    const raw = this.getAttribute('files');
    try {
      this.#tree = buildTree(JSON.parse(raw || '[]'));
    } catch {
      this.#tree = { name: '', path: '', is_dir: true, children: [] };
    }
    return this.#tree;
  }

  render() {
    const tree = this.#getTree();
    if (!tree.children.length) {
      this.innerHTML =
        '<div class="px-2 py-4 text-sm text-gray-400">No files</div>';
      return;
    }
    const selected = this.getAttribute('selected') || '';
    const result = html`<ul class="text-gray-700 dark:text-gray-300 text-sm select-none">${sortChildren(tree.children).map((c) => renderNode(c, this.#openPaths, selected))}</ul>`;
    this.innerHTML = result.value;
  }
}

customElements.define('hq-file-tree', HqFileTree);
