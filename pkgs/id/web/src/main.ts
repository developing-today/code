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
  navHistory: string[];
  currentPath: string;
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
 * Initialize scroll-show behavior for editor inline header.
 * Header is in normal flow at top. Once scrolled past, it becomes fixed
 * and shows when scrolling up, hides when scrolling down.
 * When scrolled back to very top, it returns to normal flow.
 */
function initScrollShowHeader(): (() => void) | null {
  const header = document.querySelector('.editor-inline-header') as HTMLElement | null;
  
  if (!header) {
    console.log('[id] scroll-show: header not found');
    return null;
  }
  
  console.log('[id] scroll-show: initializing, header element:', header);
  
  // Get header height for threshold calculation
  const headerHeight = header.offsetHeight;
  let lastScrollTop = window.scrollY || document.documentElement.scrollTop;
  let ticking = false;
  
  const handleScroll = (): void => {
    const scrollTop = window.scrollY || document.documentElement.scrollTop;
    
    if (!ticking) {
      window.requestAnimationFrame(() => {
        // At the very top - header is in normal document flow
        if (scrollTop <= headerHeight) {
          header.classList.remove('floating', 'visible');
        }
        // Scrolled past header - make it floating
        else {
          if (!header.classList.contains('floating')) {
            header.classList.add('floating');
          }
          
          // Scrolling up - show floating header
          if (scrollTop < lastScrollTop) {
            header.classList.add('visible');
          }
          // Scrolling down - hide floating header
          else if (scrollTop > lastScrollTop) {
            header.classList.remove('visible');
          }
        }
        
        lastScrollTop = scrollTop;
        ticking = false;
      });
      ticking = true;
    }
  };
  
  window.addEventListener('scroll', handleScroll, { passive: true });
  console.log('[id] scroll-show: listener attached to window, current scrollY:', window.scrollY);
  
  // Return cleanup function
  return () => {
    window.removeEventListener('scroll', handleScroll);
    header.classList.remove('floating', 'visible');
    console.log('[id] scroll-show: listener removed');
  };
}

/**
 * Update back link based on app navigation history.
 */
function updateBackLink(navHistory: string[], currentPath: string): void {
  const backLink = document.getElementById('back-link');
  if (!backLink) return;
  
  // Find previous path (not current)
  const prevPath = navHistory.length > 0 ? navHistory[navHistory.length - 1] : null;
  
  if (prevPath && prevPath !== currentPath) {
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
    // No history - grey out and use browser back as fallback
    backLink.classList.add('disabled');
    backLink.removeAttribute('href');
    backLink.removeAttribute('hx-get');
    backLink.removeAttribute('hx-target');
    backLink.removeAttribute('hx-push-url');
    backLink.setAttribute('onclick', 'history.back()');
  }
}

// Track cleanup function for scroll handler
let scrollCleanup: (() => void) | null = null;

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
    navHistory: [],
    currentPath: window.location.pathname,
    
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
      if (docId && !app.collab) {
        console.log('[id] afterSwap: calling openEditor for docId:', docId);
        app.openEditor(docId);
      } else {
        console.log('[id] afterSwap: NOT calling openEditor - docId:', docId, 'app.collab:', app.collab);
        // Update back button on main page
        updateBackLink(app.navHistory, app.currentPath);
      }
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
  
  // Initialize back button on main page
  updateBackLink(app.navHistory, app.currentPath);
  
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
