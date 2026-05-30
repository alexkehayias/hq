// Skills page: list view and detail view with CodeMirror editor

/* ===== State ===== */
let currentSkill = null;
let currentFile = null;
let editor = null;
const fileData = {}; // { path: content }
let hasChanges = false;

/* ===== Init ===== */
const params = new URLSearchParams(window.location.search);
const skillName = params.get('name');

if (skillName) {
  showDetailView(skillName);
} else {
  showListView();
}

// Dark mode sync for CodeMirror
const darkMode = window.matchMedia('(prefers-color-scheme: dark)');
darkMode.addEventListener('change', syncTheme);
syncTheme();

function syncTheme() {
  const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
  const themeLink = document.getElementById('cm-theme');
  if (themeLink) {
    themeLink.disabled = !isDark;
  }
}

/* ===== List View ===== */
async function showListView() {
  document.getElementById('list-view').classList.remove('hidden');
  document.getElementById('detail-view').classList.add('hidden');
  await loadSkills();
}

async function loadSkills() {
  const loading = document.getElementById('list-loading');
  const error = document.getElementById('list-error');
  const empty = document.getElementById('list-empty');
  const grid = document.getElementById('skill-grid');

  loading.classList.remove('hidden');
  error.classList.add('hidden');
  empty.classList.add('hidden');
  grid.classList.add('hidden');

  try {
    const res = await fetch('/api/skills');
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();
    loading.classList.add('hidden');

    if (!data.skills || data.skills.length === 0) {
      empty.classList.remove('hidden');
      return;
    }

    grid.innerHTML = data.skills
      .map(
        (skill) => `
      <a href="/skills/?name=${encodeURIComponent(skill.name)}"
         class="block p-5 bg-gray-50 dark:bg-gray-700 hover:bg-blue-50 dark:hover:bg-gray-600 rounded-xl transition-all duration-200 border border-transparent hover:border-blue-200 dark:hover:border-blue-800">
        <h3 class="font-semibold text-gray-900 dark:text-white truncate">${escHtml(skill.name)}</h3>
        <p class="mt-1 text-sm text-gray-500 dark:text-gray-400 line-clamp-2">${escHtml(skill.description)}</p>
      </a>
    `,
      )
      .join('');
    grid.classList.remove('hidden');
  } catch (err) {
    loading.classList.add('hidden');
    document.getElementById('list-error-msg').textContent =
      `Failed to load skills: ${err.message}`;
    error.classList.remove('hidden');
  }
}

document.getElementById('list-retry').addEventListener('click', loadSkills);

/* ===== Detail View ===== */
async function showDetailView(name) {
  document.getElementById('list-view').classList.add('hidden');
  document.getElementById('detail-view').classList.remove('hidden');

  const loading = document.getElementById('detail-loading');
  const content = document.getElementById('detail-content');
  const error = document.getElementById('detail-error');
  const body = document.getElementById('detail-body');

  loading.classList.remove('hidden');
  content.classList.add('hidden');
  error.classList.add('hidden');
  body.classList.add('hidden');

  try {
    const [skillRes, filesRes] = await Promise.all([
      fetch(`/api/skills/${encodeURIComponent(name)}`),
      fetch(`/api/skills/${encodeURIComponent(name)}/files`),
    ]);

    if (!skillRes.ok) {
      const errData = await skillRes.json().catch(() => ({}));
      throw new Error(errData.error || `Skill '${name}' not found`);
    }

    currentSkill = await skillRes.json();
    loading.classList.add('hidden');
    content.classList.remove('hidden');

    // Render header
    document.getElementById('skill-name').textContent = currentSkill.name;
    document.getElementById('skill-description').textContent =
      currentSkill.description;

    const badges = document.getElementById('skill-badges');
    badges.innerHTML = '';
    if (currentSkill.license) {
      badges.appendChild(badge(currentSkill.license, 'blue'));
    }
    if (currentSkill.compatibility) {
      badges.appendChild(badge(currentSkill.compatibility, 'green'));
    }

    // Render file tree
    if (filesRes.ok) {
      const filesData = await filesRes.json();
      renderFileTree(filesData.files || []);
    }

    body.classList.remove('hidden');

    // Auto-open SKILL.md if available
    const skillFile = fileData['SKILL.md'];
    if (skillFile !== undefined) {
      openFile('SKILL.md');
    }
  } catch (err) {
    loading.classList.add('hidden');
    document.getElementById('detail-error-msg').textContent =
      `Failed to load skill: ${err.message}`;
    error.classList.remove('hidden');
  }
}

