/**
 * Main entry point for the id web interface.
 * Initializes SPA navigation, the ProseMirror editor, and theme switching.
 */

import "@starfederation/datastar";
import { type CollabConnection, initCollab } from "./collab";
import { type EditorInstance, getEditorState } from "./editor";
import { cycleTheme, initTheme, setTheme, type Theme } from "./theme";

declare global {
  interface Window {
    idApp: IdApp;
    cycleTheme: typeof cycleTheme;
  }
}

interface IdApp {
  collab: CollabConnection | null;
  tagsWs: WebSocket | null;
  tagsWsReconnectAttempts: number;
  setTheme: (theme: Theme) => void;
  openEditor: (docId: string) => Promise<void>;
  closeEditor: () => void;
  saveFile: () => Promise<void>;
  createFile: (event: Event) => Promise<void>;
  downloadFile: (format: string) => Promise<void>;
  renameFile: () => Promise<void>;
  copyFile: () => Promise<void>;
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

// =============================================================================
// Client Identity Management
// =============================================================================

const IDENTITY_TOKEN_KEY = "id_identity_token";
const IDENTITY_NAME_KEY = "id_identity_name";
const IDENTITY_CLIENT_ID_KEY = "id_identity_client_id";

/** Stored identity state from localStorage. */
interface IdentityState {
  token: string;
  clientId: string;
  name: string | null;
}

/** Read identity from localStorage (if exists). */
function getStoredIdentity(): IdentityState | null {
  const token = localStorage.getItem(IDENTITY_TOKEN_KEY);
  const clientId = localStorage.getItem(IDENTITY_CLIENT_ID_KEY);
  if (!token || !clientId) return null;
  return {
    token,
    clientId,
    name: localStorage.getItem(IDENTITY_NAME_KEY),
  };
}

/** Save identity to localStorage. */
function saveIdentity(token: string, clientId: string, name: string | null): void {
  localStorage.setItem(IDENTITY_TOKEN_KEY, token);
  localStorage.setItem(IDENTITY_CLIENT_ID_KEY, clientId);
  if (name) {
    localStorage.setItem(IDENTITY_NAME_KEY, name);
  } else {
    localStorage.removeItem(IDENTITY_NAME_KEY);
  }
}

/** Update the display name in localStorage (keeps token/clientId). */
function updateStoredName(name: string | null): void {
  if (name) {
    localStorage.setItem(IDENTITY_NAME_KEY, name);
  } else {
    localStorage.removeItem(IDENTITY_NAME_KEY);
  }
}

/**
 * Ensure we have a valid identity. Checks localStorage first,
 * validates with server, registers if needed.
 * Returns the identity state or null on failure.
 */
async function ensureIdentity(): Promise<IdentityState | null> {
  // Check localStorage for existing identity
  const stored = getStoredIdentity();
  if (stored) {
    // Validate token with server
    try {
      const resp = await fetch(`/api/identity/me?token=${encodeURIComponent(stored.token)}`);
      if (resp.ok) {
        const data = await resp.json();
        // Server returns a refreshed token on each validation, resetting the
        // 30-day expiry clock. Save it so the client stays authenticated as
        // long as they visit within every 30 days.
        if (data.token) {
          stored.token = data.token;
          saveIdentity(stored.token, stored.clientId, data.name || null);
        }
        // Update name from server (may have changed via another tab/session)
        const serverName = data.name || null;
        if (serverName !== stored.name) {
          updateStoredName(serverName);
          stored.name = serverName;
        }
        console.log("[id] Identity validated:", stored.clientId, stored.name);
        return stored;
      }
      // Token invalid — fall through to register
      console.log("[id] Stored token invalid, re-registering");
    } catch (err) {
      console.warn("[id] Identity check failed:", err);
      // Network error — use stored identity optimistically
      return stored;
    }
  }

  // Register new identity
  try {
    const resp = await fetch("/api/identity/register", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: null }),
    });
    if (!resp.ok) {
      console.error("[id] Identity registration failed:", resp.status);
      return null;
    }
    const data = await resp.json();
    const identity: IdentityState = {
      token: data.token,
      clientId: data.client_id,
      name: data.name || null,
    };
    saveIdentity(identity.token, identity.clientId, identity.name);
    console.log("[id] New identity registered:", identity.clientId);
    return identity;
  } catch (err) {
    console.error("[id] Identity registration error:", err);
    return null;
  }
}

/**
 * Update the display name on the server and in localStorage.
 * Returns the updated name or null on failure.
 */
async function updateDisplayName(token: string, name: string | null): Promise<string | null> {
  try {
    const resp = await fetch("/api/identity/name", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ token, name: name || null }),
    });
    if (!resp.ok) {
      console.error("[id] Name update failed:", resp.status);
      return null;
    }
    const data = await resp.json();
    const updatedName = data.name || null;
    updateStoredName(updatedName);
    console.log("[id] Display name updated:", updatedName);
    return updatedName;
  } catch (err) {
    console.error("[id] Name update error:", err);
    return null;
  }
}

