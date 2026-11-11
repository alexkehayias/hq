import '/components/hq-file-tree.js';
import '/components/hq-icon.js';
import '/components/hq-nav-rail.js';
import '/components/hq-terminal.js';
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
  const sessionTitle = document.getElementById('session-title');

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
        if (data?.title) sessionTitle.textContent = data.title;

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

        // If a response is still being generated, attach to its stream and
        // render the remaining output as it arrives.
        if (data?.in_progress) {
          const assistantBubble = new MessageBubble();
          assistantBubble.setAttribute('is-user-message', 'false');
          assistantBubble.setAttribute('is-tool-call', 'false');
          assistantBubble.setAttribute('is-loading', 'true');
          document.getElementById('chat-display').prepend(assistantBubble);
          openResumeStream(sessionId, 0, assistantBubble, { content: '' });
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

  // Session workspace: a file browser and a persistent bashkit shell mounted
  // on this session's workspace.
  const terminal = document.getElementById('session-terminal');
  const workspaceTabFiles = document.getElementById('workspace-tab-files');
  const workspaceTabTerminal = document.getElementById(
    'workspace-tab-terminal',
  );
  const workspaceFilesPane = document.getElementById('workspace-files-pane');
  const workspaceTerminalPane = document.getElementById(
    'workspace-terminal-pane',
  );
  const workspacePanel = document.getElementById('workspace-panel');
  const workspaceFullscreen = document.getElementById('workspace-fullscreen');
  const workspaceRefresh = document.getElementById('workspace-refresh');
  const workspaceCollapse = document.getElementById('workspace-collapse');
  const workspaceToggle = document.getElementById('workspace-toggle');
  const workspaceResizer = document.getElementById('workspace-resizer');
  const chatBody = document.getElementById('chat-body');
  const workspaceFileTree = document.getElementById('workspace-file-tree');
  const workspaceTreeView = document.getElementById('workspace-tree-view');
  const workspaceFilesEmpty = document.getElementById('workspace-files-empty');
  const workspacePreview = document.getElementById('workspace-preview');
  const workspacePreviewBack = document.getElementById(
    'workspace-preview-back',
  );
  const workspacePreviewPath = document.getElementById(
    'workspace-preview-path',
  );
  const workspacePreviewContent = document.getElementById(
    'workspace-preview-content',
  );
  const workspacePreviewTruncated = document.getElementById(
    'workspace-preview-truncated',
  );

  let workspaceIsFullscreen = false;
  let workspaceHasFiles = false;

  const setWorkspaceFullscreen = (fullscreen) => {
    workspaceIsFullscreen = fullscreen;
    workspacePanel.classList.toggle('fullscreen', fullscreen);
    // Keep the conversation from scrolling behind the fixed panel.
    document.body.classList.toggle('overflow-hidden', fullscreen);

    const label = fullscreen ? 'Exit full screen' : 'Full screen';
    workspaceFullscreen.setAttribute('aria-pressed', String(fullscreen));
    workspaceFullscreen.title = label;
    workspaceFullscreen.setAttribute('aria-label', label);
    workspaceFullscreen
      .querySelector('hq-icon')
      ?.setAttribute('name', fullscreen ? 'collapse' : 'expand');

    syncWorkspaceFilesLayout();
    previewEditor?.refresh();
  };
  workspaceFullscreen.addEventListener('click', () =>
    setWorkspaceFullscreen(!workspaceIsFullscreen),
  );

  const encodeWorkspacePath = (path) =>
    path.split('/').map(encodeURIComponent).join('/');

  // The shell lives on the server, so after a reload its working directory may
  // differ from the initial prompt.
  const refreshTerminalPrompt = async () => {
    try {
      const response = await fetch(
        `/api/chat/${encodeURIComponent(sessionId)}/terminal`,
      );
      if (!response.ok) return;
      const { cwd } = await response.json();
      if (cwd) terminal.setPrompt(`hq:${cwd}$`);
    } catch (error) {
      console.warn('Failed to load terminal prompt:', error);
    }
  };

  const setWorkspaceTab = (tab) => {
    const filesActive = tab === 'files';
    workspaceTabFiles.setAttribute('aria-selected', String(filesActive));
    workspaceTabTerminal.setAttribute('aria-selected', String(!filesActive));
    workspaceFilesPane.classList.toggle('hidden', !filesActive);
    workspaceTerminalPane.classList.toggle('hidden', filesActive);
    // Refreshing only applies to the file browser.
    workspaceRefresh.classList.toggle('hidden', !filesActive);
    if (filesActive) {
      loadWorkspaceFiles();
      // The editor was display:none while the terminal tab was active.
      previewEditor?.refresh();
    } else {
      refreshTerminalPrompt();
      terminal.focus();
    }
  };

  workspaceTabFiles.addEventListener('click', () => setWorkspaceTab('files'));
  workspaceTabTerminal.addEventListener('click', () =>
    setWorkspaceTab('terminal'),
  );

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && workspaceIsFullscreen) {
      event.preventDefault();
      setWorkspaceFullscreen(false);
      return;
    }
    if ((event.ctrlKey || event.metaKey) && event.key === '`') {
      event.preventDefault();
      setWorkspaceTab('terminal');
    }
  });

  // ---- Files ----

  // Map a filename to one of the CodeMirror modes vendored under
  // /vendor/codemirror/mode. Unknown extensions fall back to plain text.
  const modeForPath = (path) => {
    const name = path.toLowerCase();
    if (name.endsWith('.md') || name.endsWith('.markdown')) return 'markdown';
    if (name.endsWith('.py')) return 'python';
    if (name.endsWith('.json')) return { name: 'javascript', json: true };
    if (
      name.endsWith('.js') ||
      name.endsWith('.mjs') ||
      name.endsWith('.cjs') ||
      name.endsWith('.jsx') ||
      name.endsWith('.ts') ||
      name.endsWith('.tsx')
    ) {
      return 'javascript';
    }
    if (
      name.endsWith('.sh') ||
      name.endsWith('.bash') ||
      name.endsWith('.zsh')
    ) {
      return 'shell';
    }
    if (name.endsWith('.yaml') || name.endsWith('.yml')) return 'yaml';
    if (
      name.endsWith('.html') ||
      name.endsWith('.htm') ||
      name.endsWith('.xml') ||
      name.endsWith('.svg')
    ) {
      return 'xml';
    }
    return null;
  };

  let previewEditor = null;

  // CodeMirror is a global loaded by index.html before this module runs.
  const ensurePreviewEditor = () => {
    if (previewEditor) return previewEditor;
    previewEditor = CodeMirror(workspacePreviewContent, {
      value: '',
      mode: null,
      readOnly: true,
      lineNumbers: true,
      lineWrapping: true,
    });
    previewEditor.setSize('100%', '100%');
    return previewEditor;
  };

  const isWideWorkspace = () =>
    window.matchMedia('(min-width: 1024px)').matches;

  // Full screen on a wide viewport keeps the picker and preview side by side.
  const usesSplitFilesLayout = () => workspaceIsFullscreen && isWideWorkspace();

  // ---- Panel visibility and width ----

  const WORKSPACE_WIDTH_KEY = 'hq-workspace-width';
  const WORKSPACE_COLLAPSED_KEY = 'hq-workspace-collapsed';
  const WORKSPACE_MIN_WIDTH = 320;

  // Leave room for the conversation on the left.
  const maxWorkspaceWidth = () =>
    Math.max(WORKSPACE_MIN_WIDTH, Math.min(1000, window.innerWidth - 360));

  const clampWorkspaceWidth = (width) =>
    Math.min(maxWorkspaceWidth(), Math.max(WORKSPACE_MIN_WIDTH, width));

  const applyWorkspaceWidth = (width) => {
    workspacePanel.style.setProperty(
      '--workspace-width',
      `${Math.round(clampWorkspaceWidth(width))}px`,
    );
  };

  const storedWorkspaceWidth = Number.parseInt(
    localStorage.getItem(WORKSPACE_WIDTH_KEY),
    10,
  );
  let workspaceWidth = Number.isFinite(storedWorkspaceWidth)
    ? clampWorkspaceWidth(storedWorkspaceWidth)
    : null;
  if (workspaceWidth) applyWorkspaceWidth(workspaceWidth);

  let resizingPointerId = null;

  workspaceResizer.addEventListener('pointerdown', (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    resizingPointerId = event.pointerId;
    workspaceResizer.setPointerCapture(event.pointerId);
    workspaceResizer.classList.add('dragging');
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  });

  workspaceResizer.addEventListener('pointermove', (event) => {
    if (event.pointerId !== resizingPointerId) return;
    workspaceWidth = clampWorkspaceWidth(window.innerWidth - event.clientX);
    applyWorkspaceWidth(workspaceWidth);
    previewEditor?.refresh();
  });

  const endWorkspaceResize = (event) => {
    if (event.pointerId !== resizingPointerId) return;
    resizingPointerId = null;
    workspaceResizer.classList.remove('dragging');
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
    if (workspaceWidth) {
      localStorage.setItem(
        WORKSPACE_WIDTH_KEY,
        String(Math.round(workspaceWidth)),
      );
    }
    previewEditor?.refresh();
  };

  workspaceResizer.addEventListener('pointerup', endWorkspaceResize);
  workspaceResizer.addEventListener('pointercancel', endWorkspaceResize);
  workspaceResizer.addEventListener('lostpointercapture', endWorkspaceResize);

  // Keyboard resizing for the separator role.
  workspaceResizer.addEventListener('keydown', (event) => {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    const delta = event.key === 'ArrowLeft' ? 16 : -16;
    workspaceWidth = clampWorkspaceWidth(
      (workspaceWidth ?? workspacePanel.offsetWidth) + delta,
    );
    applyWorkspaceWidth(workspaceWidth);
    localStorage.setItem(
      WORKSPACE_WIDTH_KEY,
      String(Math.round(workspaceWidth)),
    );
    previewEditor?.refresh();
  });

  const setWorkspaceCollapsed = (collapsed) => {
    chatBody.classList.toggle('workspace-collapsed', collapsed);
    workspaceCollapse.setAttribute('aria-expanded', String(!collapsed));
    workspaceToggle.setAttribute('aria-expanded', String(!collapsed));
    localStorage.setItem(WORKSPACE_COLLAPSED_KEY, collapsed ? '1' : '0');
    if (!collapsed) {
      // The editor was display:none while the panel was collapsed.
      previewEditor?.refresh();
    }
  };

  workspaceCollapse.addEventListener('click', () => {
    if (workspaceIsFullscreen) setWorkspaceFullscreen(false);
    setWorkspaceCollapsed(true);
  });
  workspaceToggle.addEventListener('click', () => {
    const collapsed = !chatBody.classList.contains('workspace-collapsed');
    if (collapsed && workspaceIsFullscreen) setWorkspaceFullscreen(false);
    setWorkspaceCollapsed(collapsed);
  });

  setWorkspaceCollapsed(localStorage.getItem(WORKSPACE_COLLAPSED_KEY) === '1');

  let currentWorkspaceFile = null;

  // Full screen with an empty workspace shows the empty state on its own —
  // there is nothing to pick from, so the file picker column is unnecessary.
  const isEmptyWorkspaceFullscreen = () =>
    workspaceIsFullscreen && !workspaceHasFiles;

  const syncWorkspaceFilesLayout = () => {
    const emptyFullscreen = isEmptyWorkspaceFullscreen();
    workspacePanel.classList.toggle('empty-workspace', emptyFullscreen);
    workspaceFilesEmpty.classList.toggle('hidden', !emptyFullscreen);
    if (emptyFullscreen) {
      workspaceTreeView.classList.add('hidden');
      workspacePreview.classList.add('hidden');
      return;
    }
    if (usesSplitFilesLayout()) {
      workspaceTreeView.classList.remove('hidden');
      workspacePreview.classList.remove('hidden');
      if (!currentWorkspaceFile) {
        const editor = ensurePreviewEditor();
        editor.setOption('mode', null);
        editor.setValue('Select a file to preview.');
      }
      return;
    }
    workspaceTreeView.classList.toggle('hidden', !!currentWorkspaceFile);
    workspacePreview.classList.toggle('hidden', !currentWorkspaceFile);
  };

  const showWorkspaceTree = () => {
    if (usesSplitFilesLayout()) return;
    currentWorkspaceFile = null;
    workspaceFileTree.removeAttribute('selected');
    // Removing the attribute re-renders the tree, but call render() as well so
    // the highlight is cleared even if the attribute was already absent.
    workspaceFileTree.render?.();
    syncWorkspaceFilesLayout();
  };

  const loadWorkspaceFiles = async () => {
    try {
      const response = await fetch(
        `/api/chat/${encodeURIComponent(sessionId)}/workspace`,
      );
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const { files } = await response.json();
      workspaceHasFiles = (files || []).length > 0;
      if (!workspaceHasFiles) {
        currentWorkspaceFile = null;
        workspaceFileTree.removeAttribute('selected');
      }
      workspaceFileTree.setAttribute('files', JSON.stringify(files || []));
      syncWorkspaceFilesLayout();
    } catch (error) {
      console.warn('Failed to load workspace files:', error);
    }
  };

  // Guards against an earlier file's response overwriting a newer selection.
  let previewRequest = 0;

  const openWorkspaceFile = async (path) => {
    const requestId = ++previewRequest;

    currentWorkspaceFile = path;
    workspaceFileTree.setAttribute('selected', path);
    workspacePreviewPath.textContent = path;
    workspacePreviewTruncated.classList.add('hidden');
    syncWorkspaceFilesLayout();

    const editor = ensurePreviewEditor();
    editor.setOption('mode', null);
    editor.setValue('Loading…');

    try {
      const response = await fetch(
        `/api/chat/${encodeURIComponent(sessionId)}/workspace/${encodeWorkspacePath(path)}`,
      );
      if (!response.ok) {
        const message =
          response.status === 415
            ? 'Binary files can’t be previewed.'
            : (await response.text()) || `Request failed (${response.status})`;
        throw new Error(message);
      }
      const result = await response.json();
      if (requestId !== previewRequest) return;

      workspacePreviewPath.textContent = result.path || path;
      editor.setOption('mode', modeForPath(result.path || path));
      editor.setValue(result.content || '');
      workspacePreviewTruncated.classList.toggle('hidden', !result.truncated);
    } catch (error) {
      if (requestId !== previewRequest) return;
      editor.setOption('mode', null);
      editor.setValue(error.message || String(error));
    }
    editor.refresh();
  };

  workspaceFileTree.addEventListener('file-select', (event) => {
    openWorkspaceFile(event.detail.path);
  });

  workspacePreviewBack.addEventListener('click', showWorkspaceTree);

  window.addEventListener('resize', () => {
    if (workspaceWidth) applyWorkspaceWidth(workspaceWidth);
    syncWorkspaceFilesLayout();
    previewEditor?.refresh();
  });

  workspaceRefresh.addEventListener('click', () => {
    loadWorkspaceFiles();
  });

  loadWorkspaceFiles();
  refreshTerminalPrompt();

  // Drop the trailing newline so command output doesn't leave a blank line.
  const formatTerminalOutput = (text) =>
    text.replace(/\r\n/g, '\n').replace(/\n$/, '');

  // bashkit caps output at 1 MB, but that can still be hundreds of thousands
  // of short lines. Render only the newest slice so the page stays responsive.
  const MAX_OUTPUT_CHARS = 100_000;
  const MAX_OUTPUT_LINES = 2_000;

  const printTerminalOutput = (text, tone) => {
    let output = formatTerminalOutput(text);
    let truncated = false;

    // Trim by characters first so splitting can't explode on huge output.
    if (output.length > MAX_OUTPUT_CHARS) {
      output = output.slice(-MAX_OUTPUT_CHARS);
      truncated = true;
    }

    const lines = output.split('\n');
    if (lines.length > MAX_OUTPUT_LINES) {
      output = lines.slice(-MAX_OUTPUT_LINES).join('\n');
      truncated = true;
    }

    if (truncated) {
      terminal.print('… output truncated for display …', 'muted');
    }
    terminal.print(output, tone);
  };

  terminal.addEventListener('command', async (event) => {
    const line = event.detail.line;

    // `clear` is a display concern, not a shell command.
    if (line.trim() === 'clear') {
      terminal.clear();
      return;
    }

    terminal.setAttribute('disabled', '');
    try {
      const response = await fetch(
        `/api/chat/${encodeURIComponent(sessionId)}/terminal`,
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ command: line }),
        },
      );
      if (!response.ok) {
        throw new Error(
          (await response.text()) || `Request failed (${response.status})`,
        );
      }

      const result = await response.json();
      if (result.stdout) printTerminalOutput(result.stdout);
      if (result.stderr) printTerminalOutput(result.stderr, 'error');
      if (result.exit_code !== 0) {
        terminal.print(`[exit ${result.exit_code}]`, 'muted');
      }
      if (result.truncated) terminal.print('[output truncated]', 'muted');
      if (result.cwd) terminal.setPrompt(`hq:${result.cwd}$`);
    } catch (error) {
      terminal.print(error.message || String(error), 'error');
    } finally {
      terminal.removeAttribute('disabled');
      terminal.focus();
      // Commands can create or remove files; keep the browser in sync.
      loadWorkspaceFiles();
    }
  });

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

  // iPhone photos are HEIC and up to 48MP. iOS only transcodes them to JPEG
  // when picked in Safari; a standalone PWA hands over the raw HEIC, so we
  // decode and re-encode here. Downscale before drawing: iOS caps canvas
  // dimensions and a full-resolution canvas makes `toBlob` return null.
  const MAX_IMAGE_DIMENSION = 2048;

  // `createImageBitmap` decodes HEIC; `<img>` is the fallback for browsers
  // that lack it.
  const decodeImage = async (file) => {
    if (typeof createImageBitmap === 'function') {
      try {
        return await createImageBitmap(file, {
          imageOrientation: 'from-image',
        });
      } catch {
        // Fall through to the <img> path.
      }
    }
    return await new Promise((resolve, reject) => {
      const url = URL.createObjectURL(file);
      const img = new Image();
      img.onload = () => {
        URL.revokeObjectURL(url);
        resolve(img);
      };
      img.onerror = () => {
        URL.revokeObjectURL(url);
        reject(new Error('Could not read image file'));
      };
      img.src = url;
    });
  };

  const convertToJpeg = async (file) => {
    const source = await decodeImage(file);
    const sourceWidth = source.naturalWidth ?? source.width;
    const sourceHeight = source.naturalHeight ?? source.height;
    const scale = Math.min(
      1,
      MAX_IMAGE_DIMENSION / Math.max(sourceWidth, sourceHeight),
    );
    const width = Math.max(1, Math.round(sourceWidth * scale));
    const height = Math.max(1, Math.round(sourceHeight * scale));

    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext('2d');
    // JPEG has no alpha channel; fill white so transparent areas don't
    // turn black.
    ctx.fillStyle = '#ffffff';
    ctx.fillRect(0, 0, width, height);
    ctx.drawImage(source, 0, 0, width, height);
    source.close?.();

    const blob = await new Promise((resolve) =>
      canvas.toBlob(resolve, 'image/jpeg', 0.92),
    );
    if (!blob) {
      throw new Error('Could not convert image');
    }
    return blob;
  };

  const buildChip = (attachment) => {
    const isError = attachment.status === 'error';
    const chip = document.createElement('div');
    chip.className = isError
      ? 'flex items-center gap-2 pl-1 pr-2 py-1 rounded-[12px] border border-rust-tint bg-rust-tint max-w-[22rem]'
      : 'flex items-center gap-2 pl-1 pr-2 py-1 rounded-[12px] border border-line bg-card max-w-[16rem]';

    if (isError) {
      const icon = document.createElement('span');
      icon.className =
        'w-8 h-8 flex items-center justify-center shrink-0 text-rust';
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
        'w-8 h-8 flex items-center justify-center shrink-0 text-muted';
      icon.textContent = '📄';
      chip.appendChild(icon);
    }

    const label = document.createElement('span');
    if (isError) {
      label.className = 'text-xs text-rust break-words min-w-0';
      label.textContent = attachment.error || 'Upload failed';
    } else {
      label.className = 'text-xs text-body truncate';
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
    removeButton.className = 'shrink-0 text-muted hover:text-ink';
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
    loadWorkspaceFiles();
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
    loadWorkspaceFiles();
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

  // Tracks the active resume stream so a new one replaces any previous.
  let activeResumeStream = null;

  // Apply a streamed delta to an assistant bubble, accumulating content.
  const applyDelta = (bubble, accum, parsed) => {
    const choice = parsed.choices?.[0];
    if (!choice) return;
    const content = choice.delta?.content;
    const reasoning =
      choice.delta?.reasoning || choice.delta?.reasoning_content;
    if (content) {
      accum.content += content;
      bubble.setAttribute('is-loading', 'false');
      bubble.updateContent(accum.content);
    }
    if (reasoning) {
      bubble.setAttribute('is-loading', 'false');
      bubble.addReasoning(reasoning);
    }
  };

  // Reconnect to an in-flight response for a session. `after` is the resume
  // cursor: 0 replays the whole turn (used on page load), while a nonzero value
  // continues after the chunks already rendered.
  const openResumeStream = (id, after, assistantBubble, accum) => {
    if (activeResumeStream) {
      activeResumeStream.close();
      activeResumeStream = null;
    }
    const es = new EventSource(
      `/api/chat/${encodeURIComponent(id)}/stream?after=${after}`,
    );
    activeResumeStream = es;

    es.onmessage = (event) => {
      if (event.data === '[DONE]') return;
      let parsed;
      try {
        parsed = JSON.parse(event.data);
      } catch (error) {
        console.error('Error parsing JSON:', error);
        return;
      }
      applyDelta(assistantBubble, accum, parsed);
    };

    // The server sends a terminal `done` event when the turn finishes.
    es.addEventListener('done', () => {
      assistantBubble.finishReasoning();
      es.close();
      if (activeResumeStream === es) activeResumeStream = null;
    });

    es.onerror = () => {
      // A 204 (no active turn) closes the connection for good; otherwise
      // EventSource reconnects automatically with Last-Event-ID.
      if (es.readyState === EventSource.CLOSED) {
        assistantBubble.finishReasoning();
        if (activeResumeStream === es) activeResumeStream = null;
      }
    };
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

    // Accumulates streamed content so a reconnect can continue the same
    // bubble. `lastEventId` is the SSE cursor for resuming after a read error.
    const accum = { content: '' };
    let lastEventId = 0;
    let pendingEventId = null;
    let finished = false;

    fetch('/api/chat', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(chatRequest),
    })
      .then((response) => {
        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let buffer = '';

        function read() {
          reader
            .read()
            .then(({ done, value }) => {
              if (done) {
                finished = true;
                assistantBubble.finishReasoning();
                return;
              }

              // Convert Uint8Array to string
              const chunk = decoder.decode(value, { stream: true });
              buffer += chunk;

              // Process complete lines
              const lines = buffer.split('\n');
              buffer = lines.pop(); // Keep incomplete line in buffer

              lines.forEach((line) => {
                // Remember the event id, but don't commit it as the resume
                // cursor yet: the id line precedes the data line, so a read
                // error between the two must not skip this event's payload.
                if (line.startsWith('id:')) {
                  const id = Number.parseInt(line.slice(3).trim(), 10);
                  if (!Number.isNaN(id)) pendingEventId = id;
                  return;
                }
                // Skip non-data lines (e.g. the terminal `event: done`, or
                // keep-alive comments) without trying to parse them.
                if (!line.startsWith('data:')) return;
                const data = line.slice(5).trim();
                if (data === '' || data === '[DONE]') {
                  // No payload to apply; safe to advance past this event.
                  if (pendingEventId !== null) {
                    lastEventId = pendingEventId;
                    pendingEventId = null;
                  }
                  return;
                }
                try {
                  applyDelta(assistantBubble, accum, JSON.parse(data));
                  // Advance the cursor only once the payload has been applied.
                  if (pendingEventId !== null) {
                    lastEventId = pendingEventId;
                    pendingEventId = null;
                  }
                } catch (e) {
                  console.error('Error parsing JSON:', e);
                }
              });

              read();
            })
            .catch((error) => {
              console.error('Read error:', error);
              // Resume the in-flight response from where the reader stopped.
              if (finished) {
                assistantBubble.finishReasoning();
              } else {
                openResumeStream(
                  sessionId,
                  lastEventId,
                  assistantBubble,
                  accum,
                );
              }
            });
        }

        read();
      })
      .catch((error) => {
        console.error('Chat request failed:', error);
        openResumeStream(sessionId, lastEventId, assistantBubble, accum);
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
