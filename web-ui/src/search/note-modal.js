/**
 * <hq-note-modal> — full note viewer with task status editing.
 *
 * Page-specific to the search page. Wraps <hq-modal> and renders a note's
 * title, task status chips (with dropdowns), source, tags, and markdown body.
 * Uses the global `marked` loaded by the page.
 *
 * @method open(id) - fetch and display the note by id
 *
 * @fires close - fired when the modal is dismissed (backdrop, Escape, or X)
 *
 * Light DOM: renders an <hq-modal> with a content div. The page calls open()
 * and listens for close() to clear the note_id URL param.
 */
import '/components/hq-modal.js';
import { esc } from '/components/lib/html.js';

const CHIP_LABEL = {
  TODO: 'To do',
  NEXT: 'Next',
  WAITING: 'Waiting',
  DONE: 'Done',
  CANCELED: 'Canceled',
  SOMEDAY: 'Someday',
};

const CHIP_GROUP = {
  TODO: 'todo',
  NEXT: 'todo',
  WAITING: 'waiting',
  DONE: 'done',
  CANCELED: 'done',
  SOMEDAY: 'done',
};

const CHIP_INACTIVE = 'border-line bg-card text-muted';

const CHIP_COLOR = {
  todo: {
    active: 'border-teal-tint bg-teal-tint text-teal-strong',
    inactive: CHIP_INACTIVE,
  },
  waiting: {
    active: 'border-amber-tint bg-amber-tint text-amber-ink',
    inactive: CHIP_INACTIVE,
  },
  done: {
    active: 'border-moss-tint bg-moss-tint text-moss',
    inactive: CHIP_INACTIVE,
  },
};

const CHECK_SVG =
  '<svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2h11"/></svg>';
const CLOSE_SVG =
  '<svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M6 6l12 12M18 6L6 18"/></svg>';
const EDIT_SVG =
  '<svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 20h9M16.5 3.5a2.1 2.1 0 013 3L7 19l-4 1 1-4z"/></svg>';
const DELETE_SVG =
  '<svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m2 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6"/></svg>';

class HqNoteModal extends HTMLElement {
  #modal = null;
  #content = null;
  #noteData = null;
  #closeDropdowns = null;
  #initialized = false;

  connectedCallback() {
    if (this.#initialized) return;
    this.#initialized = true;

    this.innerHTML =
      '<hq-modal><div data-hq-content class="text-ink"></div></hq-modal>';
    this.#modal = this.querySelector('hq-modal');
    this.#content = this.querySelector('[data-hq-content]');

    this.#modal.addEventListener('close', () => {
      if (this.#closeDropdowns) {
        document.removeEventListener('click', this.#closeDropdowns);
        this.#closeDropdowns = null;
      }
      this.dispatchEvent(new CustomEvent('close', { bubbles: true }));
    });
  }

  async open(id) {
    this.#modal.setAttribute('open', '');
    this.#content.innerHTML =
      '<div class="text-center text-muted py-8">Loading...</div>';
    try {
      const resp = await fetch(`/api/notes/${id}/view`, {
        headers: { Accept: 'application/json' },
      });
      if (!resp.ok) throw new Error('Failed to fetch note');
      const data = await resp.json();
      this.#noteData = data;
      this.#render(data);
    } catch (err) {
      this.#content.innerHTML = `<div class="text-center text-rust py-8">Failed to load note: ${esc(err.message)}</div>`;
    }
  }

  #render(noteData) {
    const rawStatus = noteData.status;
    const isTask = noteData.type === 'task' && rawStatus;
    const status = isTask ? rawStatus.toUpperCase() : null;
    const currentChip = status ? CHIP_GROUP[status] || 'todo' : null;
    const isDone = currentChip === 'done';

    let html = '';

    // Header: task badge + close button
    html += '<div class="flex items-start justify-between mb-3.5">';
    html += isTask
      ? `<span class="inline-flex items-center gap-1.5 rounded-md bg-teal-tint px-2.5 py-1 text-xs font-medium text-teal-strong">${CHECK_SVG} Task</span>`
      : '<span></span>';
    html += `<button id="modal-close-btn" type="button" aria-label="Close" class="flex h-8 w-8 items-center justify-center rounded-md text-muted hover:bg-mist hover:text-ink focus:outline-none focus:ring-2 focus:ring-line">${CLOSE_SVG}</button>`;
    html += '</div>';

    // Title
    html += `<h2 class="text-lg font-medium leading-snug text-ink">${esc(noteData.title || '')}</h2>`;