// Global identity state (initialized in init())
let currentIdentity: IdentityState | null = null;

/**
 * Update the editor status indicator.
 */
function updateStatus(status: "connecting" | "connected" | "disconnected" | "error"): void {
  const statusEl = document.getElementById("editor-status");
  if (!statusEl) return;

  const statusText: Record<string, string> = {
    connecting: "connecting...",
    connected: "connected",
    disconnected: "disconnected",
    error: "error",
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
function initScrollShowHeader(
  headerSelector: string = ".editor-inline-header",
  footerSelector: string = ".editor-inline-footer",
): (() => void) | null {
  const header = document.querySelector(headerSelector) as HTMLElement | null;
  const footer = document.querySelector(footerSelector) as HTMLElement | null;

  if (!header) {
    return null;
  }

  const headerHeight = header.offsetHeight;
  const footerHeight = footer?.offsetHeight || 18;
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
          header.classList.remove("floating", "visible");
        } else {
          // Scrolled past header - floating behavior
          if (!header.classList.contains("floating")) {
            header.classList.add("floating");
          }
          if (isScrollingUp) {
            header.classList.add("visible");
          } else {
            header.classList.remove("visible");
          }
        }

        // === FOOTER ===
        if (footer) {
          if (atBottom) {
            // At the very bottom - in normal document flow
            footer.classList.remove("floating", "visible");
          } else if (atTop) {
            // At the very top - show footer floating (with header visible)
            if (!footer.classList.contains("floating")) {
              footer.classList.add("floating");
            }
            footer.classList.add("visible");
          } else {
            // In the middle - floating behavior
            if (!footer.classList.contains("floating")) {
              footer.classList.add("floating");
            }
            if (isScrollingUp) {
              footer.classList.add("visible");
            } else {
              footer.classList.remove("visible");
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

  if (footer) {
    if (atBottom) {
      footer.classList.remove("floating", "visible");
    } else if (atTop) {
      footer.classList.add("floating", "visible");
    } else {
      footer.classList.add("floating");
      footer.classList.remove("visible");
    }
  }

  window.addEventListener("scroll", handleScroll, { passive: true });

  // Return cleanup function
  return () => {
    window.removeEventListener("scroll", handleScroll);
    header.classList.remove("floating", "visible");
    footer?.classList.remove("floating", "visible");
  };
}

/**
 * Update header subtitle based on navigation state.
 * Shows "p2p file sharing" on initial load, or last filename as link after navigation.
 */
function updateHeaderSubtitle(lastFilename: string | null, lastFilePath: string | null, hasHistory: boolean): void {
  const subtitle = document.getElementById("header-subtitle");
  if (!subtitle) return;

  if (lastFilename && lastFilePath && hasHistory) {
    // Create a link to the last file
    subtitle.innerHTML = `// <a href="${lastFilePath}" data-nav>${lastFilename}</a>`;
  } else {
    subtitle.textContent = "// p2p file sharing";
  }
}

/**
 * Update back link based on app navigation history.
 * If there's history, use SPA navigation. Otherwise, grey out but still allow browser back.
 */
function updateBackLink(navHistory: string[], currentPath: string): void {
  const backLink = document.getElementById("back-link");
  if (!backLink) return;

  // Find previous path (not current)
  const prevPath = navHistory.length > 0 ? navHistory[navHistory.length - 1] : null;

  if (prevPath && prevPath !== currentPath) {
    // Has app history - use SPA navigation
    backLink.classList.remove("disabled");
    backLink.setAttribute("href", prevPath);
    backLink.setAttribute("data-nav", "");
    backLink.removeAttribute("onclick");
  } else {
    // No app history - grey out but use browser back as fallback
    backLink.classList.add("disabled");
    backLink.setAttribute("href", "#");
    backLink.removeAttribute("data-nav");
    backLink.setAttribute("onclick", "history.back(); return false;");
  }
}

/**
 * Initialize file filter: search input and show-auto checkbox.
 * Filters .file-item elements based on data-name and data-kind attributes.
 */
function initFileFilter(): void {
  const showAutoCheckbox = document.getElementById("show-auto") as HTMLInputElement | null;

  if (!showAutoCheckbox) return;

  const applyFilter = (): void => {
    const showAuto = showAutoCheckbox?.checked || false;
    const items = document.querySelectorAll(".file-item[data-kind]");

    items.forEach((el) => {
      const item = el as HTMLElement;
      const kind = item.getAttribute("data-kind") || "";

      // Hide auto/archive unless checkbox is checked
      if ((kind === "auto" || kind === "archive") && !showAuto) {
        item.style.display = "none";
        return;
      }

      item.style.display = "";
    });
  };

  // Guard against duplicate listeners (element persists across SPA navigation)
  if (!showAutoCheckbox.dataset.filterInit) {
    showAutoCheckbox.addEventListener("change", applyFilter);
    showAutoCheckbox.dataset.filterInit = "1";
  }

  // Apply filter immediately (auto files hidden by default)
  applyFilter();
}

// Track cleanup function for scroll handler
let scrollCleanup: (() => void) | null = null;

// Track peers auto-refresh interval
let peersRefreshInterval: ReturnType<typeof setInterval> | null = null;

// Track search debounce timer
let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * Fetch a URL and replace the innerHTML of a target element.
 * Used for partial page updates (search, pagination, tags WS refresh, peers auto-refresh).
 */
async function fetchPartial(url: string, targetSelector: string): Promise<void> {
  try {
    const response = await fetch(url, {
      headers: { "X-Partial-Request": "true" },
    });
    if (!response.ok) {
      console.error("[id] fetchPartial failed:", response.status, response.statusText);
      return;
    }
    const html = await response.text();
    const target = document.querySelector(targetSelector);
    if (target) {
      target.innerHTML = html;
      onPartialSwapped(targetSelector);
    }
  } catch (err) {
    console.error("[id] fetchPartial error:", err);
  }
}

/**
 * Navigate to a URL by fetching it as a partial and swapping #main content.
 * Used by [data-nav] link clicks, programmatic navigation, and popstate.
 */
async function navigateTo(url: string, pushUrl: boolean = true): Promise<void> {
  // Close editor before navigation
  const app = (window as unknown as Record<string, IdApp>).idApp;
  if (app?.collab) {
    app.closeEditor();
  }

  try {
    const response = await fetch(url, {
      headers: { "X-Partial-Request": "true" },
    });
    if (!response.ok) {
      console.error("[id] navigateTo failed:", response.status, response.statusText);
      // Fallback to full page navigation
      window.location.href = url;
      return;
    }
    const html = await response.text();
    const main = document.getElementById("main");
    if (main) {
      main.innerHTML = html;
      if (pushUrl) {
        window.history.pushState(null, "", url);
      }
      onMainSwapped();
    } else {
      // No #main element, fallback
      window.location.href = url;
    }
  } catch (err) {
    console.error("[id] navigateTo error:", err);
    window.location.href = url;
  }
}

/**
 * Called after #main content is swapped. Handles re-initialization of UI components
 * including scroll behavior, navigation state, editor, filters, and auto-refresh.
 */
function onMainSwapped(): void {
  const app = (window as unknown as Record<string, IdApp>).idApp;
  if (!app) return;

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

  const editorContainer = document.getElementById("editor-container");
  const docId = editorContainer?.dataset.docId;

  // Clean up previous scroll handler
  if (scrollCleanup) {
    scrollCleanup();
    scrollCleanup = null;
  }

  if (docId && !app.collab) {
    app.openEditor(docId);
  } else {
    // Initialize scroll handler for main page
    scrollCleanup = initScrollShowHeader(".inline-header", ".inline-footer");
    // Update back button on main page
    updateBackLink(app.navHistory, app.currentPath);
    // Update header subtitle (show last filename if we have history)
    updateHeaderSubtitle(app.lastFilename, app.lastFilePath, app.navHistory.length > 0);
    // Re-initialize file filter after swap to file list
    initFileFilter();
    // Re-initialize bulk select checkboxes
    app.initBulkSelect();
    // Re-initialize search debounce (new DOM elements)
    initSearchDebounce();
    // Re-initialize peers auto-refresh if on peers page
    initPeersAutoRefresh();
    // Populate display name input on settings page
    initSettingsIdentity();
  }
}

/**
 * Called after a partial content swap (e.g. #file-list-content).
 * Re-initializes filters and other UI state for the swapped content.
 */
function onPartialSwapped(_targetSelector: string): void {
  // Re-apply show-auto filter after file-list-content swaps
  initFileFilter();
}

/**
 * Initialize search debounce for file search input and show-deleted checkbox.
 * Search triggers after 300ms of inactivity; checkbox triggers immediately.
 */
function initSearchDebounce(): void {
  const searchInput = document.getElementById("file-search") as HTMLInputElement | null;
  const showDeletedCheckbox = document.getElementById("show-deleted") as HTMLInputElement | null;

  if (!searchInput && !showDeletedCheckbox) return;

  const doSearch = () => {
    if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
    searchDebounceTimer = setTimeout(() => {
      const query = searchInput?.value || "";
      const showDeleted = showDeletedCheckbox?.checked || false;
      const params = new URLSearchParams();
      if (query) params.set("search", query);
      if (showDeleted) params.set("show_deleted", "true");
      const qs = params.toString();
      const url = qs ? `/api/files?${qs}` : "/api/files";
      fetchPartial(url, "#file-list-content");
    }, 300);
  };

  if (searchInput) {
    searchInput.addEventListener("keyup", doSearch);
    searchInput.addEventListener("search", doSearch); // For clearing via X button
  }
  if (showDeletedCheckbox) {
    showDeletedCheckbox.addEventListener("change", () => {
      // Immediate on checkbox change (no debounce)
      if (searchDebounceTimer) clearTimeout(searchDebounceTimer);
      const query = searchInput?.value || "";
      const showDeleted = showDeletedCheckbox?.checked || false;
      const params = new URLSearchParams();
      if (query) params.set("search", query);
      if (showDeleted) params.set("show_deleted", "true");
      const qs = params.toString();
      const url = qs ? `/api/files?${qs}` : "/api/files";
      fetchPartial(url, "#file-list-content");
    });
  }
}

/**
 * Initialize peers auto-refresh via setInterval.
 * Reads interval from the data-auto-refresh attribute (in seconds).
 */
function initPeersAutoRefresh(): void {
  // Clear any existing interval
  if (peersRefreshInterval) {
    clearInterval(peersRefreshInterval);
    peersRefreshInterval = null;
  }

  const peersContent = document.querySelector("[data-auto-refresh]");
  if (!peersContent) return;

  const interval = parseInt(peersContent.getAttribute("data-auto-refresh") || "10", 10) * 1000;
  peersRefreshInterval = setInterval(() => {
    fetchPartial("/api/peers", "#peers-content");
  }, interval);
}

/**
 * Populate the display name input on the settings page (if present).
 * Reads the current name from the identity state and fills the input.
 */
function initSettingsIdentity(): void {
  const input = document.getElementById("display-name-input") as HTMLInputElement | null;
  if (!input) return; // Not on settings page

  // Populate with current display name
  if (currentIdentity?.name) {
    input.value = currentIdentity.name;
  }

  // Also handle Enter key to save
  input.addEventListener("keydown", (event: KeyboardEvent) => {
    if (event.key === "Enter") {
      event.preventDefault();
      const saveBtn = document.getElementById("display-name-save") as HTMLButtonElement | null;
      if (saveBtn) saveBtn.click();
    }
  });
}

/**
 * Initialize the application.
 */
async function init(): Promise<void> {
  // Expose cycleTheme globally for onclick handlers
  window.cycleTheme = cycleTheme;

  // Initialize theme system
  initTheme();

  // Initialize client identity (register if needed, validate if stored)
  currentIdentity = await ensureIdentity();

  // Create app API
  const app: IdApp = {
    collab: null,
    tagsWs: null,
    tagsWsReconnectAttempts: 0,
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

      const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      const wsUrl = `${wsProtocol}//${window.location.host}/ws/tags`;
      console.log("[id] Tags WS connecting:", wsUrl);
      const ws = new WebSocket(wsUrl);
      this.tagsWs = ws;

      ws.onopen = () => {
        console.log("[id] Tags WS connected");
        this.tagsWsReconnectAttempts = 0;
      };

      ws.onmessage = (event: MessageEvent) => {
        try {
          const data = JSON.parse(event.data as string);
          console.log("[id] Tag event:", data);

          // On any tag change, refresh the file list if we're on the home page
          const fileListContent = document.getElementById("file-list-content");
          if (fileListContent) {
            // Debounce: don't refresh more than once per 500ms
            const now = Date.now();
            const lastRefresh = (window as unknown as Record<string, number>).__tagRefreshTs || 0;
            if (now - lastRefresh > 500) {
              (window as unknown as Record<string, number>).__tagRefreshTs = now;
              const searchInput = document.getElementById("file-search") as HTMLInputElement | null;
              const showDeletedCheckbox = document.getElementById("show-deleted") as HTMLInputElement | null;
              const query = searchInput?.value || "";
              const showDeleted = showDeletedCheckbox?.checked || false;
              const params = new URLSearchParams();
              if (query) params.set("search", query);
              if (showDeleted) params.set("show_deleted", "true");
              const qs = params.toString();
              const url = qs ? `/api/files?${qs}` : "/api/files";
              fetchPartial(url, "#file-list-content");
            }
          }

          // On editor page, refresh tags for the current file
          const editorContainer = document.getElementById("editor-container");
          if (editorContainer && data.subject) {
            const filenameEncoded = editorContainer.dataset.filename;
            const filename = filenameEncoded ? decodeURIComponent(filenameEncoded) : null;
            if (filename && data.subject === filename) {
              console.log("[id] Tag changed for current file:", data.key, "=", data.value);
              this.loadFileTags(filename);
            }
          }
        } catch {
          // Ignore non-JSON messages
        }
      };

      ws.onclose = () => {
        this.tagsWs = null;
        const delay = Math.min(3000 * 2 ** this.tagsWsReconnectAttempts, 30000);
        const jitter = delay * 0.2 * Math.random();
        this.tagsWsReconnectAttempts++;
        console.log(
          `[id] Tags WS disconnected, reconnecting in ${Math.round((delay + jitter) / 1000)}s (attempt ${this.tagsWsReconnectAttempts})`,
        );
        setTimeout(() => this.connectTagsWs(), delay + jitter);
      };

      ws.onerror = (err) => {
        console.warn("[id] Tags WS error:", err);
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
        const tags: Array<{ key: string; value: string | null }> = Array.isArray(raw) ? raw : raw.tags || [];
        // Filter out system tags (created, modified, deleted, archive.*)
        const userTags = tags.filter(
          (t: { key: string }) =>
            t.key !== "created" && t.key !== "modified" && t.key !== "deleted" && !t.key.startsWith("archive."),
        );
        this.renderEditorTags(userTags);
      } catch (err) {
        console.warn("[id] Failed to load file tags:", err);
      }
    },

    /**
     * Render tag pills in the editor tag panel.
     */
    renderEditorTags(tags: Array<{ key: string; value: string | null }>): void {
      const list = document.getElementById("editor-tag-list");
      if (!list) return;

      const panel = document.getElementById("editor-tag-panel");
      const filename = panel?.dataset.filename ? decodeURIComponent(panel.dataset.filename) : null;

      list.innerHTML = "";
      for (const tag of tags) {
        const pill = document.createElement("span");
        pill.className = "tag-pill-removable";
        const label = tag.value ? `${tag.key}: ${tag.value}` : tag.key;
        pill.textContent = label;

        // Add remove button
        const removeBtn = document.createElement("button");
        removeBtn.className = "tag-remove-btn";
        removeBtn.textContent = "\u00d7";
        removeBtn.title = "Remove tag";
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
        const empty = document.createElement("span");
        empty.className = "text-muted";
        empty.textContent = "none";
        empty.style.fontSize = "9px";
        list.appendChild(empty);
      }
    },

    /**
     * Add a tag from the inline inputs on the editor page.
     */
    async addTagInline(): Promise<void> {
      const keyInput = document.getElementById("tag-add-key") as HTMLInputElement | null;
      const valueInput = document.getElementById("tag-add-value") as HTMLInputElement | null;
      const panel = document.getElementById("editor-tag-panel");
      const filename = panel?.dataset.filename ? decodeURIComponent(panel.dataset.filename) : null;

      if (!keyInput || !filename) return;
      const key = keyInput.value.trim();
      if (!key) return;
      const value = valueInput?.value.trim() || null;

      try {
        const body: Record<string, string> = { subject: filename, key };
        if (value) body.value = value;

        const response = await fetch("/api/tags", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        });

        if (response.ok) {
          keyInput.value = "";
          if (valueInput) valueInput.value = "";
          // Tags WS will trigger a refresh, but also reload immediately
          this.loadFileTags(filename);
        } else {
          console.error("[id] Failed to add tag:", await response.text());
        }
      } catch (err) {
        console.error("[id] Add tag error:", err);
      }
    },

    /**
     * Remove a tag via the REST API.
     */
    async removeTag(subject: string, key: string, value: string | null): Promise<void> {
      try {
        const body: Record<string, string | null> = { subject, key };
        if (value !== null) body.value = value;

        const response = await fetch("/api/tags", {
          method: "DELETE",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body),
        });

        if (response.ok) {
          // Reload tags
          this.loadFileTags(subject);
        } else {
          console.error("[id] Failed to remove tag:", await response.text());
        }
      } catch (err) {
        console.error("[id] Remove tag error:", err);
      }
    },

    /**
     * Bulk add a tag to all selected files on the home page.
     */
    async bulkAddTag(): Promise<void> {
      const keyInput = document.getElementById("bulk-tag-key") as HTMLInputElement | null;
      const valueInput = document.getElementById("bulk-tag-value") as HTMLInputElement | null;
      if (!keyInput) return;

      const key = keyInput.value.trim();
      if (!key) return;
      const value = valueInput?.value.trim() || null;

      const checkboxes = document.querySelectorAll(".file-select:checked") as NodeListOf<HTMLInputElement>;
      const subjects: string[] = [];
      checkboxes.forEach((cb) => {
        const name = cb.dataset.name;
        if (name) subjects.push(name);
      });

      if (subjects.length === 0) return;

      let successCount = 0;
      for (const subject of subjects) {
        try {
          const body: Record<string, string> = { subject, key };
          if (value) body.value = value;

          const response = await fetch("/api/tags", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(body),
          });
          if (response.ok) successCount++;
        } catch {
          // continue with others
        }
      }

      console.log(`[id] Bulk tag: added "${key}" to ${successCount}/${subjects.length} files`);
      keyInput.value = "";
      if (valueInput) valueInput.value = "";
      // Tags WS will refresh the file list
    },

    /**
     * Clear all file selection checkboxes on the home page.
     */
    bulkClearSelection(): void {
      const checkboxes = document.querySelectorAll(".file-select:checked") as NodeListOf<HTMLInputElement>;
      checkboxes.forEach((cb) => {
        cb.checked = false;
      });
      const bar = document.getElementById("bulk-action-bar");
      if (bar) bar.style.display = "none";
    },

    /**
     * Initialize bulk select checkbox listeners on file list items.
     */
    initBulkSelect(): void {
      // Use event delegation on the file list content
      const container = document.getElementById("file-list-content");
      if (!container) return;

      container.addEventListener("change", (event: Event) => {
        const target = event.target as HTMLInputElement;
        if (!target.classList.contains("file-select")) return;

        const checked = document.querySelectorAll(".file-select:checked");
        const bar = document.getElementById("bulk-action-bar");
        const countEl = document.getElementById("bulk-count");

        if (checked.length > 0) {
          if (bar) bar.style.display = "flex";
          if (countEl) countEl.textContent = `${checked.length} selected`;
        } else {
          if (bar) bar.style.display = "none";
        }
      });
    },

    async openEditor(docId: string): Promise<void> {
      // Guard against double initialization
      if (this.collab) {
        return;
      }

      const editorContainer = document.getElementById("editor-container");
      const container = document.getElementById("editor");
      if (!container || !editorContainer) {
        console.error("[id] Editor container not found");
        return;
      }

      try {
        // Get filename from data attribute (URL-encoded by server)
        const filenameEncoded = editorContainer.dataset.filename;
        const filename = filenameEncoded ? decodeURIComponent(filenameEncoded) : undefined;
        console.log("[id] Filename:", filename);

        // Track the filename and path for header subtitle
        if (filename) {
          this.lastFilename = filename;
          this.lastFilePath = this.currentPath;
        }

        // Clear container - server doc comes via WebSocket Init message
        container.innerHTML = "";

        // Connect to collab server - editor will be initialized after receiving server doc
        updateStatus("connecting");
        const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
        const wsUrl = `${wsProtocol}//${window.location.host}/ws/collab/${docId}`;
        console.log("[id] Connecting to WebSocket:", wsUrl);

        this.collab = initCollab(
          wsUrl,
          container,
          docId,
          filename,
          currentIdentity?.token ?? null,
          updateStatus,
          (editor: EditorInstance) => {
            console.log("[id] Editor initialized with server version, mode:", editor.mode);
            // Initialize scroll-show header after editor is ready
            scrollCleanup = initScrollShowHeader();
            // Update back link based on navigation history
            updateBackLink(this.navHistory, this.currentPath);
            // Enable save button
            const saveBtn = document.getElementById("save-btn") as HTMLButtonElement | null;
            if (saveBtn) saveBtn.disabled = false;
            // Load tags for the current file
            if (filename) {
              this.loadFileTags(filename);
            }
          },
        );
        console.log("[id] Collab connection initiated");
      } catch (err) {
        console.error("[id] Error initializing editor:", err);
        updateStatus("error");
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
      updateStatus("disconnected");
    },

    async saveFile(): Promise<void> {
      if (!this.collab?.editor) {
        console.warn("[id] No editor to save");
        return;
      }

      const editorContainer = document.getElementById("editor-container");
      if (!editorContainer) return;

      const docId = editorContainer.dataset.docId;
      const filenameEncoded = editorContainer.dataset.filename;
      const filename = filenameEncoded ? decodeURIComponent(filenameEncoded) : null;

      if (!docId || !filename) {
        console.error("[id] Missing doc_id or filename for save");
        return;
      }

      // Get current editor state
      const state = getEditorState(this.collab.editor.view);
      const saveBtn = document.getElementById("save-btn") as HTMLButtonElement | null;

      try {
        if (saveBtn) {
          saveBtn.disabled = true;
          saveBtn.textContent = "saving...";
        }

        const response = await fetch("/api/save", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            doc_id: docId,
            name: filename,
            doc: state.doc,
          }),
        });

        if (!response.ok) {
          const errorText = await response.text();
          console.error("[id] Save failed:", errorText);
          if (saveBtn) saveBtn.textContent = "error!";
          setTimeout(() => {
            if (saveBtn) saveBtn.textContent = "save";
          }, 2000);
          return;
        }

        const result = (await response.json()) as { hash: string; name: string; archive_name: string | null };
        console.log("[id] File saved:", result);

        // Update the doc_id in the container to the new hash
        editorContainer.dataset.docId = result.hash;

        // Update the URL to reflect the new hash
        const newUrl = `/edit/${result.hash}`;
        window.history.replaceState(null, "", newUrl);

        if (saveBtn) {
          saveBtn.textContent = "saved!";
          setTimeout(() => {
            if (saveBtn) saveBtn.textContent = "save";
          }, 2000);
        }
      } catch (err) {
        console.error("[id] Save error:", err);
        if (saveBtn) {
          saveBtn.textContent = "error!";
          setTimeout(() => {
            if (saveBtn) saveBtn.textContent = "save";
          }, 2000);
        }
      }
    },

    async createFile(event: Event): Promise<void> {
      event.preventDefault();
      const input = document.getElementById("new-file-name") as HTMLInputElement | null;
      if (!input) return;

      const name = input.value.trim();
      if (!name) return;

      try {
        const response = await fetch("/api/new", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name }),
        });

        if (!response.ok) {
          const errorText = await response.text();
          console.error("[id] Create file failed:", errorText);
          return;
        }

        const result = (await response.json()) as { hash: string; name: string };
        console.log("[id] File created:", result);

        // Clear input
        input.value = "";

        // Navigate to the new file's editor
        const editUrl = `/edit/${result.hash}`;
        await navigateTo(editUrl);
      } catch (err) {
        console.error("[id] Create file error:", err);
      }
    },

    async downloadFile(format: string): Promise<void> {
      if (!this.collab?.editor) {
        console.warn("[id] No editor for download");
        return;
      }

      const editorContainer = document.getElementById("editor-container");
      if (!editorContainer) return;

      const filenameEncoded = editorContainer.dataset.filename;
      const filename = filenameEncoded ? decodeURIComponent(filenameEncoded) : "download";

      // Get current editor state
      const state = getEditorState(this.collab.editor.view);

      try {
        const response = await fetch("/api/download", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            doc: state.doc,
            name: filename,
            format,
          }),
        });

        if (!response.ok) {
          console.error("[id] Download failed:", await response.text());
          return;
        }

        // Get filename from Content-Disposition header or use default
        const disposition = response.headers.get("Content-Disposition");
        let dlFilename = filename;
        if (disposition) {
          const match = disposition.match(/filename="?([^"]+)"?/);
          if (match) dlFilename = decodeURIComponent(match[1]);
        }

        // Create blob and trigger download
        const blob = await response.blob();
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = dlFilename;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      } catch (err) {
        console.error("[id] Download error:", err);
      }
    },

    async renameFile(): Promise<void> {
      // Find filename from editor page or viewer page
      const editorContainer = document.getElementById("editor-container");
      const viewerActions = document.querySelector(".viewer-actions") as HTMLElement | null;
      const filenameEncoded = editorContainer?.dataset.filename ?? viewerActions?.dataset.filename;
      const currentName = filenameEncoded ? decodeURIComponent(filenameEncoded) : null;
      if (!currentName) {
        console.error("[id] No filename for rename");
        return;
      }

      const newName = prompt(`Rename "${currentName}" to:`, currentName);
      if (!newName || newName.trim() === "" || newName.trim() === currentName) return;

      const trimmedName = newName.trim();
      const archive = confirm("Archive the original name as a backup?");

      const renameBtn = document.getElementById("rename-btn") as HTMLButtonElement | null;

      try {
        if (renameBtn) {
          renameBtn.disabled = true;
          renameBtn.textContent = "renaming...";
        }

        const response = await fetch("/api/rename", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            name: currentName,
            new_name: trimmedName,
            archive,
          }),
        });

        if (!response.ok) {
          const errorText = await response.text();
          console.error("[id] Rename failed:", errorText);
          if (renameBtn) renameBtn.textContent = "error!";
          setTimeout(() => {
            if (renameBtn) renameBtn.textContent = "rename";
          }, 2000);
          return;
        }

        const result = (await response.json()) as {
          name: string;
          hash: string;
          archived_original: string | null;
          archived_replaced: string | null;
        };
        console.log("[id] File renamed:", result);

        if (renameBtn) {
          renameBtn.textContent = "renamed!";
        }

        // Navigate to the new file name
        const fileUrl = `/file/${encodeURIComponent(result.name)}`;
        await navigateTo(fileUrl);
      } catch (err) {
        console.error("[id] Rename error:", err);
        if (renameBtn) {
          renameBtn.textContent = "error!";
          setTimeout(() => {
            if (renameBtn) renameBtn.textContent = "rename";
          }, 2000);
        }
      }
    },

    async copyFile(): Promise<void> {
      // Find filename from editor page or viewer page
      const editorContainer = document.getElementById("editor-container");
      const viewerActions = document.querySelector(".viewer-actions") as HTMLElement | null;
      const filenameEncoded = editorContainer?.dataset.filename ?? viewerActions?.dataset.filename;
      const currentName = filenameEncoded ? decodeURIComponent(filenameEncoded) : null;
      if (!currentName) {
        console.error("[id] No filename for copy");
        return;
      }

      const newName = prompt(`Copy "${currentName}" to:`, currentName);
      if (!newName || newName.trim() === "" || newName.trim() === currentName) return;

      const trimmedName = newName.trim();
      const copyBtn = document.getElementById("copy-btn") as HTMLButtonElement | null;

      try {
        if (copyBtn) {
          copyBtn.disabled = true;
          copyBtn.textContent = "copying...";
        }

        const response = await fetch("/api/copy", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            name: currentName,
            new_name: trimmedName,
          }),
        });

        if (!response.ok) {
          const errorText = await response.text();
          console.error("[id] Copy failed:", errorText);
          if (copyBtn) copyBtn.textContent = "error!";
          setTimeout(() => {
            if (copyBtn) copyBtn.textContent = "copy";
          }, 2000);
          return;
        }

        const result = (await response.json()) as {
          name: string;
          hash: string;
        };
        console.log("[id] File copied:", result);

        if (copyBtn) {
          copyBtn.textContent = "copied!";
        }

        // Navigate to the copied file
        const fileUrl = `/file/${encodeURIComponent(result.name)}`;
        await navigateTo(fileUrl);
      } catch (err) {
        console.error("[id] Copy error:", err);
        if (copyBtn) {
          copyBtn.textContent = "error!";
          setTimeout(() => {
            if (copyBtn) copyBtn.textContent = "copy";
          }, 2000);
        }
      }
    },
  };

  window.idApp = app;

  // Event delegation for theme buttons (handles both header and settings page buttons)
  document.body.addEventListener("click", (event: MouseEvent) => {
    const target = event.target as HTMLElement;
    // Handle theme buttons with data-theme attribute
    const themeBtn = target.closest("[data-theme]");
    if (themeBtn?.classList.contains("theme-btn")) {
      const theme = themeBtn.getAttribute("data-theme");
      if (theme === "sneak" || theme === "arch" || theme === "mech") {
        setTheme(theme);
      }
    }

    // Handle display name save button on settings page
    if (target.id === "display-name-save" || target.closest("#display-name-save")) {
      const input = document.getElementById("display-name-input") as HTMLInputElement | null;
      const status = document.getElementById("display-name-status");
      if (input && currentIdentity?.token) {
        const newName = input.value.trim() || null;
        const btn = document.getElementById("display-name-save") as HTMLButtonElement | null;
        if (btn) btn.disabled = true;
        if (status) status.textContent = "saving...";

        updateDisplayName(currentIdentity.token, newName).then((updatedName) => {
          if (btn) btn.disabled = false;
          if (updatedName !== null || newName === null) {
            // Success: update local state
            if (currentIdentity) currentIdentity.name = updatedName;
            if (status) {
              status.textContent = "saved!";
              setTimeout(() => {
                if (status) status.textContent = "";
              }, 2000);
            }
          } else {
            if (status) {
              status.textContent = "failed";
              setTimeout(() => {
                if (status) status.textContent = "";
              }, 2000);
            }
          }
        });
      }
    }

    // Handle download format buttons
    const dlBtn = target.closest("[data-dl-format]");
    if (dlBtn) {
      const format = dlBtn.getAttribute("data-dl-format");
      if (format) {
        app.downloadFile(format);
      }
    }

    // Toggle download dropdown
    const downloadBtn = target.closest("#download-btn");
    if (downloadBtn) {
      const menu = document.getElementById("download-menu");
      if (menu) {
        menu.classList.toggle("show");
      }
    } else {
      // Close dropdown when clicking outside
      const dropdown = target.closest("#download-dropdown");
      if (!dropdown) {
        const menu = document.getElementById("download-menu");
        if (menu) menu.classList.remove("show");
      }
    }
  });

  // Ctrl+S to save, Enter to submit tags
  document.addEventListener("keydown", (event: KeyboardEvent) => {
    if ((event.ctrlKey || event.metaKey) && event.key === "s") {
      event.preventDefault();
      if (app.collab?.editor) {
        app.saveFile();
      }
      return;
    }

    // Enter key on tag inputs submits the tag
    if (event.key === "Enter") {
      const target = event.target as HTMLElement;
      if (target.id === "tag-add-key" || target.id === "tag-add-value") {
        event.preventDefault();
        app.addTagInline();
      } else if (target.id === "bulk-tag-key" || target.id === "bulk-tag-value") {
        event.preventDefault();
        app.bulkAddTag();
      }
    }
  });

  // SPA navigation: click delegation for [data-nav] links
  document.addEventListener("click", (event: MouseEvent) => {
    const link = (event.target as Element)?.closest<HTMLAnchorElement>("a[data-nav]");
    if (link) {
      event.preventDefault();
      const url = link.getAttribute("href");
      if (url) {
        navigateTo(url, true);
      }
      return;
    }

    // Pagination links: [data-page-nav] buttons
    const pageBtn = (event.target as Element)?.closest<HTMLElement>("[data-page-nav]");
    if (pageBtn) {
      event.preventDefault();
      const url = pageBtn.getAttribute("data-page-nav");
      const target = pageBtn.getAttribute("data-target") || "#file-list-content";
      if (url) {
        fetchPartial(url, target);
      }
    }
  });

  // Browser back/forward navigation
  window.addEventListener("popstate", () => {
    const url = window.location.pathname + window.location.search;
    navigateTo(url, false);
  });

  console.log("[id] Web interface initialized");

  // Connect tags WebSocket for live updates (global — stays connected across pages)
  app.connectTagsWs();

  // Initialize back button on main page
  updateBackLink(app.navHistory, app.currentPath);

  // Initialize scroll-show header for main page
  const mainHeader = document.getElementById("main-header");
  if (mainHeader) {
    scrollCleanup = initScrollShowHeader(".inline-header", ".inline-footer");
  }

  // Initialize file filter on main page (if file list is present)
  initFileFilter();
  // Initialize bulk select checkboxes on main page
  app.initBulkSelect();
  // Initialize search debounce for file search input
  initSearchDebounce();
  // Initialize auto-refresh for peers page
  initPeersAutoRefresh();
  // Populate display name on settings page (if direct navigation)
  initSettingsIdentity();

  // Check if we're on an editor page (direct navigation)
  const editorContainer = document.getElementById("editor-container");
  const docId = editorContainer?.dataset.docId;
  if (docId) {
    console.log("[id] Found editor container, initializing for doc:", docId);
    app.openEditor(docId);
  }
}

// Initialize when DOM is ready
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", init);
} else {
  init();
}

export { init };