/* ===== File Tree ===== */
async function renderFileTree(files) {
  const tree = document.getElementById('file-tree');
  const loading = document.getElementById('file-tree-loading');

  // Pre-fetch all file contents
  const fetchPromises = files
    .filter((f) => !f.is_directory)
    .map(async (f) => {
      try {
        const res = await fetch(
          `/api/skills/${encodeURIComponent(currentSkill.name)}/files/${encodeURIComponent(f.path)}`,
        );
        if (res.ok) {
          const data = await res.json();
          fileData[f.path] = data.content;
        }
      } catch (_) {
        /* skip unreadable files */
      }
    });

  await Promise.all(fetchPromises);

  loading.classList.add('hidden');
  tree.classList.remove('hidden');

  // Sort: SKILL.md first, then directories first, then alphabetical
  const sorted = [...files].sort((a, b) => {
    if (a.path === 'SKILL.md') return -1;
    if (b.path === 'SKILL.md') return 1;
    if (a.is_directory && !b.is_directory) return -1;
    if (!a.is_directory && b.is_directory) return 1;
    return a.path.localeCompare(b.path);
  });

  // Group by directory prefix
  const dirs = {};
  const rootFiles = [];
  for (const f of sorted) {
    if (f.is_directory) {
      dirs[f.path] = [];
    } else if (f.path.includes('/')) {
      const dir = f.path.substring(0, f.path.lastIndexOf('/'));
      if (!dirs[dir]) dirs[dir] = [];
      dirs[dir].push(f);
    } else {
      rootFiles.push(f);
    }
  }

  let html = '';
  // Root files
  for (const f of rootFiles) {
    html += fileTreeItem(f.path, fileData[f.path] !== undefined);
  }
  // Directory groups
  for (const [dir, dirFiles] of Object.entries(dirs)) {
    if (dirFiles.length === 0) continue;
    html += `<div class="border-t border-gray-200 dark:border-gray-600 mt-2 pt-2">`;
    html += `<div class="px-4 py-1 text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase tracking-wider">${escHtml(dir)}/</div>`;
    for (const f of dirFiles) {
      html += fileTreeItem(f.path, fileData[f.path] !== undefined);
    }
    html += `</div>`;
  }

  tree.innerHTML = html;

  // Click handlers
  tree.querySelectorAll('[data-file-path]').forEach((el) => {
    el.addEventListener('click', () => openFile(el.dataset.filePath));
  });
}

function fileTreeItem(path, hasContent) {
  const name = path.substring(path.lastIndexOf('/') + 1);
  const icon = hasContent
    ? `<svg class="h-4 w-4 text-gray-400 dark:text-gray-500 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m2.25 0H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 00-9-9z" /></svg>`
    : `<svg class="h-4 w-4 text-gray-300 dark:text-gray-600 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke-width="1.5" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m0 12l-3-3m0 0l3-3m-3 3h12" /></svg>`;
  return `<button data-file-path="${escAttr(path)}" class="file-tree-item w-full text-left px-4 py-1.5 text-sm text-gray-700 dark:text-gray-300 hover:bg-blue-100 dark:hover:bg-gray-600 flex items-center gap-2 transition-colors ${currentFile === path ? 'bg-blue-100 dark:bg-gray-600 font-medium' : ''}">${icon}<span class="truncate">${escHtml(name)}</span></button>`;
}

/* ===== Editor ===== */
function openFile(path) {
  const content = fileData[path];
  if (content === undefined) return;

  currentFile = path;
  hasChanges = false;
  updateSaveButton();

  // Highlight active file in tree
  document.querySelectorAll('.file-tree-item').forEach((el) => {
    el.classList.remove('bg-blue-100', 'dark:bg-gray-600', 'font-medium');
    if (el.dataset.filePath === path) {
      el.classList.add('bg-blue-100', 'dark:bg-gray-600', 'font-medium');
    }
  });

  document.getElementById('editor-filename').textContent = path;

  const container = document.getElementById('editor-container');
  container.innerHTML = '';

  const mode = modeForPath(path);
  const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;

  editor = CodeMirror(container, {
    value: content,
    mode: mode,
    theme: isDark ? 'dracula' : 'default',
    lineNumbers: true,
    indentUnit: 2,
    tabSize: 2,
    lineWrapping: true,
    extraKeys: {
      'Ctrl-S': () => saveFile(),
      'Cmd-S': () => saveFile(),
    },
  });

  editor.on('change', () => {
    if (!hasChanges) {
      hasChanges = true;
      updateSaveButton();
    }
  });

  // Refresh after a tick to ensure proper sizing
  setTimeout(() => editor.refresh(), 100);
}

/* ===== Save ===== */
document.getElementById('save-btn').addEventListener('click', saveFile);

async function saveFile() {
  if (!currentFile || !hasChanges || !editor) return;

  const btn = document.getElementById('save-btn');
  btn.disabled = true;
  btn.textContent = 'Saving...';

  try {
    const res = await fetch(
      `/api/skills/${encodeURIComponent(currentSkill.name)}/files/${encodeURIComponent(currentFile)}`,
      {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ content: editor.getValue() }),
      },
    );

    if (!res.ok) {
      const errData = await res.json().catch(() => ({}));
      throw new Error(errData.error || `HTTP ${res.status}`);
    }

    // Update cached content
    fileData[currentFile] = editor.getValue();
    hasChanges = false;

    btn.textContent = 'Saved!';
    setTimeout(() => {
      updateSaveButton();
    }, 2000);
  } catch (err) {
    btn.textContent = `Error: ${err.message}`;
    btn.disabled = false;
    setTimeout(() => updateSaveButton(), 3000);
  }
}

function updateSaveButton() {
  const btn = document.getElementById('save-btn');
  if (!hasChanges || !currentFile) {
    btn.disabled = true;
    btn.textContent = 'Save';
  } else {
    btn.disabled = false;
    btn.textContent = 'Save';
  }
}

/* ===== Helpers ===== */
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
  return 'markdown';
}

function escHtml(s) {
  const div = document.createElement('div');
  div.textContent = s;
  return div.innerHTML;
}

function escAttr(s) {
  return s.replace(/"/g, '"').replace(/'/g, '&#39;');
}

function badge(text, color) {
  const colors = {
    blue: 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-300',
    green: 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-300',
    purple:
      'bg-purple-100 text-purple-800 dark:bg-purple-900 dark:text-purple-300',
  };
  const el = document.createElement('span');
  el.className = `inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium ${colors[color] || colors.blue}`;
  el.textContent = text;
  return el;
}