    // Status chips with dropdowns (tasks only)
    if (isTask) {
      html += '<p class="mb-1.5 mt-4 text-xs text-muted">Status</p>';
      html += '<div class="flex gap-1.5">';
      const chips = [
        {
          key: 'todo',
          label: CHIP_LABEL[status] === 'Next' ? 'Next' : 'To do',
          dropdown: [{ status: 'NEXT', label: 'Next' }],
        },
        { key: 'waiting', label: 'Waiting', dropdown: [] },
        {
          key: 'done',
          label:
            CHIP_LABEL[status] === 'Canceled'
              ? 'Canceled'
              : CHIP_LABEL[status] === 'Someday'
                ? 'Someday'
                : 'Done',
          dropdown: [
            { status: 'CANCELED', label: 'Canceled' },
            { status: 'SOMEDAY', label: 'Someday' },
          ],
        },
      ];
      chips.forEach((chip) => {
        const active = currentChip === chip.key;
        const c = CHIP_COLOR[chip.key];
        const chipClasses = `flex-1 rounded-md border py-1.5 text-xs font-medium focus:outline-none focus:ring-2 focus:ring-teal ${active ? c.active : `${c.inactive} hover:bg-mist`}`;
        if (chip.dropdown.length > 0) {
          html += `<div class="relative flex-1">
            <div class="flex rounded-md border overflow-hidden ${active ? c.active : c.inactive}">
              <button type="button" data-status="${chip.key === 'todo' ? 'TODO' : 'DONE'}" class="flex-1 py-1.5 px-2 text-xs font-medium ${active ? c.active : c.inactive} focus:outline-none focus:ring-2 focus:ring-inset focus:ring-teal">${chip.label}</button>
              <button type="button" data-dropdown="${chip.key}" class="py-1.5 px-1 text-xs border-l ${active ? 'border-teal-tint text-teal-strong' : 'border-line text-muted'} hover:bg-mist focus:outline-none focus:ring-2 focus:ring-inset focus:ring-teal">
                <svg class="w-3 h-3" viewBox="0 0 20 20" fill="currentColor"><path d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z"/></svg>
              </button>
            </div>
            <div id="dropdown-${chip.key}" class="hidden absolute z-20 mt-1 w-full rounded-md bg-card shadow-lg ring-1 ring-line overflow-hidden">${chip.dropdown.map((item) => `<button type="button" data-status="${item.status}" class="flex w-full items-center gap-2 px-3 py-2 text-xs text-body hover:bg-mist">${item.label}</button>`).join('')}</div>
          </div>`;
        } else {
          html += `<button type="button" data-status="WAITING" class="${chipClasses}">${chip.label}</button>`;
        }
      });
      html += '</div>';
    }

    // Source
    if (noteData.file_name) {
      html += '<div class="mt-4 flex gap-2.5">';
      html += `<div class="flex-1 rounded-md bg-mist px-2.5 py-2">
        <p class="text-xs text-muted">Source</p>
        <p class="text-sm font-medium text-ink">${esc(noteData.file_name)}</p>
      </div>`;
      html += '</div>';
    }

