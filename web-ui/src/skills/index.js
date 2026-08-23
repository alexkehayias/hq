// Skills page: list view and detail view with CodeMirror editor
import { html } from '/components/lib/html.js';
import './code-editor.js';

/* ===== State ===== */
let currentSkill = null;
let currentFile = null;
const fileData = {}; // { path: content }

/* ===== Elements ===== */
const pageShell = document.getElementById('page-shell');
const listView = document.getElementById('list-view');
const detailView = document.getElementById('detail-view');
const listViewState = document.getElementById('list-view-state');
const listErrorMsg = document.getElementById('list-error-msg');
const skillGrid = document.getElementById('skill-grid');
const detailViewState = document.getElementById('detail-view-state');
const detailErrorMsg = document.getElementById('detail-error-msg');
const skillNameEl = document.getElementById('skill-name');
const skillDescEl = document.getElementById('skill-description');
const skillBadgesEl = document.getElementById('skill-badges');
const fileTree = document.getElementById('file-tree');
const codeEditor = document.getElementById('code-editor');

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
  if (themeLink) themeLink.disabled = !isDark;
}

/* ===== List View ===== */
function showListView() {
  listView.classList.remove('hidden');
  detailView.classList.add('hidden');
  pageShell.setAttribute('meta-title', 'Skills');
  pageShell.setAttribute('back-href', '/');
  pageShell.setAttribute('back-label', 'Home');
  loadSkills();
}

async function loadSkills() {
  listViewState.setAttribute('state', 'loading');
  try {
    const res = await fetch('/api/skills');
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = await res.json();

    if (!data.skills || data.skills.length === 0) {
      listViewState.setAttribute('state', 'empty');
      return;
    }

    skillGrid.innerHTML = data.skills
      .map(
        (skill) => html`
          <a href="/skills/?name=${encodeURIComponent(skill.name)}"
             class="block p-5 bg-gray-50 dark:bg-gray-700 hover:bg-blue-50 dark:hover:bg-gray-600 rounded-xl transition-all duration-200 border border-transparent hover:border-blue-200 dark:hover:border-blue-800">
            <h3 class="font-semibold text-gray-900 dark:text-white truncate">${skill.name}</h3>
            <p class="mt-1 text-sm text-gray-500 dark:text-gray-400 line-clamp-2">${skill.description}</p>
          </a>
        `,
      )
      .map((r) => r.value)
      .join('');

    listViewState.setAttribute('state', 'content');
  } catch (err) {
    listErrorMsg.textContent = `Failed to load skills: ${err.message}`;
    listViewState.setAttribute('state', 'error');
  }
}

document.getElementById('list-retry').addEventListener('click', loadSkills);

/* ===== Detail View ===== */
async function showDetailView(name) {
  listView.classList.add('hidden');
  detailView.classList.remove('hidden');
  pageShell.setAttribute('meta-title', name);
  pageShell.setAttribute('back-href', '/skills/');
  pageShell.setAttribute('back-label', 'Skills');

  detailViewState.setAttribute('state', 'loading');
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

    // Render header
    skillNameEl.textContent = currentSkill.name;
    skillDescEl.textContent = currentSkill.description;

    skillBadgesEl.innerHTML = '';
    if (currentSkill.license) {
      skillBadgesEl.appendChild(badge(currentSkill.license, 'blue'));
    }
    if (currentSkill.compatibility) {
      skillBadgesEl.appendChild(badge(currentSkill.compatibility, 'green'));
    }

    // Render file tree
    if (filesRes.ok) {
      const filesData = await filesRes.json();
      await renderFileTree(filesData.files || []);
    }

    detailViewState.setAttribute('state', 'content');

    // Auto-open SKILL.md if available
    if (fileData['SKILL.md'] !== undefined) {
      openFile('SKILL.md');
    }
  } catch (err) {
    detailErrorMsg.textContent = `Failed to load skill: ${err.message}`;
    detailViewState.setAttribute('state', 'error');
  }
}

/* ===== File Tree ===== */
async function renderFileTree(files) {
  // Pre-fetch all file contents so files open instantly
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

  fileTree.setAttribute(
    'files',
    JSON.stringify(
      files.map((f) => ({ path: f.path, is_directory: f.is_directory })),
    ),
  );
}

fileTree.addEventListener('file-select', (e) => {
  openFile(e.detail.path);
});

/* ===== Editor ===== */
function openFile(path) {
  const content = fileData[path];
  if (content === undefined) return;

  currentFile = path;
  fileTree.setAttribute('selected', path);
  codeEditor.open(path, content);
}

/* ===== Save ===== */
codeEditor.addEventListener('save-request', saveFile);

async function saveFile() {
  if (!currentFile) return;

  codeEditor.setSaving();

  try {
    const res = await fetch(
      `/api/skills/${encodeURIComponent(currentSkill.name)}/files/${encodeURIComponent(currentFile)}`,
      {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ content: codeEditor.getValue() }),
      },
    );

    if (!res.ok) {
      const errData = await res.json().catch(() => ({}));
      throw new Error(errData.error || `HTTP ${res.status}`);
    }

    fileData[currentFile] = codeEditor.getValue();
    codeEditor.setSaved();
  } catch (err) {
    codeEditor.setSaveError(err.message);
  }
}

/* ===== Helpers ===== */
function badge(text, tone) {
  const el = document.createElement('hq-badge');
  el.setAttribute('tone', tone);
  el.textContent = text;
  return el;
}
