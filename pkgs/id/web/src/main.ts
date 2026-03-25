/**
 * Main entry point for the id web interface.
 * Initializes HTMX, the ProseMirror editor, and theme switching.
 */

import htmx from 'htmx.org';
import { type EditorInstance, getEditorState } from './editor';
import { initCollab, type CollabConnection } from './collab';
import { initTheme, setTheme, cycleTheme, type Theme } from './theme';

// Expose htmx globally for HTMX attributes in HTML
declare global {
  interface Window {
    htmx: typeof htmx;
    idApp: IdApp;
    cycleTheme: typeof cycleTheme;
  }
}

interface IdApp {
  collab: CollabConnection | null;
  tagsWs: WebSocket | null;
  setTheme: (theme: Theme) => void;
  openEditor: (docId: string) => Promise<void>;
  closeEditor: () => void;
  saveFile: () => Promise<void>;
  createFile: (event: Event) => Promise<void>;
  downloadFile: (format: string) => Promise<void>;
  renameFile: () => Promise<void>;
  connectTagsWs: () => void;
  disconnectTagsWs: () => void;
  loadFileTags: (filename: string) => Promise<void>;
  renderEditorTags: (tags: Array<{ key: string; value: string | null }>) => void;
  addTagInline: () => Promise<void>;
  removeTag: (subject: string, key: string, value: string | null) => Promise<void>;
  bulkAddTag: () => Promise<void>;
  bulkClearSelection: () => void;
  initBulkSelect: () => void;
  navHistory: string[];
  currentPath: string;
  lastFilename: string | null;
  lastFilePath: string | null;
}

/**
 * Update the editor status indicator.
 */
function updateStatus(status: 'connecting' | 'connected' | 'disconnected' | 'error'): void {
  const statusEl = document.getElementById('editor-status');
  if (!statusEl) return;
  
  const statusText: Record<string, string> = {
    connecting: 'connecting...',
    connected: 'connected',
    disconnected: 'disconnected',
    error: 'error',
  };
  
  statusEl.textContent = statusText[status] || status;
  statusEl.className = `editor-status status-${status}`;
}

/**
 * Initialize scroll-show behavior for inline header and footer.
 * 
 * Header: In normal flow at top. When scrolled past, becomes fixed and 
 *         shows on scroll-up, hides on scroll-down.
 * 
 * Footer: In normal flow at bottom. When not at bottom, becomes fixed and
 *         shows on scroll-up (with header), hides on scroll-down.
 *         Also shows when at top (with header).
 */
