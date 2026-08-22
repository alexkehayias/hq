import { html } from '/components/lib/html.js';
import './session-item.js';

const view = document.getElementById('sessions-view');
const list = document.getElementById('sessions-list');
const pagination = document.getElementById('sessions-pagination');
const limit = 20;

pagination.addEventListener('page-change', (e) => {
  view.setAttribute('state', 'loading');
  loadSessions(e.detail.page);
});

async function loadSessions(page = 1) {
  try {
    const response = await fetch(
      `/api/chat/sessions?exclude_tags=background&page=${page}&limit=${limit}`,
      {
        method: 'GET',
        headers: {
          'Content-Type': 'application/json',
        },
      },
    );

    if (!response.ok) {
      throw new Error(`HTTP error! status: ${response.status}`);
    }

    const data = await response.json();

    if (data.sessions.length === 0) {
      view.setAttribute('state', 'empty');
    } else {
      list.innerHTML = data.sessions
        .map(
          (session) =>
            html`<hq-session-item session=${JSON.stringify(session)}></hq-session-item>`,
        )
        .map((r) => r.value)
        .join('');

      pagination.setAttribute('page', String(data.page));
      pagination.setAttribute('total-pages', String(data.total_pages));
      view.setAttribute('state', 'content');
    }

    updateURL(page);
  } catch (error) {
    console.error('Error loading sessions:', error);
    view.setAttribute('state', 'error');
  }
}

function updateURL(page) {
  const url = new URL(window.location);
  url.searchParams.set('page', page);
  window.history.replaceState({}, '', url);
}

// Load the sessions for the current page when DOM is ready
const urlParams = new URLSearchParams(window.location.search);
const currentPage = parseInt(urlParams.get('page'), 10) || 1;
loadSessions(currentPage);
