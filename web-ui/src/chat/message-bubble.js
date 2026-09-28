class MessageBubble extends HTMLElement {
  constructor() {
    super();

    // Initialize state
    this.isToolCall = false;
    this.isLoading = false;
    this.isUserMessage = false;
  }

  static get observedAttributes() {
    return ['message', 'is-user-message', 'is-tool-call', 'is-loading'];
  }

  attributeChangedCallback(name, oldValue, newValue) {
    if (oldValue !== newValue) {
      switch (name) {
        case 'message':
          this.message = newValue;
          break;
        case 'is-user-message':
          this.isUserMessage = newValue === 'true';
          break;
        case 'is-tool-call':
          this.isToolCall = newValue === 'true';
          break;
        case 'is-loading':
          this.isLoading = newValue === 'true';
          break;
      }
      this.render();
    }
  }

  connectedCallback() {
    this.render();
  }

  render() {
    if (this.isLoading) {
      this.innerHTML = `
        <div class="flex justify-start mb-4">
          <div class="flex items-center rounded-bubble bg-mist p-4 md:border md:border-line md:bg-card md:p-6">
            <img src="./img/dog1.png" class="w-8 h-8 animate-bounce-dog1" alt="Loading">
            <img src="./img/dog2.png" class="w-8 h-8 animate-bounce-dog2" alt="Loading">
            <img src="./img/dog3.png" class="w-8 h-8 animate-bounce-dog3" alt="Loading">
          </div>
        </div>
      `;
    } else {
      const wrapperClass = this.isUserMessage
        ? 'flex justify-end mb-4'
        : 'flex justify-start mb-4';
      const bubbleClass = this.isUserMessage
        ? 'max-w-[82%] md:max-w-[70%] rounded-bubble bg-teal-strong px-4 py-3 text-white'
        : 'w-full rounded-bubble bg-mist p-4 text-body md:border md:border-line md:bg-card md:p-6';

      this.innerHTML = `
        <div class="${wrapperClass}">
          <div class="${bubbleClass}">
            <div class="reasoning"></div>
            <div class="markdown overflow-auto text-sm lg:text-base font-normal empty:p-0"></div>
          </div>
        </div>
      `;

      // Update content if message is provided
      if (this.message) {
        this.updateContent(this.message);
      }
    }
  }

  updateContent(message) {
    if (this.isLoading) return;

    const messageTextElement = this.querySelector('.markdown');
    if (messageTextElement) {
      // Parse markdown
      const messageHTML = marked.parse(message, { breaks: true });

      // Add syntax highlighting to code blocks
      const tempDiv = document.createElement('div');
      tempDiv.innerHTML = messageHTML;
      tempDiv.querySelectorAll('pre code').forEach((block) => {
        hljs.highlightElement(block);
      });

      messageTextElement.innerHTML = tempDiv.innerHTML;
    }
  }

  // Method to add reasoning section
  addReasoning(reasoningContent) {
    if (this.isLoading) return;

    // Create reasoning container if it doesn't exist
    const reasoningEl = this.querySelector('.reasoning');

    if (reasoningEl) {
      let reasoningContainer = this.querySelector('.reasoning details');

      if (!reasoningContainer) {
        reasoningContainer = document.createElement('details');
        reasoningContainer.className = 'mb-2 cursor-pointer list-none';
        reasoningContainer.innerHTML = `
          <summary class="inline-flex items-center gap-1.5 rounded-full bg-amber-tint text-amber-ink text-[11px] font-bold px-2.5 py-1 list-none">
            <svg data-hq-thinking-icon class="h-3 w-3 animate-spin" viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 0 1 8-8V0C5.373 0 0 5.373 0 12h4z"></path>
            </svg>
            <span>Thinking</span>
          </summary>
        `;

        const reasoningContentElement = document.createElement('div');
        reasoningContentElement.dataset.hqReasoning = '';
        reasoningContentElement.className = 'text-sm text-muted pl-4 pt-2';
        reasoningContainer.appendChild(reasoningContentElement);

        reasoningEl.appendChild(reasoningContainer);
      }

      // Update reasoning content
      const contentElement = reasoningContainer.querySelector(
        '[data-hq-reasoning]',
      );
      if (contentElement) {
        contentElement.textContent += reasoningContent;
      }
    }
  }

  // Stop the spinner once the response has finished streaming.
  finishReasoning() {
    const icon = this.querySelector('[data-hq-thinking-icon]');
    if (!icon) return;
    icon.classList.remove('animate-spin');
    icon.outerHTML =
      '<svg data-hq-thinking-icon class="h-3 w-3" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/></svg>';
  }
}

// Define the custom element
customElements.define('message-bubble', MessageBubble);

export default MessageBubble;