function initScrollShowHeader(headerSelector: string = '.editor-inline-header', footerSelector: string = '.editor-inline-footer'): (() => void) | null {
  const header = document.querySelector(headerSelector) as HTMLElement | null;
  const footer = document.querySelector(footerSelector) as HTMLElement | null;
  
  if (!header) {
    console.log('[id] scroll-show: header not found for selector:', headerSelector);
    return null;
  }
  
  console.log('[id] scroll-show: initializing for', headerSelector, 'footer selector:', footerSelector, 'footer found:', !!footer);
  
  const headerHeight = header.offsetHeight;
  const footerHeight = footer?.offsetHeight || 18;
  console.log('[id] scroll-show: headerHeight:', headerHeight, 'footerHeight:', footerHeight);
  let lastScrollTop = window.scrollY || document.documentElement.scrollTop;
  let ticking = false;
  
  const handleScroll = (): void => {
    const scrollTop = window.scrollY || document.documentElement.scrollTop;
    const windowHeight = window.innerHeight;
    const docHeight = document.documentElement.scrollHeight;
    const scrollBottom = docHeight - scrollTop - windowHeight;
    const isScrollingUp = scrollTop < lastScrollTop;
    const atTop = scrollTop <= headerHeight;
    const atBottom = scrollBottom <= footerHeight;
    
    if (!ticking) {
      window.requestAnimationFrame(() => {
        // === HEADER ===
        if (atTop) {
          // At the very top - in normal document flow
          header.classList.remove('floating', 'visible');
        } else {
          // Scrolled past header - floating behavior
          if (!header.classList.contains('floating')) {
            header.classList.add('floating');
          }
          if (isScrollingUp) {
            header.classList.add('visible');
          } else {
            header.classList.remove('visible');
          }
        }
        
        // === FOOTER ===
        if (footer) {
          if (atBottom) {
            // At the very bottom - in normal document flow
            footer.classList.remove('floating', 'visible');
          } else if (atTop) {
            // At the very top - show footer floating (with header visible)
            if (!footer.classList.contains('floating')) {
              footer.classList.add('floating');
            }
            footer.classList.add('visible');
          } else {
            // In the middle - floating behavior
            if (!footer.classList.contains('floating')) {
              footer.classList.add('floating');
            }
            if (isScrollingUp) {
              footer.classList.add('visible');
            } else {
              footer.classList.remove('visible');
            }
          }
        }
        
        lastScrollTop = scrollTop;
        ticking = false;
      });
      ticking = true;
    }
  };
  
  // Initial state check
  const scrollTop = window.scrollY || document.documentElement.scrollTop;
  const windowHeight = window.innerHeight;
  const docHeight = document.documentElement.scrollHeight;
  const scrollBottom = docHeight - scrollTop - windowHeight;
  const atTop = scrollTop <= headerHeight;
  const atBottom = scrollBottom <= footerHeight;
  
  console.log('[id] scroll-show initial state:', {
    scrollTop,
    headerHeight,
    footerHeight,
    windowHeight,
    docHeight,
    scrollBottom,
    atTop,
    atBottom,
    footer: footer ? 'found' : 'not found',
  });
  
  if (footer) {
    if (atBottom) {
      // At bottom - footer in normal flow
      console.log('[id] scroll-show: footer at bottom - normal flow');
      footer.classList.remove('floating', 'visible');
    } else if (atTop) {
      // At top - footer floating and visible
      console.log('[id] scroll-show: footer at top - floating visible');
      footer.classList.add('floating', 'visible');
    } else {
      // Middle - footer floating and hidden
      console.log('[id] scroll-show: footer in middle - floating hidden');
      footer.classList.add('floating');
      footer.classList.remove('visible');
    }
  }
  
  window.addEventListener('scroll', handleScroll, { passive: true });
  
  // Return cleanup function
  return () => {
    window.removeEventListener('scroll', handleScroll);
    header.classList.remove('floating', 'visible');
    footer?.classList.remove('floating', 'visible');
  };
}

/**
 * Update header subtitle based on navigation state.
 * Shows "p2p file sharing" on initial load, or last filename as link after navigation.
 */
function updateHeaderSubtitle(lastFilename: string | null, lastFilePath: string | null, hasHistory: boolean): void {
  const subtitle = document.getElementById('header-subtitle');
  if (!subtitle) return;
  
  if (lastFilename && lastFilePath && hasHistory) {
    // Create a link to the last file
    subtitle.innerHTML = `// <a href="${lastFilePath}" hx-get="${lastFilePath}" hx-target="#main" hx-push-url="true">${lastFilename}</a>`;
    // Re-process with HTMX so the link works
    if (window.htmx) {
      window.htmx.process(subtitle);
    }
  } else {
    subtitle.textContent = '// p2p file sharing';
  }
}

/**
 * Update back link based on app navigation history.
 * If there's history, use HTMX to navigate. Otherwise, grey out but still allow browser back.
 */
function updateBackLink(navHistory: string[], currentPath: string): void {
  const backLink = document.getElementById('back-link');
  if (!backLink) return;
  
  // Find previous path (not current)
  const prevPath = navHistory.length > 0 ? navHistory[navHistory.length - 1] : null;
  
  if (prevPath && prevPath !== currentPath) {
    // Has app history - use HTMX navigation
    backLink.classList.remove('disabled');
    backLink.setAttribute('href', prevPath);
    backLink.setAttribute('hx-get', prevPath);
    backLink.setAttribute('hx-target', '#main');
    backLink.setAttribute('hx-push-url', 'true');
    backLink.removeAttribute('onclick');
    // Re-process with HTMX
    if (window.htmx) {
      window.htmx.process(backLink);
    }
  } else {
    // No app history - grey out but use browser back as fallback
    backLink.classList.add('disabled');
    backLink.setAttribute('href', '#');
    backLink.removeAttribute('hx-get');
    backLink.removeAttribute('hx-target');
    backLink.removeAttribute('hx-push-url');
    backLink.setAttribute('onclick', 'history.back(); return false;');
  }
}

