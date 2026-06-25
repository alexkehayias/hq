(async () => {
  const searchInput = document.getElementById('search');
  const resultList = document.getElementById('results');
  const emptyState = document.getElementById('empty-state');

  const handleSearch = async (includeSimilarity, val) => {
    try {
      const queryEncoded = encodeURIComponent(val);
      // Auto hide results from journal entries
      const defaultParams = encodeURIComponent('-title:journal');

      // Update the URL params so the link to the results can be
      // shared nicely
      const url = new URL(window.location);
      const params = url.searchParams;
      params.set('query', queryEncoded);
      window.history.replaceState(null, '', url);

      const headers = new Headers();
      headers.append('Content-Type', 'application/json');

      // Join the original query and default search params with a
      // space
      const queryWithDefaults = `${queryEncoded}%20${defaultParams}`;

      const response = await fetch(
        `/api/notes/search?query=${queryWithDefaults}&include_similarity=${includeSimilarity}`,
        {
          method: 'GET',
          headers,
        },
      );
      if (!response.ok) {
        throw new Error(`Error fetching: ${response.status}`);
      }

      const data = await response.json();

      if (data.results.length > 0) {
        emptyState.classList.add('hidden');
        resultList.classList.remove('hidden');

        const hits = data.results.map((r) => {
          // Create a list item for each hit
          const hit = document.createElement('li');
          hit.dataset.noteId = r.id;
          hit.classList.add(
            ...[
              'group',
              'flex',
              'justify-between',
              'cursor-default',
              'select-none',
              'items-center',
              'rounded-md',
              'px-3',
              'py-2',
              'hover:cursor-pointer',
            ],
          );

          const titleContainer = document.createElement('div');
          titleContainer.classList.add(...['flex', 'space-x-2']);

          // If this is a task, show a todo icon
          if (r.is_task) {
            const taskIconContainer = document.createElement('span');
            taskIconContainer.classList.add(
              ...['py-0.5', 'text-gray-800', 'text-xs', 'rounded-full'],
            );
            // Map the status to an icon
            switch (r.task_status.toLowerCase()) {
              case 'todo':
                taskIconContainer.innerText = '⬜';
                break;
              case 'next':
                taskIconContainer.innerText = '⏭️';
                break;
              case 'waiting':
                taskIconContainer.innerText = '⏳';
                break;
              case 'canceled':
                taskIconContainer.innerText = '❌';
                break;
              case 'done':
                taskIconContainer.innerText = '✅';
                break;
              case 'someday':
                taskIconContainer.innerText = '🤷';
                break;
              default:
                break;
            }
            titleContainer.appendChild(taskIconContainer);
          }

          // If this is a chat result, show a chat icon
          if (r.type === 'chat') {
            const chatIconContainer = document.createElement('span');
            chatIconContainer.classList.add(
              ...['py-0.5', 'text-gray-800', 'text-xs', 'rounded-full'],
            );
            chatIconContainer.innerText = '💬';
            titleContainer.appendChild(chatIconContainer);
          }

          // Add in the title
          const titleTextContainer = document.createElement('span');
          titleTextContainer.classList.add(...['line-clamp-1']);
          titleTextContainer.innerText = r.title;
          titleContainer.appendChild(titleTextContainer);

          hit.appendChild(titleContainer);

          // Add in each tag
          // Tags are a comma separated string so we need to check if
          // there is an empty string to determine if there are any tags
          // to render
          if (r.tags) {
            const tagContainer = document.createElement('div');
            tagContainer.classList.add(...['flex', 'flex-row']);
            r.tags.split(',').forEach((tag) => {
              const tagDiv = document.createElement('div');
              tagDiv.classList.add(
                ...[
                  'bg-gray-200',
                  'text-gray-700',
                  'text-xs',
                  'px-2',
                  'py-0.5',
                  'rounded-full',
                  'mr-2',
                ],
              );
              tagDiv.innerText = `#${tag}`;
              tagContainer.appendChild(tagDiv);
            });
            hit.appendChild(tagContainer);
          }

          hit.addEventListener('click', async (_clickEvent) => {
            console.log(`Clicked result with id ${r.id}`);

            // Unselect all other hits
            hits.forEach((hit) => {
              hit.classList.remove(...['bg-blue-700', 'text-white']);
            });

            // Highlight the selected hit
            hit.classList.add(...['bg-blue-700', 'text-white']);

            // Update URL with the selected note ID
            const url = new URL(window.location);
            url.searchParams.set('note_id', r.id);
            window.history.replaceState(null, '', url);

            // Store the selected hit in the search session
            const resp = await fetch(`/api/notes/search/latest`, {
              method: 'POST',
              body: JSON.stringify({
                id: r.id,
                file_name: r.file_name,
                title: r.title,
              }),
              headers: {
                Accept: 'application/json',
                'Content-Type': 'application/json',
              },
            });
            if (!resp.ok) {
              throw new Error(`Error updating latest hit: ${response.status}`);
            } else {
              console.log(`Updated latest hit to ${r.id}`);
            }

            // Show note in fullscreen modal
            // Create or reuse overlay modal
            let modal = document.getElementById('note-modal');
            let addedModal = false;
            if (!modal) {
              modal = document.createElement('div');
              modal.id = 'note-modal';
              modal.className =
                'fixed inset-0 flex items-start justify-center bg-black bg-opacity-85 z-[10000] p-4 sm:p-8';
              modal.innerHTML = `<div id="note-modal-content" class="w-full max-w-lg sm:max-w-xl rounded-xl bg-white dark:bg-gray-800 p-5 shadow-xl ring-1 ring-black/5 overflow-auto max-h-[90vh]"></div>`;
              document.body.appendChild(modal);
              addedModal = true;
            }
            const content = modal.querySelector('#note-modal-content');

            // Show loading
            content.innerHTML =
              '<div class="mb-4 text-center text-xl">Loading...</div>';
            modal.style.display = 'flex';

            let dismissModal = () => {
              modal.style.display = 'none';
              document.removeEventListener('keydown', escListener);
              if (addedModal) {
                modal.remove();
              }
              // Remove note_id from URL when modal is dismissed
              const url = new URL(window.location);
              url.searchParams.delete('note_id');
              window.history.replaceState(null, '', url);
            };

            // Click outside the modal content to close
            modal.onclick = (e) => {
              if (e.target === modal) {
                dismissModal();
              }
            };

            // ESC key closes modal
            function escListener(e) {
              if (e.key === 'Escape') {
                dismissModal();
              }
            }
            document.addEventListener('keydown', escListener);

            // Fetch and render the note JSON
            fetch(`/api/notes/${r.id}/view`, {
              headers: { Accept: 'application/json' },
            })
              .then(async (resp) => {
                if (!resp.ok) throw new Error('Failed to fetch note');
                return resp.json();
              })
              .then((noteData) => {
                let html = '';
                const rawStatus = noteData.status;
                const isTask = noteData.type === 'task' && rawStatus;
                const status = isTask ? rawStatus.toUpperCase() : null;
                const chipLabel = {
                  TODO: 'To do',
                  NEXT: 'Next',
                  WAITING: 'Waiting',
                  DONE: 'Done',
                  CANCELED: 'Canceled',
                  SOMEDAY: 'Someday',
                };
                const chipGroup = {
                  TODO: 'todo',
                  NEXT: 'todo',
                  WAITING: 'waiting',
                  DONE: 'done',
                  CANCELED: 'done',
                  SOMEDAY: 'done',
                };
                const currentChip = status ? chipGroup[status] || 'todo' : null;
                const isDone = currentChip === 'done';

                // Header: task badge + close button
                html += '<div class="flex items-start justify-between mb-3.5">';
                if (isTask) {
                  html +=
                    '<span class="inline-flex items-center gap-1.5 rounded-md bg-blue-50 dark:bg-blue-900/30 px-2.5 py-1 text-xs font-medium text-blue-700 dark:text-blue-300">';
                  html +=
                    '<svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">';
                  html +=
                    '<path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2h11"/></svg> Task</span>';
                } else {
                  html += '<span></span>';
                }
                html +=
                  '<button id="modal-close-btn" type="button" aria-label="Close" class="flex h-8 w-8 items-center justify-center rounded-md text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-600 dark:hover:text-gray-300 focus:outline-none focus:ring-2 focus:ring-gray-300">';
                html +=
                  '<svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M6 6l12 12M18 6L6 18"/></svg></button>';
                html += '</div>';

                // Title
                html += `<h2 class="text-lg font-medium leading-snug text-gray-900 dark:text-gray-100">${noteData.title || ''}</h2>`;

                // Status chips with dropdowns (tasks only)
                if (isTask) {
                  const chipColor = {
                    todo: {
                      active:
                        'border-blue-200 bg-blue-50 text-blue-700 dark:border-blue-800 dark:bg-blue-900/30 dark:text-blue-300',
                      inactive:
                        'border-gray-200 bg-white text-gray-500 dark:border-gray-600 dark:bg-transparent dark:text-gray-400',
                    },
                    waiting: {
                      active:
                        'border-yellow-200 bg-yellow-50 text-yellow-700 dark:border-yellow-800 dark:bg-yellow-900/30 dark:text-yellow-300',
                      inactive:
                        'border-gray-200 bg-white text-gray-500 dark:border-gray-600 dark:bg-transparent dark:text-gray-400',
                    },
                    done: {
                      active:
                        'border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-800 dark:bg-emerald-900/30 dark:text-emerald-300',
                      inactive:
                        'border-gray-200 bg-white text-gray-500 dark:border-gray-600 dark:bg-transparent dark:text-gray-400',
                    },
                  };
                  const chips = [
                    {
                      key: 'todo',
                      label: chipLabel[status] === 'Next' ? 'Next' : 'To do',
                      dropdown: [{ status: 'NEXT', label: 'Next' }],
                    },
                    { key: 'waiting', label: 'Waiting', dropdown: [] },
                    {
                      key: 'done',
                      label:
                        chipLabel[status] === 'Canceled'
                          ? 'Canceled'
                          : chipLabel[status] === 'Someday'
                            ? 'Someday'
                            : 'Done',
                      dropdown: [
                        { status: 'CANCELED', label: 'Canceled' },
                        { status: 'SOMEDAY', label: 'Someday' },
                      ],
                    },
                  ];
                  html +=
                    '<p class="mb-1.5 mt-4 text-xs text-gray-500">Status</p>';
                  html += '<div class="flex gap-1.5">';
                  chips.forEach((chip) => {
                    const active = currentChip === chip.key;
                    const c = chipColor[chip.key];
                    const chipClasses = `flex-1 rounded-md border py-1.5 text-xs font-medium focus:outline-none focus:ring-2 focus:ring-blue-500/40 ${active ? c.active : `${c.inactive} hover:bg-gray-50 dark:hover:bg-gray-800`}`;
                    if (chip.dropdown.length > 0) {
                      html += `<div class="relative flex-1">
                        <div class="flex rounded-md border overflow-hidden ${active ? c.active : c.inactive}">
                          <button type="button" data-status="${chip.key === 'todo' ? 'TODO' : 'DONE'}" class="flex-1 py-1.5 px-2 text-xs font-medium ${active ? c.active : c.inactive} focus:outline-none focus:ring-2 focus:ring-inset focus:ring-blue-500/40">${chip.label}</button>
                          <button type="button" data-dropdown="${chip.key}" class="py-1.5 px-1 text-xs border-l ${active ? 'border-blue-200 dark:border-blue-800 text-blue-700 dark:text-blue-300' : 'border-gray-200 dark:border-gray-600 text-gray-400 dark:text-gray-500'} hover:bg-gray-50 dark:hover:bg-gray-800 focus:outline-none focus:ring-2 focus:ring-inset focus:ring-blue-500/40">
                            <svg class="w-3 h-3" viewBox="0 0 20 20" fill="currentColor"><path d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z"/></svg>
                          </button>
                        </div>
                        <div id="dropdown-${chip.key}" class="hidden absolute z-20 mt-1 w-full rounded-md bg-white dark:bg-gray-700 shadow-lg ring-1 ring-black ring-opacity-5 overflow-hidden">${chip.dropdown.map((item) => `<button type="button" data-status="${item.status}" class="flex w-full items-center gap-2 px-3 py-2 text-xs text-gray-700 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-600">${item.label}</button>`).join('')}</div>
                      </div>`;
                    } else {
                      html += `<button type="button" data-status="WAITING" class="${chipClasses}">${chip.label}</button>`;
                    }
                  });
                  html += '</div>';
                }

                // Info chips
                if (noteData.file_name) {
                  html += '<div class="mt-4 flex gap-2.5">';
                  html += `<div class="flex-1 rounded-md bg-gray-50 dark:bg-gray-700/50 px-2.5 py-2">
                    <p class="text-xs text-gray-400">Source</p>
                    <p class="text-sm font-medium text-gray-900 dark:text-gray-100">${noteData.file_name}</p>
                  </div>`;
                  html += '</div>';
                }

                // Tags
                if (noteData.tags) {
                  html += `<div class="mt-3">${noteData.tags
                    .split(',')
                    .map(
                      (t) =>
                        `<span class="inline-block mr-1.5 mb-1 bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-400 text-xs px-2 py-0.5 rounded-full">#${t}</span>`,
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
                const messageHtml = marked.parse(bodyWithoutTitle, {
                  breaks: true,
                });
                html += '<p class="mb-1.5 mt-4 text-xs text-gray-500">Note</p>';
                html += `<div class="rounded-md bg-gray-50 dark:bg-gray-700/50 px-3 py-2.5 text-sm text-gray-600 dark:text-gray-300 markdown">${bodyWithoutTitle ? messageHtml : '<span class="italic text-gray-400 dark:text-gray-500">No additional content</span>'}</div>`;

                // Done confirmation strip
                if (isTask) {
                  html += `<div id="done-hint" class="mt-4 flex items-center gap-1.5 rounded-md bg-emerald-50 dark:bg-emerald-900/30 px-3 py-2 text-xs text-emerald-700 dark:text-emerald-300 ${isDone ? '' : 'hidden'}">
                    <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 13l4 4L19 7"/></svg>
                    Marked done
                  </div>`;
                }

                // Actions
                html +=
                  '<div class="mt-4 flex gap-2 border-t border-gray-100 dark:border-gray-700 pt-4">';
                html +=
                  '<button type="button" class="flex flex-1 items-center justify-center gap-1.5 rounded-md border border-gray-200 dark:border-gray-600 py-1.5 text-xs font-medium text-gray-700 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-700 focus:outline-none focus:ring-2 focus:ring-gray-300">';
                html +=
                  '<svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 20h9M16.5 3.5a2.1 2.1 0 013 3L7 19l-4 1 1-4z"/></svg> Edit</button>';
                html +=
                  '<button type="button" class="flex flex-1 items-center justify-center gap-1.5 rounded-md border border-gray-200 dark:border-gray-600 py-1.5 text-xs font-medium text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/30 focus:outline-none focus:ring-2 focus:ring-red-400/40">';
                html +=
                  '<svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m2 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6"/></svg> Delete</button>';
                html += '</div>';

                content.innerHTML = html;
                document.getElementById('modal-close-btn').onclick =
                  dismissModal;

                // Wire up status chips and dropdowns
                if (isTask) {
                  const doneHint = document.getElementById('done-hint');
                  const chipColor = {
                    todo: {
                      active:
                        'border-blue-200 bg-blue-50 text-blue-700 dark:border-blue-800 dark:bg-blue-900/30 dark:text-blue-300',
                      inactive:
                        'border-gray-200 bg-white text-gray-500 dark:border-gray-600 dark:bg-transparent dark:text-gray-400',
                    },
                    waiting: {
                      active:
                        'border-yellow-200 bg-yellow-50 text-yellow-700 dark:border-yellow-800 dark:bg-yellow-900/30 dark:text-yellow-300',
                      inactive:
                        'border-gray-200 bg-white text-gray-500 dark:border-gray-600 dark:bg-transparent dark:text-gray-400',
                    },
                    done: {
                      active:
                        'border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-800 dark:bg-emerald-900/30 dark:text-emerald-300',
                      inactive:
                        'border-gray-200 bg-white text-gray-500 dark:border-gray-600 dark:bg-transparent dark:text-gray-400',
                    },
                  };

                  // Close any open dropdown on outside click
                  function closeDropdowns(e) {
                    document
                      .querySelectorAll('[id^="dropdown-"]')
                      .forEach((dd) => {
                        if (
                          !dd.classList.contains('hidden') &&
                          !dd.parentElement.contains(e.target)
                        ) {
                          dd.classList.add('hidden');
                        }
                      });
                  }
                  document.addEventListener('click', closeDropdowns);
                  const origDismiss = dismissModal;
                  dismissModal = () => {
                    document.removeEventListener('click', closeDropdowns);
                    origDismiss();
                  };

                  // Click on chip label → set status
                  content.querySelectorAll('[data-status]').forEach((btn) => {
                    btn.addEventListener('click', async () => {
                      const newStatus = btn.dataset.status;
                      await updateStatus(newStatus);
                    });
                  });

                  // Click on chevron → toggle dropdown
                  content.querySelectorAll('[data-dropdown]').forEach((btn) => {
                    btn.addEventListener('click', (e) => {
                      e.stopPropagation();
                      const key = btn.dataset.dropdown;
                      const dd = document.getElementById(`dropdown-${key}`);
                      // Close other dropdowns
                      document
                        .querySelectorAll('[id^="dropdown-"]')
                        .forEach((d) => {
                          if (d.id !== `dropdown-${key}`)
                            d.classList.add('hidden');
                        });
                      dd.classList.toggle('hidden');
                    });
                  });

                  // Click on dropdown item → set status
                  content
                    .querySelectorAll('[id^="dropdown-"] button[data-status]')
                    .forEach((btn) => {
                      btn.addEventListener('click', async () => {
                        const newStatus = btn.dataset.status;
                        // Close dropdown
                        btn
                          .closest('[id^="dropdown-"]')
                          .classList.add('hidden');
                        await updateStatus(newStatus);
                      });
                    });

                  async function updateStatus(newStatus) {
                    const newChip = chipGroup[newStatus] || 'todo';
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

                    // Update todo chip
                    if (todoContainer) {
                      const outer =
                        todoContainer.querySelector('div:first-child');
                      const labelBtn = todoContainer.querySelector(
                        'button[data-status]',
                      );
                      const chevronBtn = todoContainer.querySelector(
                        '[data-dropdown="todo"]',
                      );
                      const active = newChip === 'todo';
                      const c = chipColor.todo;
                      outer.className = `flex rounded-md border overflow-hidden ${active ? c.active : c.inactive}`;
                      labelBtn.className = `flex-1 py-1.5 px-2 text-xs font-medium ${active ? c.active : c.inactive} focus:outline-none focus:ring-2 focus:ring-inset focus:ring-blue-500/40`;
                      labelBtn.textContent =
                        chipLabel[newStatus] === 'Next' ? 'Next' : 'To do';
                      labelBtn.dataset.status =
                        newStatus === 'NEXT' ? 'NEXT' : 'TODO';
                      chevronBtn.className = `py-1.5 px-1 text-xs border-l ${active ? 'border-blue-200 dark:border-blue-800 text-blue-700 dark:text-blue-300' : 'border-gray-200 dark:border-gray-600 text-gray-400 dark:text-gray-500'} hover:bg-gray-50 dark:hover:bg-gray-800 focus:outline-none focus:ring-2 focus:ring-inset focus:ring-blue-500/40`;
                    }

                    // Update waiting chip
                    if (waitingBtn) {
                      const active = newChip === 'waiting';
                      const c = chipColor.waiting;
                      waitingBtn.className = `flex-1 rounded-md border py-1.5 text-xs font-medium focus:outline-none focus:ring-2 focus:ring-blue-500/40 ${active ? c.active : `${c.inactive} hover:bg-gray-50 dark:hover:bg-gray-800`}`;
                    }

                    // Update done chip
                    if (doneContainer) {
                      const outer =
                        doneContainer.querySelector('div:first-child');
                      const labelBtn = doneContainer.querySelector(
                        'button[data-status]',
                      );
                      const chevronBtn = doneContainer.querySelector(
                        '[data-dropdown="done"]',
                      );
                      const active = newChip === 'done';
                      const c = chipColor.done;
                      outer.className = `flex rounded-md border overflow-hidden ${active ? c.active : c.inactive}`;
                      labelBtn.className = `flex-1 py-1.5 px-2 text-xs font-medium ${active ? c.active : c.inactive} focus:outline-none focus:ring-2 focus:ring-inset focus:ring-blue-500/40`;
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
                      chevronBtn.className = `py-1.5 px-1 text-xs border-l ${active ? 'border-emerald-200 dark:border-emerald-800 text-emerald-700 dark:text-emerald-300' : 'border-gray-200 dark:border-gray-600 text-gray-400 dark:text-gray-500'} hover:bg-gray-50 dark:hover:bg-gray-800 focus:outline-none focus:ring-2 focus:ring-inset focus:ring-blue-500/40`;
                    }

                    if (doneHint) {
                      doneHint.classList.toggle('hidden', !newIsDone);
                    }

                    try {
                      const resp = await fetch(`/api/notes/${noteData.id}`, {
                        method: 'PATCH',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ status: newStatus }),
                      });
                      if (!resp.ok)
                        throw new Error(`Failed to update: ${resp.status}`);
                      const updated = await resp.json();
                      noteData.status = updated.status
                        ? updated.status.toUpperCase()
                        : null;
                    } catch (_err) {
                      // Revert
                      const prevChip = chipGroup[noteData.status] || 'todo';
                      const _prevIsDone = prevChip === 'done';
                      // Re-render chips to previous state by re-setting up
                      // For simplicity, just revert the visual using the stored noteData.status
                      // Re-run the same visual update with the previous status
                      // Since we already modified the DOM, we need to undo
                      // Reload the modal content instead
                      // Actually, let's just re-fetch and re-render
                      content.innerHTML =
                        '<div class="mb-4 text-center text-xl">Error — reloading...</div>';
                      fetch(`/api/notes/${noteData.id}/view`, {
                        headers: { Accept: 'application/json' },
                      })
                        .then((r) => r.json())
                        .then((_data) => {
                          // Re-run the whole .then logic... this is complex.
                          // For now, just show a simple error state
                          content.innerHTML = `<div class="text-center text-red-700 p-4 text-sm">Failed to update status. Please close and reopen the note.</div>`;
                        });
                    }
                  }
                }
              })
              .catch((err) => {
                content.innerHTML = `<div class="text-center text-red-700 p-8">Failed to load note: ${err.message}</div>`;
              });
            return;
          });
          return hit;
        });
        resultList.replaceChildren(...hits);
      } else {
        resultList.classList.add('hidden');
        emptyState.classList.remove('hidden');
      }
    } catch (error) {
      console.error('Server error', error.message);
    }
  };

  // If there is already a query, initiate the search
  const urlParams = new URLSearchParams(window.location.search);
  const initQuery = urlParams.get('query');
  const includeSimilarity = urlParams.get('include_similarity') === 'true';

  if (initQuery) {
    searchInput.value = decodeURIComponent(initQuery);
    await handleSearch(includeSimilarity, searchInput.value);

    // If there's a note_id param, open the corresponding note modal
    const noteId = urlParams.get('note_id');
    if (noteId) {
      const targetHit = resultList.querySelector(`[data-note-id="${noteId}"]`);
      if (targetHit) {
        targetHit.click();
      }
    }
  }

  // Handle search as you type
  searchInput.addEventListener('input', async (e) => {
    const val = e.target.value;

    if (val) {
      await handleSearch(includeSimilarity, val);
    }
  });

  // Register the service worker
  if ('serviceWorker' in navigator) {
    window.addEventListener('load', () => {
      navigator.serviceWorker
        .register('/service-worker.js')
        .then((registration) => {
          console.log('SW registered: ', registration);
        })
        .catch((registrationError) => {
          console.log('SW registration failed: ', registrationError);
        });
    });
  }

  // Function to detect mobile Safari
  const isMobileSafari = () => {
    return /iP(ad|hone|od).+Version\/[\d.]+.*Safari/i.test(navigator.userAgent);
  };

  const subscribeToPushNotifications = async () => {
    try {
      const permission = await Notification.requestPermission();
      if (permission !== 'granted') {
        console.log('Notification permission not granted');
        return;
      }

      // Subscribe to the Push service
      const registration = await navigator.serviceWorker.ready;
      const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey:
          'BNKK9yweDqrtqTqUdHIhtne8YpfymNIsADbQt2ctFirKrgy1kaWu5mrPUG2F1GQAooQyVzqEa_4BnDIWzz7XRBc',
      });

      // Send subscription to server
      await fetch('/api/push/subscribe', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(subscription),
      });
    } catch (error) {
      console.error('Failed to subscribe the user: ', error);
    }
  };

  // Show push notification permission button if on mobile Safari
  if (isMobileSafari() && 'Notification' in window && navigator.serviceWorker) {
    const permissionButton = document.createElement('button');
    permissionButton.innerText = 'Enable Notifications';
    permissionButton.classList.add(
      ...[
        'fixed',
        'z-10',
        'bottom-10',
        'right-10',
        'rounded-md',
        'bg-white',
        'px-2.5',
        'py-1.5',
        'text-sm',
        'font-semibold',
        'text-gray-900',
        'shadow-sm',
        'ring-1',
        'ring-inset',
        'ring-gray-300',
        'hover:bg-gray-50',
        'hover:cursor-pointer',
      ],
    );

    document.body.appendChild(permissionButton);

    permissionButton.addEventListener('click', async () => {
      try {
        await subscribeToPushNotifications();
        permissionButton.style.display = 'none';
      } catch (error) {
        console.error('Failed to subscribe the user: ', error);
      }
    });
  } else {
    await subscribeToPushNotifications();
  }
})();
