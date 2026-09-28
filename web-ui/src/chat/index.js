import AttachmentBubble from './attachment-bubble.js';
import MessageBubble from './message-bubble.js';

// Polyfill to get a UUID with a fallback if running in a non https
// session or localhost
if (!window.crypto.randomUUID) {
  window.crypto.randomUUID = () =>
    ([1e7] + -1e3 + -4e3 + -8e3 + -1e11).replace(/[018]/g, (c) =>
      (
        c ^
        (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (c / 4)))
      ).toString(16),
    );
}

document.addEventListener('DOMContentLoaded', () => {
  // Preload dog images to avoid fetching them each time
  const dogImages = [];
  for (let i = 1; i <= 3; i++) {
    const img = new Image();
    img.src = `./img/dog${i}.png`;
    dogImages.push(img);
  }

  const urlParams = new URLSearchParams(window.location.search);
  let sessionId;
  const maybeSessionId = urlParams.get('session_id');
  if (maybeSessionId) {
    sessionId = maybeSessionId;
    fetch(`/api/chat/${sessionId}`, {
      method: 'GET',
      headers: {
        'Content-Type': 'application/json',
      },
    })
      .then((response) => {
        if (response.status === 404) {
          console.log('Session not found, starting new conversation');
          return Promise.resolve(null);
        } else if (!response.ok) {
          throw new Error(`HTTP error! status: ${response.status}`);
        }
        return response.json();
      })
      .then((data) => {
        // Only process transcript if we have data
        if (data?.transcript) {
          data.transcript.forEach((message) => {
            const isUser = message.role === 'user';
            const isAssistant = message.role === 'assistant';
            const _isSystem = message.role === 'system';

            // Messages with attachments store content as an array of parts;
            // older messages store it as a plain string.
            const parts = Array.isArray(message.content)
              ? message.content
              : null;
            const textContent = parts
              ? parts
                  .filter((part) => part.type === 'text')
                  .map((part) => part.text)
                  .join('\n')
              : message.content;
            const imageParts = parts
              ? parts.filter((part) => part.type === 'image_url')
              : [];

            const isToolCall =
              message.role === 'tool' || (isAssistant && !message.content);

            if (!isToolCall && (isUser || isAssistant)) {
              let anchor = null;
              if (textContent) {
                const bubble = new MessageBubble();
                bubble.setAttribute('message', textContent);
                bubble.setAttribute('is-user-message', isUser.toString());
                bubble.setAttribute('is-tool-call', isToolCall.toString());
                document.getElementById('chat-display').prepend(bubble);
                anchor = bubble;
              }
              if (isUser && imageParts.length > 0) {
                const files = imageParts.map((part) => {
                  const url = part.image_url?.url || '';
                  const filename = url.split('/').pop() || 'image';
                  return {
                    filename,
                    content_type: 'image/*',
                    url: `/api/files/${encodeURIComponent(sessionId)}/${encodeURIComponent(filename)}`,
                  };
                });
                const attachmentBubble = new AttachmentBubble();
                attachmentBubble.setAttribute('files', JSON.stringify(files));
                if (anchor) {
                  anchor.after(attachmentBubble);
                } else {
                  document
                    .getElementById('chat-display')
                    .prepend(attachmentBubble);
                }
              }
            }
            if (isAssistant && isToolCall) {
              const bubble = new MessageBubble();

              const toolCallMessages = [];
              for (const t of message.tool_calls) {
                const toolFn = t.function;
                toolCallMessages.push(
                  `**Tool call**: \`${toolFn.name}\`\n**Args**:\n\n\`\`\`\n${toolFn.arguments}\n\`\`\``,
                );
              }

              bubble.setAttribute('message', toolCallMessages.join('\n\n'));
              bubble.setAttribute('is-user-message', 'false');
              bubble.setAttribute('is-tool-call', 'true');
              document.getElementById('chat-display').prepend(bubble);
            }
          });
        }
        scrollToBottom();
      })
      .catch((error) => console.error('Error:', error));
  } else {
    sessionId = crypto.randomUUID();
    history.replaceState({}, '', `?session_id=${sessionId}`);
  }

  const chatContainer = document.getElementById('chat-container');
  const chatInput = document.getElementById('chat-input');
  const sendButton = document.getElementById('send-button');
  const attachButton = document.getElementById('attach-button');
  const fileInput = document.getElementById('file-input');
  const chipsContainer = document.getElementById('attachment-chips');

  const scrollToBottom = () => {
    chatContainer.scrollTop = chatContainer.scrollHeight;
  };

  // Auto-resize textarea up to 7 lines
  const autoResize = () => {
    chatInput.style.height = 'auto';
    const newHeight = Math.min(chatInput.scrollHeight, 7 * 24); // Approximate line height
    chatInput.style.height = `${newHeight}px`;
  };

  chatInput.addEventListener('input', autoResize);

  // Files uploaded for the next message. Each entry is
  // { file, objectUrl, status: 'uploading' | 'done' | 'error', data?, error? }.
  let pendingAttachments = [];

  // The send button is only meaningful once there's something to send: typed
  // text or at least one fully-uploaded attachment.
  const updateSendButtonState = () => {
    const hasText = chatInput.value.trim() !== '';
    const hasReadyAttachment = pendingAttachments.some(
      (attachment) => attachment.status === 'done',
    );
    sendButton.disabled = !hasText && !hasReadyAttachment;
  };

  chatInput.addEventListener('input', updateSendButtonState);
  updateSendButtonState();

  // Types the upload API accepts, plus image extensions it doesn't (iPhone
  // photos are HEIC) which we re-encode to JPEG before uploading.
  const ALLOWED_EXTENSIONS = ['png', 'jpg', 'jpeg', 'gif', 'webp'];
  const IMAGE_EXTENSIONS = [
    'png',
    'jpg',
    'jpeg',
    'gif',
    'webp',
    'heic',
    'heif',
    'avif',
    'bmp',
    'tiff',
    'tif',
  ];

  const extensionOf = (name) => {
    const dot = name.lastIndexOf('.');
    return dot === -1 ? '' : name.slice(dot + 1).toLowerCase();
  };

  const isImageFile = (file) =>
    file.type.startsWith('image/') ||
    IMAGE_EXTENSIONS.includes(extensionOf(file.name));

  const needsConversion = (file) =>
    isImageFile(file) && !ALLOWED_EXTENSIONS.includes(extensionOf(file.name));

  const convertToJpeg = (file) =>
    new Promise((resolve, reject) => {
      const url = URL.createObjectURL(file);
      const img = new Image();
      img.onload = () => {
        URL.revokeObjectURL(url);
        const canvas = document.createElement('canvas');
        canvas.width = img.naturalWidth;
        canvas.height = img.naturalHeight;
        const ctx = canvas.getContext('2d');
        // JPEG has no alpha channel; fill white so transparent areas don't
        // turn black.
        ctx.fillStyle = '#ffffff';
        ctx.fillRect(0, 0, canvas.width, canvas.height);
        ctx.drawImage(img, 0, 0);
        canvas.toBlob(
          (blob) =>
            blob ? resolve(blob) : reject(new Error('Could not convert image')),
          'image/jpeg',
          0.92,
        );
      };
      img.onerror = () => {
        URL.revokeObjectURL(url);
        reject(new Error('Could not read image file'));
      };
      img.src = url;
    });

  const buildChip = (attachment) => {
    const isError = attachment.status === 'error';
    const chip = document.createElement('div');
    chip.className = isError
      ? 'flex items-center gap-2 pl-1 pr-2 py-1 rounded-lg border border-red-300 dark:border-red-700 bg-red-50 dark:bg-red-900/20 max-w-[22rem]'
      : 'flex items-center gap-2 pl-1 pr-2 py-1 rounded-lg border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-800 max-w-[16rem]';

    if (isError) {
      const icon = document.createElement('span');
      icon.className =
        'w-8 h-8 flex items-center justify-center shrink-0 text-red-500';
      icon.textContent = '⚠️';
      chip.appendChild(icon);
    } else if (attachment.objectUrl) {
      const img = document.createElement('img');
      img.src = attachment.objectUrl;
      img.alt = attachment.file.name;
      img.className = 'w-8 h-8 rounded object-cover shrink-0';
      chip.appendChild(img);
    } else {
      const icon = document.createElement('span');
      icon.className =
        'w-8 h-8 flex items-center justify-center shrink-0 text-gray-400';
      icon.textContent = '📄';
      chip.appendChild(icon);
    }

    const label = document.createElement('span');
    if (isError) {
      label.className =
        'text-xs text-red-700 dark:text-red-300 break-words min-w-0';
      label.textContent = attachment.error || 'Upload failed';
    } else {
      label.className = 'text-xs text-gray-700 dark:text-gray-200 truncate';
      label.textContent =
        attachment.status === 'uploading' ? 'Uploading…' : attachment.file.name;
    }
    chip.appendChild(label);

    const removeButton = document.createElement('button');
    removeButton.type = 'button';
    removeButton.setAttribute(
      'aria-label',
      isError ? 'Dismiss error' : 'Remove attachment',
    );
    removeButton.className =
      'shrink-0 text-gray-400 hover:text-gray-600 dark:hover:text-gray-200';
    removeButton.textContent = '×';
    removeButton.addEventListener('click', () => removeAttachment(attachment));
    chip.appendChild(removeButton);

    return chip;
  };

  const renderChips = () => {
    chipsContainer.innerHTML = '';
    for (const attachment of pendingAttachments) {
      chipsContainer.appendChild(buildChip(attachment));
    }
    updateSendButtonState();
  };

  // Remove an uploaded file from the session workspace on the server. The chip
  // is already gone from the UI by the time this runs, so failures are only
  // logged.
  const deleteUploadedFile = async (fileData) => {
    try {
      const response = await fetch(
        `/api/files/${encodeURIComponent(sessionId)}/${encodeURIComponent(fileData.filename)}`,
        { method: 'DELETE' },
      );
      if (!response.ok && response.status !== 404) {
        console.warn(`Failed to delete attachment (${response.status})`);
      }
    } catch (error) {
      console.warn('Failed to delete attachment:', error);
    }
  };

  const removeAttachment = (attachment) => {
    // Mark it so an upload still in flight cleans itself up when it finishes.
    attachment.removed = true;
    pendingAttachments = pendingAttachments.filter((a) => a !== attachment);
    if (attachment.objectUrl) {
      URL.revokeObjectURL(attachment.objectUrl);
    }
    if (attachment.status === 'done' && attachment.data) {
      deleteUploadedFile(attachment.data);
    }
    renderChips();
  };

  const uploadFile = async (file) => {
    const attachment = {
      file,
      objectUrl: null,
      status: 'uploading',
    };
    pendingAttachments.push(attachment);
    renderChips();

    let blob = file;
    let filename = file.name;

    if (needsConversion(file)) {
      try {
        blob = await convertToJpeg(file);
        filename = `${file.name.replace(/\.[^.]+$/, '')}.jpg`;
      } catch (error) {
        attachment.status = 'error';
        attachment.error = error.message;
        renderChips();
        return;
      }
    }

    // The chip may have been dismissed while we were converting the image.
    if (attachment.removed) {
      return;
    }

    attachment.objectUrl = URL.createObjectURL(blob);
    renderChips();

    const formData = new FormData();
    formData.append('session_id', sessionId);
    formData.append('file', blob, filename);

    try {
      const response = await fetch('/api/files', {
        method: 'POST',
        body: formData,
      });
      if (!response.ok) {
        const text = await response.text();
        throw new Error(text || `Upload failed (${response.status})`);
      }
      const data = await response.json();
      attachment.status = 'done';
      attachment.data = data.files[0];
      // The chip was dismissed while the upload was in flight; don't leave the
      // file behind on the server.
      if (attachment.removed) {
        deleteUploadedFile(attachment.data);
      }
    } catch (error) {
      console.error('Upload failed:', error);
      attachment.status = 'error';
      attachment.error = error.message;
    }
    renderChips();
  };

  attachButton.addEventListener('click', () => fileInput.click());
  fileInput.addEventListener('change', () => {
    for (const file of fileInput.files) {
      uploadFile(file);
    }
    // Reset so picking the same file again still fires a change event.
    fileInput.value = '';
  });

  // Unix-style word navigation helpers
  const findPreviousWordBoundary = (text, pos) => {
    // Skip whitespace
    while (pos > 0 && /\s/.test(text[pos - 1])) {
      pos--;
    }
    // Skip word characters
    while (pos > 0 && /\S/.test(text[pos - 1])) {
      pos--;
    }
    return pos;
  };

  const findNextWordBoundary = (text, pos) => {
    // If we're on whitespace, skip it first
    while (pos < text.length && /\s/.test(text[pos])) {
      pos++;
    }
    // Now skip to the end of the word (stop before whitespace or punctuation)
    const punctuation = /[.!?;:,]/;
    while (
      pos < text.length &&
      /\S/.test(text[pos]) &&
      !punctuation.test(text[pos])
    ) {
      pos++;
    }
    return pos;
  };

  const moveCursorByWord = (direction) => {
    const text = chatInput.value;
    const cursorPos = chatInput.selectionStart;

    let newPos;
    if (direction === -1) {
      // Move backward
      newPos = findPreviousWordBoundary(text, cursorPos);
    } else {
      // Move forward
      newPos = findNextWordBoundary(text, cursorPos);
    }

    chatInput.setSelectionRange(newPos, newPos);
  };

  const sendMessage = () => {
    const message = chatInput.value.trim();
    const hasReadyAttachment = pendingAttachments.some(
      (attachment) => attachment.status === 'done',
    );
    if (message === '' && !hasReadyAttachment) return;

    // Create user message bubble
    const userBubble = new MessageBubble();
    userBubble.setAttribute('message', message);
    userBubble.setAttribute('is-user-message', 'true');
    userBubble.setAttribute('is-tool-call', 'false');
    userBubble.setAttribute('is-loading', 'false');
    document.getElementById('chat-display').prepend(userBubble);

    // Create assistant message bubble
    const assistantBubble = new MessageBubble();
    assistantBubble.setAttribute('is-user-message', 'false');
    assistantBubble.setAttribute('is-tool-call', 'false');
    assistantBubble.setAttribute('is-loading', 'true');
    document.getElementById('chat-display').prepend(assistantBubble);

    // Move finished uploads into their own bubble. Inserted directly after the
    // user bubble so it renders above the text in the reversed chat display.
    const readyAttachments = pendingAttachments.filter(
      (attachment) => attachment.status === 'done',
    );
    const attachments = readyAttachments.map((attachment) => ({
      filename: attachment.data.filename,
      content_type: attachment.data.content_type,
    }));
    if (readyAttachments.length > 0) {
      const files = readyAttachments.map((attachment) => ({
        filename: attachment.data.filename,
        content_type: attachment.data.content_type,
        objectUrl: attachment.objectUrl,
      }));
      const attachmentBubble = new AttachmentBubble();
      attachmentBubble.setAttribute('files', JSON.stringify(files));
      userBubble.after(attachmentBubble);

      pendingAttachments = pendingAttachments.filter(
        (attachment) => attachment.status !== 'done',
      );
      renderChips();
    }

    scrollToBottom();

    const chatRequest = {
      session_id: sessionId,
      message: message,
      attachments: attachments,
    };

    fetch('/api/chat', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(chatRequest),
    }).then((response) => {
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';

      let contentAccum = '';

      function read() {
        reader
          .read()
          .then(({ done, value }) => {
            if (done) {
              console.log('Stream complete');
              return;
            }

            // Convert Uint8Array to string
            const chunk = decoder.decode(value, { stream: true });
            buffer += chunk;

            // Process complete lines
            const lines = buffer.split('\n');
            buffer = lines.pop(); // Keep incomplete line in buffer

            lines.forEach((line) => {
              if (line.startsWith('data: ')) {
                const data = line.slice(6).trim();
                if (data === '[DONE]') {
                  console.log('Stream finished');
                  return;
                }
                try {
                  const parsed = JSON.parse(data);
                  const content = parsed.choices[0].delta.content;
                  const reasoning =
                    parsed.choices[0].delta.reasoning ||
                    parsed.choices[0].delta.reasoning_content;

                  // TODO: Handle rendering tool calls
                  const _toolCalls = parsed.choices[0].delta.tool_calls;
                  const _toolCallsFinished =
                    parsed.choices[0].finish_reason === 'tool_calls';

                  // Handle content delta
                  if (content) {
                    contentAccum += content;
                    assistantBubble.setAttribute('is-loading', 'false');
                    assistantBubble.updateContent(contentAccum);
                  }

                  // Handle reasoning delta
                  if (reasoning) {
                    // Tool call deltas are interleaved with reasoning
                    // deltas as the model thinks through the tools to
                    // use and retrieves the results so we wait until
                    // it's dont to render the message bubble.
                    assistantBubble.setAttribute('is-loading', 'false');
                    // FIX: There is no div to add this to because it
                    // might still be in the loading state.
                    // Need to refactor reasoning to appear in a message bubble
                    assistantBubble.addReasoning(reasoning);
                  }
                } catch (e) {
                  console.error('Error parsing JSON:', e);
                }
              }
            });

            read();
          })
          .catch((error) => {
            console.error('Read error:', error);
          });
      }

      read();
    });

    chatInput.value = ''; // Clear input field
    chatInput.style.height = 'auto'; // Reset height after sending
    updateSendButtonState();
  };

  // Handle initial prompt from query parameter (after functions are defined)
  const maybePrompt = urlParams.get('prompt');
  if (maybePrompt) {
    chatInput.value = decodeURIComponent(maybePrompt);
    autoResize();
    // Auto-send the prompt
    sendMessage();
  }

  sendButton.addEventListener('click', () => sendMessage());
  chatInput.addEventListener('keydown', (e) => {
    if (e.metaKey && e.key === 'Enter') {
      e.preventDefault();
      sendMessage();
    }

    // Unix-style word navigation
    if (e.altKey && e.code === 'KeyB') {
      e.preventDefault();
      moveCursorByWord(-1);
    } else if (e.altKey && e.code === 'KeyF') {
      e.preventDefault();
      moveCursorByWord(1);
    }
  });
});
