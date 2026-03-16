/**
 * Main entry point for the id web interface.
 * Initializes HTMX, the ProseMirror editor, and theme switching.
 */

import htmx from 'htmx.org';
import { type EditorInstance } from './editor';
import { initCollab, type CollabConnection } from './collab';
import { initTheme, setTheme, type Theme } from './theme';

// Expose htmx globally for HTMX attributes in HTML
declare global {
  interface Window {
    htmx: typeof htmx;
    idApp: IdApp;
  }
}

interface IdApp {
  collab: CollabConnection | null;
  setTheme: (theme: Theme) => void;
  openEditor: (docId: string) => Promise<void>;
  closeEditor: () => void;
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
 * Initialize the application.
 */
function init(): void {
  // Initialize HTMX
  window.htmx = htmx;
  
  // Configure HTMX
  htmx.config.defaultSwapStyle = 'innerHTML';
  htmx.config.historyCacheSize = 10;
  htmx.config.refreshOnHistoryMiss = true;
  
  // Initialize theme system
  initTheme();
  
  // Create app API
  const app: IdApp = {
    collab: null,
    setTheme,
    
    async openEditor(docId: string): Promise<void> {
      // Guard against double initialization
      if (this.collab) {
        console.log('[id] Editor already initialized');
        return;
      }
      
      const container = document.getElementById('editor');
      if (!container) {
        console.error('[id] Editor container not found');
        return;
      }
      
      try {
        // Get initial content from the container (server-rendered)
        const initialContent = container.innerHTML;
        console.log('[id] Initial content length:', initialContent.length);
        
        // Clear container before connecting
        container.innerHTML = '';
        
        // Connect to collab server - editor will be initialized after receiving server version
        updateStatus('connecting');
        const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${wsProtocol}//${window.location.host}/ws/collab/${docId}`;
        console.log('[id] Connecting to WebSocket:', wsUrl);
        
        this.collab = initCollab(
          wsUrl,
          container,
          initialContent,
          docId,
          updateStatus,
          (editor: EditorInstance) => {
            console.log('[id] Editor initialized with server version');
          }
        );
        console.log('[id] Collab connection initiated');
      } catch (err) {
        console.error('[id] Error initializing editor:', err);
        updateStatus('error');
      }
    },
    
    closeEditor(): void {
      if (this.collab) {
        // The collab connection owns the editor, so destroying collab cleans up both
        if (this.collab.editor) {
          this.collab.editor.view.destroy();
        }
        this.collab.disconnect();
        this.collab = null;
      }
      updateStatus('disconnected');
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
  });
  
  // Listen for HTMX events to handle editor initialization
  document.body.addEventListener('htmx:afterSwap', (event: Event) => {
    const target = (event as CustomEvent).detail?.target;
    // After swap into #main, check if editor-container exists
    if (target?.id === 'main') {
      const editorContainer = document.getElementById('editor-container');
      const docId = editorContainer?.dataset.docId;
      if (docId && !app.collab) {
        app.openEditor(docId);
      }
    }
  });
  
  // Handle navigation away from editor
  document.body.addEventListener('htmx:beforeRequest', () => {
    if (app.collab) {
      app.closeEditor();
    }
  });
  
  console.log('[id] Web interface initialized');
  
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