    // Tags
    if (noteData.tags) {
      html += `<div class="mt-3">${noteData.tags
        .split(',')
        .map(
          (t) =>
            `<span class="inline-block mr-1.5 mb-1 bg-mist text-muted text-xs px-2 py-0.5 rounded-full">#${esc(t)}</span>`,
        )
        .join('')}</div>`;
    }

    // Note body — strip the title from the body to avoid duplication
    const bodyWithoutTitle = noteData.body
      ? noteData.body
          .replace(
            new RegExp(
              `^#+\\s*${noteData.title.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\n?`,
              'm',
            ),
            '',
          )
          .trim()
      : '';
    const messageHtml = marked.parse(bodyWithoutTitle, { breaks: true });
    html += '<p class="mb-1.5 mt-4 text-xs text-muted">Note</p>';
    html += `<div class="rounded-md bg-mist px-3 py-2.5 text-sm text-body markdown">${bodyWithoutTitle ? messageHtml : '<span class="italic text-muted">No additional content</span>'}</div>`;

    // Done confirmation strip
    if (isTask) {
      html += `<div id="done-hint" class="mt-4 flex items-center gap-1.5 rounded-md bg-moss-tint px-3 py-2 text-xs text-moss ${isDone ? '' : 'hidden'}">
        <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 13l4 4L19 7"/></svg>
        Marked done
      </div>`;
    }

    // Actions
    html += '<div class="mt-4 flex gap-2 border-t border-line pt-4">';
    html += `<button type="button" class="flex flex-1 items-center justify-center gap-1.5 rounded-md border border-line py-1.5 text-xs font-medium text-body hover:bg-mist focus:outline-none focus:ring-2 focus:ring-line">${EDIT_SVG} Edit</button>`;
    html += `<button type="button" class="flex flex-1 items-center justify-center gap-1.5 rounded-md border border-line py-1.5 text-xs font-medium text-rust hover:bg-rust-tint focus:outline-none focus:ring-2 focus:ring-rust">${DELETE_SVG} Delete</button>`;
    html += '</div>';

    this.#content.innerHTML = html;

    this.#content
      .querySelector('#modal-close-btn')
      .addEventListener('click', () => {
        this.#modal.removeAttribute('open');
        this.dispatchEvent(new CustomEvent('close', { bubbles: true }));
      });

    if (isTask) this.#wireTaskStatus();
  }

  #wireTaskStatus() {
    const content = this.#content;

    this.#closeDropdowns = (e) => {
      document.querySelectorAll('[id^="dropdown-"]').forEach((dd) => {
        if (
          !dd.classList.contains('hidden') &&
          !dd.parentElement.contains(e.target)
        ) {
          dd.classList.add('hidden');
        }
      });
    };
    document.addEventListener('click', this.#closeDropdowns);

    // Dropdown items carry data-status too, so this single selector covers both
    // the top-level chips and the menu options. Wiring each button once keeps a
    // click to one PATCH request.
    content.querySelectorAll('[data-status]').forEach((btn) => {
      btn.addEventListener('click', async () => {
        const dropdown = btn.closest('[id^="dropdown-"]');
        if (dropdown) dropdown.classList.add('hidden');
        await this.#updateStatus(btn.dataset.status);
      });
    });

    content.querySelectorAll('[data-dropdown]').forEach((btn) => {
      btn.addEventListener('click', (e) => {
        e.stopPropagation();
        const key = btn.dataset.dropdown;
        const dd = content.querySelector(`#dropdown-${key}`);
        document.querySelectorAll('[id^="dropdown-"]').forEach((d) => {
          if (d.id !== `dropdown-${key}`) d.classList.add('hidden');
        });
        dd.classList.toggle('hidden');
      });
    });
  }

  async #updateStatus(newStatus) {
    const content = this.#content;
    const noteData = this.#noteData;
    const newChip = CHIP_GROUP[newStatus] || 'todo';
    const newIsDone = newChip === 'done';
    const todoContainer = content.querySelector(
      '.flex.gap-1\\.5 > div:first-child',
    );
    const waitingBtn = content.querySelector(
      '.flex.gap-1\\.5 > button[data-status="WAITING"]',
    );
    const doneContainer = content.querySelector(
      '.flex.gap-1\\.5 > div:last-child',
    );
    const doneHint = content.querySelector('#done-hint');

    // Update todo chip
    if (todoContainer) {
      const outer = todoContainer.querySelector('div:first-child');
      const labelBtn = todoContainer.querySelector('button[data-status]');
      const chevronBtn = todoContainer.querySelector('[data-dropdown="todo"]');
      const active = newChip === 'todo';
      const c = CHIP_COLOR.todo;
      outer.className = `flex rounded-md border overflow-hidden ${active ? c.active : c.inactive}`;
      labelBtn.className = `flex-1 py-1.5 px-2 text-xs font-medium ${active ? c.active : c.inactive} focus:outline-none focus:ring-2 focus:ring-inset focus:ring-teal`;
      labelBtn.textContent =
        CHIP_LABEL[newStatus] === 'Next' ? 'Next' : 'To do';
      labelBtn.dataset.status = newStatus === 'NEXT' ? 'NEXT' : 'TODO';
      chevronBtn.className = `py-1.5 px-1 text-xs border-l ${active ? 'border-teal-tint text-teal-strong' : 'border-line text-muted'} hover:bg-mist focus:outline-none focus:ring-2 focus:ring-inset focus:ring-teal`;
    }

    // Update waiting chip
    if (waitingBtn) {
      const active = newChip === 'waiting';
      const c = CHIP_COLOR.waiting;
      waitingBtn.className = `flex-1 rounded-md border py-1.5 text-xs font-medium focus:outline-none focus:ring-2 focus:ring-teal ${active ? c.active : `${c.inactive} hover:bg-mist`}`;
    }

    // Update done chip
    if (doneContainer) {
      const outer = doneContainer.querySelector('div:first-child');
      const labelBtn = doneContainer.querySelector('button[data-status]');
      const chevronBtn = doneContainer.querySelector('[data-dropdown="done"]');
      const active = newChip === 'done';
      const c = CHIP_COLOR.done;
      outer.className = `flex rounded-md border overflow-hidden ${active ? c.active : c.inactive}`;
      labelBtn.className = `flex-1 py-1.5 px-2 text-xs font-medium ${active ? c.active : c.inactive} focus:outline-none focus:ring-2 focus:ring-inset focus:ring-teal`;
      labelBtn.textContent =
        newStatus === 'CANCELED'
          ? 'Canceled'
          : newStatus === 'SOMEDAY'
            ? 'Someday'
            : 'Done';
      labelBtn.dataset.status =
        newStatus === 'CANCELED' || newStatus === 'SOMEDAY'
          ? newStatus
          : 'DONE';
      chevronBtn.className = `py-1.5 px-1 text-xs border-l ${active ? 'border-moss-tint text-moss' : 'border-line text-muted'} hover:bg-mist focus:outline-none focus:ring-2 focus:ring-inset focus:ring-teal`;
    }

    if (doneHint) doneHint.classList.toggle('hidden', !newIsDone);

    try {
      const resp = await fetch(`/api/notes/${noteData.id}`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ status: newStatus }),
      });
      if (!resp.ok) throw new Error(`Failed to update: ${resp.status}`);
      const updated = await resp.json();
      noteData.status = updated.status ? updated.status.toUpperCase() : null;
    } catch (_err) {
      content.innerHTML =
        '<div class="text-center text-rust p-4 text-sm">Failed to update status. Please close and reopen the note.</div>';
    }
  }
}

customElements.define('hq-note-modal', HqNoteModal);