/**
 * Initialize file filter: search input and show-auto checkbox.
 * Filters .file-item elements based on data-name and data-kind attributes.
 */
function initFileFilter(): void {
  const showAutoCheckbox = document.getElementById('show-auto') as HTMLInputElement | null;
  
  if (!showAutoCheckbox) return;
  
  const applyFilter = (): void => {
    const showAuto = showAutoCheckbox?.checked || false;
    const items = document.querySelectorAll('.file-item[data-kind]');
    
    items.forEach((el) => {
      const item = el as HTMLElement;
      const kind = item.getAttribute('data-kind') || '';
      
      // Hide auto/archive unless checkbox is checked
      if ((kind === 'auto' || kind === 'archive') && !showAuto) {
        item.style.display = 'none';
        return;
      }
      
      item.style.display = '';
    });
  };
  
  // Guard against duplicate listeners (element persists across HTMX swaps)
  if (!showAutoCheckbox.dataset.filterInit) {
    showAutoCheckbox.addEventListener('change', applyFilter);
    showAutoCheckbox.dataset.filterInit = '1';
  }
  
  // Apply filter immediately (auto files hidden by default)
  applyFilter();
}

// Track cleanup function for scroll handler
let scrollCleanup: (() => void) | null = null;

/**
 * Initialize the application.
 */
