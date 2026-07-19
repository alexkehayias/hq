/**
 * ApprovalCard — renders an inline permission dialog in the chat
 * transcript when a tool call requires user approval.
 *
 * The backend emits an `approval_request` chunk over the SSE stream
 * (see src/ai/chat/middleware.rs:ApprovalMiddleware). The chat
 * reader in index.js parses these chunks and creates an
 * ApprovalCard element with the request_id, tool name(s), and
 * arguments. When the user clicks Approve or Deny, we POST to
 * /api/approval/{session_id}; the server resolves a pending
 * oneshot, unblocking the chat task.
 *
 * States:
 *   - pending: shows tool name + args + Approve/Deny buttons
 *   - approved/denied: status banner, buttons disabled
 *
 * Double-click protection: after a click, both buttons disable
 * immediately; the response just updates the status banner.
 */
class ApprovalCard extends HTMLElement {
  constructor() {
    super();

    this.requestId = '';
    this.sessionId = '';
    /** @type {{id: string, name: string, arguments: string}[]} */
    this.calls = [];
    /** @type {'pending'|'approved'|'denied'} */
    this.state = 'pending';
  }

  static get observedAttributes() {
    return ['request-id', 'session-id'];
  }

  attributeChangedCallback(name, _oldValue, newValue) {
    if (name === 'request-id') this.requestId = newValue;
    if (name === 'session-id') this.sessionId = newValue;
  }

  /**
   * Populate the calls array from a raw approval_request chunk.
   * Called by index.js after creating the element because attribute
   * passing of complex objects doesn't work — strings only.
   */
  setCalls(calls) {
    this.calls = calls;
    this.render();
  }

  connectedCallback() {
    this.render();
  }

  render() {
    if (this.state === 'pending') {
      this.renderPending();
    } else {
      this.renderResolved();
    }
  }

  renderPending() {
    // Show each tool call as its own labeled block — multiple calls
    // in a single request are rare but possible (e.g., two bash
    // calls in one assistant message).
    const callRows = this.calls
      .map((c, i) => {
        // Pretty-print arguments if they're JSON; otherwise show raw.
        let prettyArgs = c.arguments;
        try {
          const parsed = JSON.parse(c.arguments);
          prettyArgs = JSON.stringify(parsed, null, 2);
        } catch {
          // Not JSON — leave as-is
        }
        return `
          <div class="border-t border-gray-200 dark:border-gray-600 pt-2 ${i > 0 ? 'mt-3' : ''}">
            <div class="text-xs font-semibold text-gray-500 dark:text-gray-400 mb-1">
              Call ${i + 1}: <code class="text-blue-600 dark:text-blue-400">${this.escapeHtml(c.name)}</code>
            </div>
            <pre class="text-xs bg-gray-50 dark:bg-gray-900 text-gray-800 dark:text-gray-100 rounded p-2 overflow-x-auto whitespace-pre-wrap break-all">${this.escapeHtml(prettyArgs)}</pre>
          </div>`;
      })
      .join('');

    this.innerHTML = `
      <div class="approval-card flex flex-col gap-2 my-4 p-3 border border-yellow-400 dark:border-yellow-600 bg-yellow-50 dark:bg-gray-800 rounded-xl">
        <div class="flex items-start gap-2">
          <span class="text-lg" role="img" aria-label="shield">🛡️</span>
          <div class="flex-1 min-w-0">
            <div class="font-semibold text-gray-900 dark:text-gray-100 text-sm">
              Permission requested
            </div>
            <div class="text-xs text-gray-600 dark:text-gray-400 mb-2">
              ${
                this.calls.length === 1
                  ? `Claude wants to run <code class="text-blue-600 dark:text-blue-400">${this.escapeHtml(this.calls[0].name)}</code>`
                  : `Claude wants to run ${this.calls.length} tools`
              }
            </div>
            <div class="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-600 p-2 max-h-48 overflow-y-auto">
              ${callRows}
            </div>
          </div>
        </div>
        <div class="flex gap-2 justify-end mt-1">
          <button
            type="button"
            data-action="deny"
            class="px-3 py-1.5 text-sm font-medium rounded-lg border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-200 hover:bg-gray-100 dark:hover:bg-gray-700 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Deny
          </button>
          <button
            type="button"
            data-action="approve"
            class="px-3 py-1.5 text-sm font-medium rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Approve
          </button>
        </div>
      </div>`;

    // Wire up button handlers
    this.querySelector('[data-action="approve"]')?.addEventListener(
      'click',
      () => {
        this.submit(true);
      },
    );
    this.querySelector('[data-action="deny"]')?.addEventListener(
      'click',
      () => {
        const deny = true;
        this.submit(false, deny ? 'User denied via UI' : undefined);
      },
    );
  }

  renderResolved() {
    const isApproved = this.state === 'approved';
    const bannerClass = isApproved
      ? 'bg-green-100 dark:bg-green-900 border-green-400 dark:border-green-600 text-green-800 dark:text-green-200'
      : 'bg-red-100 dark:bg-red-900 border-red-400 dark:border-red-600 text-red-800 dark:text-red-200';
    const icon = isApproved ? '✓' : '✗';
    const status = isApproved
      ? 'Tool call approved'
      : `Tool call denied${this.denyReason ? `: ${this.denyReason}` : ''}`;

    this.innerHTML = `
      <div class="approval-card flex items-center gap-2 my-4 p-3 border rounded-xl ${bannerClass}">
        <span class="text-lg" role="img">${icon}</span>
        <div class="flex-1 min-w-0">
          <div class="font-semibold text-sm">${this.escapeHtml(status)}</div>
          <div class="text-xs opacity-80">
            ${this.calls.map((c) => this.escapeHtml(c.name)).join(', ')}
          </div>
        </div>
      </div>`;
  }

  /**
   * Submit the approval decision to /api/approval/{session_id}.
   * Disables buttons immediately for double-click protection; updates
   * the banner based on the response.
   */
  async submit(approved, denyReason) {
    // Disable buttons immediately to prevent double-submit
    this.querySelectorAll('button').forEach((btn) => {
      btn.disabled = true;
    });

    const body = { request_id: this.requestId, approved };
    if (!approved && denyReason) {
      body.message = denyReason;
    }

    try {
      const resp = await fetch(
        `/api/approval/${encodeURIComponent(this.sessionId)}`,
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        },
      );
      // 200 either way (resolved=true/false). For our purposes, we
      // don't distinguish — the chat task is what matters, and it
      // will continue or reject based on its registry lookup.
      if (!resp.ok) {
        throw new Error(`HTTP ${resp.status}`);
      }
      this.state = approved ? 'approved' : 'denied';
      if (!approved && denyReason) {
        this.denyReason = denyReason;
      }
    } catch (err) {
      console.error('Approval request failed:', err);
      // On error, re-enable so the user can retry. The chat task
      // is still waiting; without a response, it will eventually
      // time out (default 5 min) and deny automatically.
      this.querySelectorAll('button').forEach((btn) => {
        btn.disabled = false;
      });
      // Re-render to show pending state again
      this.render();
      return;
    }
    this.render();
  }

  escapeHtml(str) {
    const div = document.createElement('div');
    div.textContent = str;
    return div.innerHTML;
  }
}

customElements.define('approval-card', ApprovalCard);

export default ApprovalCard;
