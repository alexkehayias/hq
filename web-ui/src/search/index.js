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

// Auto hide results from journal entries
const DEFAULT_PARAMS = '-title:journal';
let includeSimilarity = false;

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
  emptyState.setAttribute(
    'title',
    noQuery ? 'Search your notes' : 'No results found. Please try again.',
  );
  searchView.setAttribute('state', 'empty');
}

// Selecting a result highlights it, shares the note via URL, and opens the
// note modal.
resultList.addEventListener('result-select', (e) => {
  const result = e.detail.result;

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

retry.addEventListener('click', () => search(searchInput.value));