function init(): void {
  // Initialize HTMX
  window.htmx = htmx;
  
  // Expose cycleTheme globally for onclick handlers
  window.cycleTheme = cycleTheme;
  
  // Configure HTMX
  htmx.config.defaultSwapStyle = 'innerHTML';
  htmx.config.historyCacheSize = 10;
  htmx.config.refreshOnHistoryMiss = true;
  
  // Initialize theme system
  initTheme();
  
  // Create app API
  const app: IdApp = {
    collab: null,
    tagsWs: null,
    setTheme,
    navHistory: [],
    currentPath: window.location.pathname,
    lastFilename: null,
    lastFilePath: null,

    /**
     * Connect to the tags WebSocket for live tag change notifications.
     * On tag events, refresh the file list on the home page.
     */
    connectTagsWs(): void {
      if (this.tagsWs && this.tagsWs.readyState <= WebSocket.OPEN) return;

      const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const wsUrl = `${wsProtocol}//${window.location.host}/ws/tags`;
      console.log('[id] Tags WS connecting:', wsUrl);
      const ws = new WebSocket(wsUrl);
      this.tagsWs = ws;

      ws.onopen = () => {
        console.log('[id] Tags WS connected');
      };

      ws.onmessage = (event: MessageEvent) => {
        try {
          const data = JSON.parse(event.data as string);
          console.log('[id] Tag event:', data);

          // On any tag change, refresh the file list if we're on the home page
          const fileListContent = document.getElementById('file-list-content');
          if (fileListContent && window.htmx) {
            // Debounce: don't refresh more than once per 500ms
            const now = Date.now();
            const lastRefresh = (window as unknown as Record<string, number>).__tagRefreshTs || 0;
            if (now - lastRefresh > 500) {
              (window as unknown as Record<string, number>).__tagRefreshTs = now;
              const searchInput = document.getElementById('file-search') as HTMLInputElement | null;
              const showDeletedCheckbox = document.getElementById('show-deleted') as HTMLInputElement | null;
              const query = searchInput?.value || '';
              const showDeleted = showDeletedCheckbox?.checked || false;
              const params = new URLSearchParams();
              if (query) params.set('search', query);
              if (showDeleted) params.set('show_deleted', 'true');
              const qs = params.toString();
              const url = qs ? `/api/files?${qs}` : '/api/files';
              window.htmx.ajax('GET', url, { target: '#file-list-content', swap: 'innerHTML' });
            }
          }

          // On editor page, refresh tags for the current file
          const editorContainer = document.getElementById('editor-container');
          if (editorContainer && data.subject) {
            const filenameEncoded = editorContainer.dataset.filename;
            const filename = filenameEncoded ? decodeURIComponent(filenameEncoded) : null;
            if (filename && data.subject === filename) {
              console.log('[id] Tag changed for current file:', data.key, '=', data.value);
              this.loadFileTags(filename);
            }
          }
        } catch {
          // Ignore non-JSON messages
        }
      };

      ws.onclose = () => {
        console.log('[id] Tags WS disconnected, reconnecting in 3s');
        this.tagsWs = null;
        setTimeout(() => this.connectTagsWs(), 3000);
      };

      ws.onerror = (err) => {
        console.warn('[id] Tags WS error:', err);
        ws.close();
      };
    },

    disconnectTagsWs(): void {
      if (this.tagsWs) {
        this.tagsWs.onclose = null; // Prevent auto-reconnect
        this.tagsWs.close();
        this.tagsWs = null;
      }
    },

    /**
     * Load tags for a file from the REST API and render them in the editor tag panel.
     */
    async loadFileTags(filename: string): Promise<void> {
      try {
        const response = await fetch(`/api/tags?subject=${encodeURIComponent(filename)}`);
        if (!response.ok) return;
        const raw = await response.json();
        // API may return { tags: [...] } or a flat array
        const tags: Array<{ key: string; value: string | null }> = Array.isArray(raw) ? raw : (raw.tags || []);
        // Filter out system tags (created, modified, deleted, archive.*)
        const userTags = tags.filter((t: { key: string }) =>
          t.key !== 'created' && t.key !== 'modified' && t.key !== 'deleted' && !t.key.startsWith('archive.')
        );
        this.renderEditorTags(userTags);
      } catch (err) {
        console.warn('[id] Failed to load file tags:', err);
      }
    },

    /**
     * Render tag pills in the editor tag panel.
     */
    renderEditorTags(tags: Array<{ key: string; value: string | null }>): void {
      const list = document.getElementById('editor-tag-list');
      if (!list) return;

      const panel = document.getElementById('editor-tag-panel');
      const filename = panel?.dataset.filename ? decodeURIComponent(panel.dataset.filename) : null;

      list.innerHTML = '';
      for (const tag of tags) {
        const pill = document.createElement('span');
        pill.className = 'tag-pill-removable';
        const label = tag.value ? `${tag.key}: ${tag.value}` : tag.key;
        pill.textContent = label;

        // Add remove button
        const removeBtn = document.createElement('button');
        removeBtn.className = 'tag-remove-btn';
        removeBtn.textContent = '\u00d7';
        removeBtn.title = 'Remove tag';
        removeBtn.onclick = (e) => {
          e.preventDefault();
          if (filename) {
            this.removeTag(filename, tag.key, tag.value);
          }
        };
        pill.appendChild(removeBtn);
        list.appendChild(pill);
      }

      if (tags.length === 0) {
        const empty = document.createElement('span');
        empty.className = 'text-muted';
        empty.textContent = 'none';
        empty.style.fontSize = '9px';
        list.appendChild(empty);
      }
    },

    /**
     * Add a tag from the inline inputs on the editor page.
     */
    async addTagInline(): Promise<void> {
      const keyInput = document.getElementById('tag-add-key') as HTMLInputElement | null;
      const valueInput = document.getElementById('tag-add-value') as HTMLInputElement | null;
      const panel = document.getElementById('editor-tag-panel');
      const filename = panel?.dataset.filename ? decodeURIComponent(panel.dataset.filename) : null;

      if (!keyInput || !filename) return;
      const key = keyInput.value.trim();
      if (!key) return;
      const value = valueInput?.value.trim() || null;

      try {
        const body: Record<string, string> = { subject: filename, key };
        if (value) body.value = value;

        const response = await fetch('/api/tags', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        });

        if (response.ok) {
          keyInput.value = '';
          if (valueInput) valueInput.value = '';
          // Tags WS will trigger a refresh, but also reload immediately
          this.loadFileTags(filename);
        } else {
          console.error('[id] Failed to add tag:', await response.text());
        }
      } catch (err) {
        console.error('[id] Add tag error:', err);
      }
    },

    /**
     * Remove a tag via the REST API.
     */
    async removeTag(subject: string, key: string, value: string | null): Promise<void> {
      try {
        const body: Record<string, string | null> = { subject, key };
        if (value !== null) body.value = value;

        const response = await fetch('/api/tags', {
          method: 'DELETE',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        });

        if (response.ok) {
          // Reload tags
          this.loadFileTags(subject);
        } else {
          console.error('[id] Failed to remove tag:', await response.text());
        }
      } catch (err) {
        console.error('[id] Remove tag error:', err);
      }
    },

    /**
     * Bulk add a tag to all selected files on the home page.
     */
    async bulkAddTag(): Promise<void> {
      const keyInput = document.getElementById('bulk-tag-key') as HTMLInputElement | null;
      const valueInput = document.getElementById('bulk-tag-value') as HTMLInputElement | null;
      if (!keyInput) return;

      const key = keyInput.value.trim();
      if (!key) return;
      const value = valueInput?.value.trim() || null;

      const checkboxes = document.querySelectorAll('.file-select:checked') as NodeListOf<HTMLInputElement>;
      const subjects: string[] = [];
      checkboxes.forEach(cb => {
        const name = cb.dataset.name;
        if (name) subjects.push(name);
      });

      if (subjects.length === 0) return;

      let successCount = 0;
      for (const subject of subjects) {
        try {
          const body: Record<string, string> = { subject, key };
          if (value) body.value = value;

          const response = await fetch('/api/tags', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
          });
          if (response.ok) successCount++;
        } catch {
          // continue with others
        }
      }

      console.log(`[id] Bulk tag: added "${key}" to ${successCount}/${subjects.length} files`);
      keyInput.value = '';
      if (valueInput) valueInput.value = '';
      // Tags WS will refresh the file list
    },

    /**
     * Clear all file selection checkboxes on the home page.
     */
    bulkClearSelection(): void {
      const checkboxes = document.querySelectorAll('.file-select:checked') as NodeListOf<HTMLInputElement>;
      checkboxes.forEach(cb => { cb.checked = false; });
      const bar = document.getElementById('bulk-action-bar');
      if (bar) bar.style.display = 'none';
    },

    /**
     * Initialize bulk select checkbox listeners on file list items.
     */
    initBulkSelect(): void {
      // Use event delegation on the file list content
      const container = document.getElementById('file-list-content');
      if (!container) return;

      container.addEventListener('change', (event: Event) => {
        const target = event.target as HTMLInputElement;
        if (!target.classList.contains('file-select')) return;

        const checked = document.querySelectorAll('.file-select:checked');
        const bar = document.getElementById('bulk-action-bar');
        const countEl = document.getElementById('bulk-count');

        if (checked.length > 0) {
          if (bar) bar.style.display = 'flex';
          if (countEl) countEl.textContent = `${checked.length} selected`;
        } else {
          if (bar) bar.style.display = 'none';
        }
      });
    },
    
    async openEditor(docId: string): Promise<void> {
      // Guard against double initialization
      if (this.collab) {
        console.log('[id] Editor already initialized');
        return;
      }
      
      const editorContainer = document.getElementById('editor-container');
      const container = document.getElementById('editor');
      if (!container || !editorContainer) {
        console.error('[id] Editor container not found');
        return;
      }
      
      try {
        // Get filename from data attribute (URL-encoded by server)
        const filenameEncoded = editorContainer.dataset.filename;
        const filename = filenameEncoded ? decodeURIComponent(filenameEncoded) : undefined;
        console.log('[id] Filename:', filename);
        
        // Track the filename and path for header subtitle
        if (filename) {
          this.lastFilename = filename;
          this.lastFilePath = this.currentPath;
        }
        
        // Clear container - server doc comes via WebSocket Init message
        container.innerHTML = '';
        
        // Connect to collab server - editor will be initialized after receiving server doc
        updateStatus('connecting');
        const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${wsProtocol}//${window.location.host}/ws/collab/${docId}`;
        console.log('[id] Connecting to WebSocket:', wsUrl);
        
        this.collab = initCollab(
          wsUrl,
          container,
          docId,
          filename,
          updateStatus,
          (editor: EditorInstance) => {
            console.log('[id] Editor initialized with server version, mode:', editor.mode);
            // Initialize scroll-show header after editor is ready
            scrollCleanup = initScrollShowHeader();
            // Update back link based on navigation history
            updateBackLink(this.navHistory, this.currentPath);
            // Enable save button
            const saveBtn = document.getElementById('save-btn') as HTMLButtonElement | null;
            if (saveBtn) saveBtn.disabled = false;
            // Load tags for the current file
            if (filename) {
              this.loadFileTags(filename);
            }
          }
        );
        console.log('[id] Collab connection initiated');
      } catch (err) {
        console.error('[id] Error initializing editor:', err);
        updateStatus('error');
      }
    },
    
    closeEditor(): void {
      // Clean up scroll handler
      if (scrollCleanup) {
        scrollCleanup();
        scrollCleanup = null;
      }
      
      if (this.collab) {
        // Disconnect first (closes WebSocket, removes event listeners)
        // This must happen before destroying the view to avoid dispatch errors
        this.collab.disconnect();
        // Then destroy the editor view
        if (this.collab.editor) {
          this.collab.editor.view.destroy();
        }
        this.collab = null;
      }
      updateStatus('disconnected');
    },

    async saveFile(): Promise<void> {
      if (!this.collab?.editor) {
        console.warn('[id] No editor to save');
        return;
      }

      const editorContainer = document.getElementById('editor-container');
      if (!editorContainer) return;

      const docId = editorContainer.dataset.docId;
      const filenameEncoded = editorContainer.dataset.filename;
      const filename = filenameEncoded ? decodeURIComponent(filenameEncoded) : null;

      if (!docId || !filename) {
        console.error('[id] Missing doc_id or filename for save');
        return;
      }

      // Get current editor state
      const state = getEditorState(this.collab.editor.view);
      const saveBtn = document.getElementById('save-btn') as HTMLButtonElement | null;

      try {
        if (saveBtn) {
          saveBtn.disabled = true;
          saveBtn.textContent = 'saving...';
        }

        const response = await fetch('/api/save', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            doc_id: docId,
            name: filename,
            doc: state.doc,
          }),
        });

        if (!response.ok) {
          const errorText = await response.text();
          console.error('[id] Save failed:', errorText);
          if (saveBtn) saveBtn.textContent = 'error!';
          setTimeout(() => { if (saveBtn) saveBtn.textContent = 'save'; }, 2000);
          return;
        }

        const result = await response.json() as { hash: string; name: string; archive_name: string | null };
        console.log('[id] File saved:', result);

        // Update the doc_id in the container to the new hash
        editorContainer.dataset.docId = result.hash;

        // Update the URL to reflect the new hash
        const newUrl = `/edit/${result.hash}`;
        window.history.replaceState(null, '', newUrl);

        if (saveBtn) {
          saveBtn.textContent = 'saved!';
          setTimeout(() => { if (saveBtn) saveBtn.textContent = 'save'; }, 2000);
        }
      } catch (err) {
        console.error('[id] Save error:', err);
        if (saveBtn) {
          saveBtn.textContent = 'error!';
          setTimeout(() => { if (saveBtn) saveBtn.textContent = 'save'; }, 2000);
        }
      }
    },

    async createFile(event: Event): Promise<void> {
      event.preventDefault();
      const input = document.getElementById('new-file-name') as HTMLInputElement | null;
      if (!input) return;

      const name = input.value.trim();
      if (!name) return;

      try {
        const response = await fetch('/api/new', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ name }),
        });

        if (!response.ok) {
          const errorText = await response.text();
          console.error('[id] Create file failed:', errorText);
          return;
        }

        const result = await response.json() as { hash: string; name: string };
        console.log('[id] File created:', result);

        // Clear input
        input.value = '';

        // Navigate to the new file's editor
        const editUrl = `/edit/${result.hash}`;
        if (window.htmx) {
          window.htmx.ajax('GET', editUrl, { target: '#main', swap: 'innerHTML' });
          window.history.pushState(null, '', editUrl);
        } else {
          window.location.href = editUrl;
        }
      } catch (err) {
        console.error('[id] Create file error:', err);
      }
    },

    async downloadFile(format: string): Promise<void> {
      if (!this.collab?.editor) {
        console.warn('[id] No editor for download');
        return;
      }

      const editorContainer = document.getElementById('editor-container');
      if (!editorContainer) return;

      const filenameEncoded = editorContainer.dataset.filename;
      const filename = filenameEncoded ? decodeURIComponent(filenameEncoded) : 'download';

      // Get current editor state
      const state = getEditorState(this.collab.editor.view);

      try {
        const response = await fetch('/api/download', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            doc: state.doc,
            name: filename,
            format,
          }),
        });

        if (!response.ok) {
          console.error('[id] Download failed:', await response.text());
          return;
        }

        // Get filename from Content-Disposition header or use default
        const disposition = response.headers.get('Content-Disposition');
        let dlFilename = filename;
        if (disposition) {
          const match = disposition.match(/filename="?([^"]+)"?/);
          if (match) dlFilename = decodeURIComponent(match[1]);
        }

        // Create blob and trigger download
        const blob = await response.blob();
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = dlFilename;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      } catch (err) {
        console.error('[id] Download error:', err);
      }
    },

    async renameFile(): Promise<void> {
      const editorContainer = document.getElementById('editor-container');
      if (!editorContainer) return;

      const filenameEncoded = editorContainer.dataset.filename;
      const currentName = filenameEncoded ? decodeURIComponent(filenameEncoded) : null;
      if (!currentName) {
        console.error('[id] No filename for rename');
        return;
      }

      const newName = prompt(`Rename "${currentName}" to:`, currentName);
      if (!newName || newName.trim() === '' || newName.trim() === currentName) return;

      const trimmedName = newName.trim();
      const archive = confirm('Archive the original name as a backup?');

      const renameBtn = document.getElementById('rename-btn') as HTMLButtonElement | null;

      try {
        if (renameBtn) {
          renameBtn.disabled = true;
          renameBtn.textContent = 'renaming...';
        }

        const response = await fetch('/api/rename', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            name: currentName,
            new_name: trimmedName,
            archive,
          }),
        });

        if (!response.ok) {
          const errorText = await response.text();
          console.error('[id] Rename failed:', errorText);
          if (renameBtn) renameBtn.textContent = 'error!';
          setTimeout(() => { if (renameBtn) renameBtn.textContent = 'rename'; }, 2000);
          return;
        }

        const result = await response.json() as {
          name: string;
          hash: string;
          archived_original: string | null;
          archived_replaced: string | null;
        };
        console.log('[id] File renamed:', result);

        if (renameBtn) {
          renameBtn.textContent = 'renamed!';
        }

        // Navigate to the new file name
        const fileUrl = `/file/${encodeURIComponent(result.name)}`;
        if (window.htmx) {
          window.htmx.ajax('GET', fileUrl, { target: '#main', swap: 'innerHTML' });
          window.history.pushState(null, '', fileUrl);
        } else {
          window.location.href = fileUrl;
        }
      } catch (err) {
        console.error('[id] Rename error:', err);
        if (renameBtn) {
          renameBtn.textContent = 'error!';
          setTimeout(() => { if (renameBtn) renameBtn.textContent = 'rename'; }, 2000);
        }
      }
    },
  };
  
  window.idApp = app;
  
  // Event delegation for theme buttons (handles both header and settings page buttons)
  document.body.addEventListener('click', (event: MouseEvent) => {
    const target = event.target as HTMLElement;
    // Handle theme buttons with data-theme attribute
    const themeBtn = target.closest('[data-theme]');
    if (themeBtn && themeBtn.classList.contains('theme-btn')) {
      const theme = themeBtn.getAttribute('data-theme');
      if (theme === 'sneak' || theme === 'arch' || theme === 'mech') {
        setTheme(theme);
      }
    }

    // Handle download format buttons
    const dlBtn = target.closest('[data-dl-format]');
    if (dlBtn) {
      const format = dlBtn.getAttribute('data-dl-format');
      if (format) {
        app.downloadFile(format);
      }
    }

    // Toggle download dropdown
    const downloadBtn = target.closest('#download-btn');
    if (downloadBtn) {
      const menu = document.getElementById('download-menu');
      if (menu) {
        menu.classList.toggle('show');
      }
    } else {
      // Close dropdown when clicking outside
      const dropdown = target.closest('#download-dropdown');
      if (!dropdown) {
        const menu = document.getElementById('download-menu');
        if (menu) menu.classList.remove('show');
      }
    }
  });

  // Ctrl+S to save, Enter to submit tags
  document.addEventListener('keydown', (event: KeyboardEvent) => {
    if ((event.ctrlKey || event.metaKey) && event.key === 's') {
      event.preventDefault();
      if (app.collab?.editor) {
        app.saveFile();
      }
      return;
    }

    // Enter key on tag inputs submits the tag
    if (event.key === 'Enter') {
      const target = event.target as HTMLElement;
      if (target.id === 'tag-add-key' || target.id === 'tag-add-value') {
        event.preventDefault();
        app.addTagInline();
      } else if (target.id === 'bulk-tag-key' || target.id === 'bulk-tag-value') {
        event.preventDefault();
        app.bulkAddTag();
      }
    }
  });
  
  // Listen for HTMX events to handle editor initialization
  document.body.addEventListener('htmx:afterSwap', (event: Event) => {
    const detail = (event as CustomEvent).detail;
    const target = detail?.target;
    console.log('[id] htmx:afterSwap fired, target:', target?.id, 'detail:', detail);
    // After swap into #main, check if editor-container exists
    if (target?.id === 'main') {
      const newPath = window.location.pathname;
      
      // Track navigation: push previous path to history
      if (app.currentPath && app.currentPath !== newPath) {
        app.navHistory.push(app.currentPath);
        // Limit history size
        if (app.navHistory.length > 50) {
          app.navHistory.shift();
        }
      }
      app.currentPath = newPath;
      console.log('[id] Navigation: path=', newPath, 'history=', app.navHistory);
      
      const editorContainer = document.getElementById('editor-container');
      const docId = editorContainer?.dataset.docId;
      console.log('[id] afterSwap: editorContainer=', editorContainer, 'docId=', docId, 'app.collab=', app.collab);
      
      // Clean up previous scroll handler
      if (scrollCleanup) {
        scrollCleanup();
        scrollCleanup = null;
      }
      
      if (docId && !app.collab) {
        console.log('[id] afterSwap: calling openEditor for docId:', docId);
        app.openEditor(docId);
      } else {
        console.log('[id] afterSwap: NOT calling openEditor - docId:', docId, 'app.collab:', app.collab);
        // Initialize scroll handler for main page
        scrollCleanup = initScrollShowHeader('.inline-header', '.inline-footer');
        // Update back button on main page
        updateBackLink(app.navHistory, app.currentPath);
        // Update header subtitle (show last filename if we have history)
        updateHeaderSubtitle(app.lastFilename, app.lastFilePath, app.navHistory.length > 0);
        // Re-initialize file filter after swap to file list
        initFileFilter();
        // Re-initialize bulk select checkboxes
        app.initBulkSelect();
      }
    }
    
    // Re-apply show-auto filter after file-list-content swaps (e.g. tag WS events, search, pagination)
    if (target?.id === 'file-list-content') {
      initFileFilter();
    }
  });
  
  // Also listen for htmx:beforeSwap to see what's happening
  document.body.addEventListener('htmx:beforeSwap', (event: Event) => {
    const detail = (event as CustomEvent).detail;
    console.log('[id] htmx:beforeSwap fired, target:', detail?.target?.id, 'xhr status:', detail?.xhr?.status);
  });
  
  // Handle navigation away from editor
  document.body.addEventListener('htmx:beforeRequest', () => {
    if (app.collab) {
      app.closeEditor();
    }
  });
  
  console.log('[id] Web interface initialized');
  
  // Connect tags WebSocket for live updates (global — stays connected across pages)
  app.connectTagsWs();
  
  // Initialize back button on main page
  updateBackLink(app.navHistory, app.currentPath);
  
  // Initialize scroll-show header for main page
  const mainHeader = document.getElementById('main-header');
  if (mainHeader) {
    scrollCleanup = initScrollShowHeader('.inline-header', '.inline-footer');
  }
  
  // Initialize file filter on main page (if file list is present)
  initFileFilter();
  // Initialize bulk select checkboxes on main page
  app.initBulkSelect();
  
  // Check if we're on an editor page (direct navigation)
  const editorContainer = document.getElementById('editor-container');
  const docId = editorContainer?.dataset.docId;
  if (docId) {
    console.log('[id] Found editor container, initializing for doc:', docId);
    app.openEditor(docId);
  }
}

// Initialize when DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', init);
} else {
  init();
}

export { init };
