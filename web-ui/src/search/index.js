// Search page: full-text + vector search over notes, tasks, and chat messages.
import { html } from '/components/lib/html.js';
import './search-result.js';
import './note-modal.js';

const searchInput = document.getElementById('search');
const resultList = document.getElementById('results');
const searchView = document.getElementById('search-view');
const emptyState = document.getElementById('empty-state');
const noteModal = document.getElementById('note-modal');
const retry = document.getElementById('retry');
const recentSearches = document.getElementById('recent-searches');
const recentList = recentSearches.querySelector('ul');

// Auto hide results from journal entries
const DEFAULT_PARAMS = '-title:journal';
let includeSimilarity = false;

// Recent searches persisted per-device (like the theme override)
const RECENT_KEY = 'hq-recent-searches';
const MAX_RECENT = 10;

function getRecentSearches() {
  try {
    const parsed = JSON.parse(localStorage.getItem(RECENT_KEY));
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function saveRecentSearch(query) {
  const q = query.trim();
  if (!q) return;
  const recents = getRecentSearches().filter((r) => r !== q);
  recents.unshift(q);
  localStorage.setItem(
    RECENT_KEY,
    JSON.stringify(recents.slice(0, MAX_RECENT)),
  );
  renderRecentSearches();
}

function renderRecentSearches() {
  recentList.innerHTML = getRecentSearches()
    .map(
      (q) =>
        html`<li>
          <button type="button" data-query="${q}" class="inline-flex items-center gap-1.5 rounded-full border border-gray-300 dark:border-gray-600 px-3 py-1 text-sm text-gray-700 dark:text-gray-300 transition-colors hover:border-blue-500 hover:text-blue-600 dark:hover:text-blue-400">
            <hq-icon name="search" size="sm"></hq-icon>
            <span>${q}</span>
          </button>
        </li>`,
    )
    .map((r) => r.value)
    .join('');
}

function updateEmptyState(noQuery) {
  // When the box is empty and there are recent searches, they are the empty
  // state: clear the title/icon so the empty state shows its content slot.
  // Otherwise show the prompt.
  const recentsAsEmpty = noQuery && getRecentSearches().length > 0;
  emptyState.setAttribute(
    'title',
    recentsAsEmpty ? '' : noQuery ? 'Search your notes' : 'No results found.',
  );
  emptyState.setAttribute(
    'icon',
    recentsAsEmpty ? '' : noQuery ? 'book' : 'close',
  );
}

async function search(query) {
  if (!query) {
    showEmpty(true);
    return;
  }
  try {
    const queryEncoded = encodeURIComponent(query);

    // Update the URL params so the link to the results can be shared nicely
    const url = new URL(window.location);
    url.searchParams.set('query', queryEncoded);
    window.history.replaceState(null, '', url);

    // Join the original query and default search params with a space
    const queryWithDefaults = `${queryEncoded}%20${encodeURIComponent(DEFAULT_PARAMS)}`;

    const response = await fetch(
      `/api/notes/search?query=${queryWithDefaults}&include_similarity=${includeSimilarity}`,
      { method: 'GET' },
    );
    if (!response.ok) {
      throw new Error(`Error fetching: ${response.status}`);
    }

    const data = await response.json();

    if (data.results.length > 0) {
      resultList.innerHTML = data.results
        .map(
          (r) =>
            html`<hq-search-result result=${JSON.stringify(r)}></hq-search-result>`,
        )
        .map((r) => r.value)
        .join('');
      searchView.setAttribute('state', 'content');
    } else {
      resultList.innerHTML = '';
      showEmpty(false);
    }
  } catch (error) {
    console.error('Server error', error.message);
    searchView.setAttribute('state', 'error');
  }
}

function showEmpty(noQuery) {
  updateEmptyState(noQuery);
  searchView.setAttribute('state', 'empty');
}

// Selecting a result highlights it, shares the note via URL, and opens the
// note modal.
resultList.addEventListener('result-select', (e) => {
  const result = e.detail.result;

  // The query that produced this hit counts as a completed search
  saveRecentSearch(searchInput.value);

  resultList.querySelectorAll('hq-search-result').forEach((el) => {
    el.removeAttribute('selected');
  });
  e.target.setAttribute('selected', '');

  const url = new URL(window.location);
  url.searchParams.set('note_id', result.id);
  window.history.replaceState(null, '', url);

  // Store the selected hit in the search session
  fetch(`/api/notes/search/latest`, {
    method: 'POST',
    body: JSON.stringify({
      id: result.id,
      file_name: result.file_name,
      title: result.title,
    }),
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json',
    },
  }).catch((error) => {
    console.error('Failed to update latest hit:', error.message);
  });

  noteModal.open(result.id);
});

// Remove note_id from URL when the modal is dismissed
noteModal.addEventListener('close', () => {
  const url = new URL(window.location);
  url.searchParams.delete('note_id');
  window.history.replaceState(null, '', url);
});

// If there is already a query, initiate the search
const urlParams = new URLSearchParams(window.location.search);
includeSimilarity = urlParams.get('include_similarity') === 'true';
const initQuery = urlParams.get('query');

if (initQuery) {
  searchInput.value = decodeURIComponent(initQuery);
  searchView.setAttribute('state', 'loading');
  await search(searchInput.value);

  // If there's a note_id param, open the corresponding note modal
  const noteId = urlParams.get('note_id');
  if (noteId) {
    const targetHit = resultList.querySelector(`[data-note-id="${noteId}"]`);
    if (targetHit) {
      targetHit.click();
    }
  }
} else {
  renderRecentSearches();
  showEmpty(true);
}

// Handle search as you type
searchInput.addEventListener('input', (e) => {
  const val = e.target.value;
  if (val) {
    search(val);
  } else {
    resultList.innerHTML = '';
    const url = new URL(window.location);
    url.searchParams.delete('query');
    window.history.replaceState(null, '', url);
    showEmpty(true);
  }
});

// Saving on Enter captures a committed search without recording every
// keystroke of the as-you-type search.
searchInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') saveRecentSearch(searchInput.value);
});

// Clicking a recent search reruns it
recentSearches.addEventListener('click', (e) => {
  const button = e.target.closest('[data-query]');
  if (!button) return;
  const query = button.dataset.query;
  searchInput.value = query;
  saveRecentSearch(query);
  search(query);
});

retry.addEventListener('click', () => search(searchInput.value));
