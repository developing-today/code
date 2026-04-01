/**
 * Find/Replace panel for the ProseMirror editor.
 *
 * Provides a custom search UI using prosemirror-search under the hood.
 * Keybindings:
 * - Ctrl+F / Cmd+F: open find panel
 * - Ctrl+H / Cmd+H: open find+replace panel
 * - Escape: close panel
 * - Enter / F3: find next
 * - Shift+Enter / Shift+F3: find previous
 */

import { keymap } from "prosemirror-keymap";
import {
  findNext,
  findPrev,
  getSearchState,
  replaceAll,
  replaceNext,
  SearchQuery,
  search,
  setSearchState,
} from "prosemirror-search";
import type { Command, Plugin } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";

/** Options for creating the search panel plugins. */
export interface SearchPanelOptions {
  /** Container element where the search panel will be rendered. */
  container: HTMLElement;
}

/** State tracking for the search panel DOM. */
interface PanelState {
  panelEl: HTMLElement | null;
  searchInput: HTMLInputElement | null;
  replaceInput: HTMLInputElement | null;
  matchCountEl: HTMLElement | null;
  replaceRow: HTMLElement | null;
  view: EditorView | null;
}

const panelState: PanelState = {
  panelEl: null,
  searchInput: null,
  replaceInput: null,
  matchCountEl: null,
  replaceRow: null,
  view: null,
};

/** Count total matches for display. */
function countMatches(view: EditorView): number {
  const ss = getSearchState(view.state);
  if (!ss || !ss.query.valid) return 0;

  let count = 0;
  let result = ss.query.findNext(view.state, 0);
  const seen = new Set<number>();
  while (result && !seen.has(result.from)) {
    seen.add(result.from);
    count++;
    result = ss.query.findNext(view.state, result.to);
  }
  return count;
}

/** Update match count label. */
function updateMatchCount(view: EditorView): void {
  if (!panelState.matchCountEl) return;
  const ss = getSearchState(view.state);
  if (!ss || !ss.query.valid || !ss.query.search) {
    panelState.matchCountEl.textContent = "";
    return;
  }
  const total = countMatches(view);
  panelState.matchCountEl.textContent = total === 0 ? "No results" : `${total} match${total === 1 ? "" : "es"}`;
}

/** Push current input values into prosemirror-search state. */
function syncQuery(view: EditorView): void {
  const searchText = panelState.searchInput?.value ?? "";
  const replaceText = panelState.replaceInput?.value ?? "";
  const caseCheckbox = panelState.panelEl?.querySelector<HTMLInputElement>("#search-case");
  const regexCheckbox = panelState.panelEl?.querySelector<HTMLInputElement>("#search-regex");

  const query = new SearchQuery({
    search: searchText,
    replace: replaceText,
    caseSensitive: caseCheckbox?.checked ?? false,
    regexp: regexCheckbox?.checked ?? false,
  });

  const tr = setSearchState(view.state.tr, query);
  view.dispatch(tr);
  updateMatchCount(view);
}

/** Create the search panel DOM. */
function createPanel(container: HTMLElement): HTMLElement {
  const panel = document.createElement("div");
  panel.className = "search-panel";
  panel.setAttribute("role", "search");

  panel.innerHTML = `
    <div class="search-panel-row">
      <input type="text" id="search-input" class="search-field" placeholder="Find…" autocomplete="off" spellcheck="false" />
      <span class="search-match-count" id="search-match-count"></span>
      <button class="search-btn" id="search-prev" title="Previous (Shift+Enter)">&#x25B2;</button>
      <button class="search-btn" id="search-next" title="Next (Enter)">&#x25BC;</button>
      <label class="search-toggle" title="Case sensitive">
        <input type="checkbox" id="search-case" /><span>Aa</span>
      </label>
      <label class="search-toggle" title="Regular expression">
        <input type="checkbox" id="search-regex" /><span>.*</span>
      </label>
      <button class="search-btn search-close" id="search-close" title="Close (Escape)">&times;</button>
    </div>
    <div class="search-panel-row search-replace-row" id="search-replace-row" style="display:none">
      <input type="text" id="replace-input" class="search-field" placeholder="Replace…" autocomplete="off" spellcheck="false" />
      <button class="search-btn" id="replace-one" title="Replace">Replace</button>
      <button class="search-btn" id="replace-all" title="Replace all">All</button>
    </div>
  `;

  container.prepend(panel);
  return panel;
}

/** Wire up the search panel event handlers. */
function setupPanelEvents(): void {
  const { panelEl, searchInput, replaceInput } = panelState;
  if (!panelEl || !searchInput) return;

  const view = panelState.view;
  if (!view) return;

  // Search input: live update on typing
  searchInput.addEventListener("input", () => syncQuery(view));

  // Enter = next, Shift+Enter = prev
  searchInput.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (e.shiftKey) {
        findPrev(view.state, view.dispatch, view);
      } else {
        findNext(view.state, view.dispatch, view);
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeSearchPanel(view);
    } else if (e.key === "F3") {
      e.preventDefault();
      if (e.shiftKey) {
        findPrev(view.state, view.dispatch, view);
      } else {
        findNext(view.state, view.dispatch, view);
      }
    }
  });

  if (replaceInput) {
    replaceInput.addEventListener("input", () => syncQuery(view));
    replaceInput.addEventListener("keydown", (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        closeSearchPanel(view);
      } else if (e.key === "Enter") {
        e.preventDefault();
        replaceNext(view.state, view.dispatch, view);
        updateMatchCount(view);
      }
    });
  }

  // Button clicks
  panelEl.querySelector("#search-prev")?.addEventListener("click", () => {
    findPrev(view.state, view.dispatch, view);
  });
  panelEl.querySelector("#search-next")?.addEventListener("click", () => {
    findNext(view.state, view.dispatch, view);
  });
  panelEl.querySelector("#search-close")?.addEventListener("click", () => {
    closeSearchPanel(view);
  });
  panelEl.querySelector("#replace-one")?.addEventListener("click", () => {
    replaceNext(view.state, view.dispatch, view);
    updateMatchCount(view);
  });
  panelEl.querySelector("#replace-all")?.addEventListener("click", () => {
    replaceAll(view.state, view.dispatch, view);
    updateMatchCount(view);
  });

  // Case/regex toggles
  panelEl.querySelector("#search-case")?.addEventListener("change", () => syncQuery(view));
  panelEl.querySelector("#search-regex")?.addEventListener("change", () => syncQuery(view));
}

/** Open find panel, optionally showing replace. */
function openPanel(view: EditorView, showReplace: boolean): void {
  const container = view.dom.parentElement;
  if (!container) return;

  if (!panelState.panelEl) {
    panelState.panelEl = createPanel(container);
    panelState.searchInput = panelState.panelEl.querySelector("#search-input");
    panelState.replaceInput = panelState.panelEl.querySelector("#replace-input");
    panelState.matchCountEl = panelState.panelEl.querySelector("#search-match-count");
    panelState.replaceRow = panelState.panelEl.querySelector("#search-replace-row");
    panelState.view = view;
    setupPanelEvents();
  }

  panelState.panelEl.style.display = "";

  // Show/hide replace row
  if (panelState.replaceRow) {
    panelState.replaceRow.style.display = showReplace ? "" : "none";
  }

  // Populate search from selection
  const { from, to } = view.state.selection;
  if (from !== to && panelState.searchInput) {
    const selectedText = view.state.doc.textBetween(from, to);
    if (selectedText.length > 0 && !selectedText.includes("\n")) {
      panelState.searchInput.value = selectedText;
      syncQuery(view);
    }
  }

  // Focus and select all
  panelState.searchInput?.focus();
  panelState.searchInput?.select();
}

/** Close the search panel and clear highlights. */
function closeSearchPanel(view: EditorView): void {
  if (panelState.panelEl) {
    panelState.panelEl.style.display = "none";
  }
  // Clear the search
  const tr = setSearchState(view.state.tr, new SearchQuery({ search: "" }));
  view.dispatch(tr);
  view.focus();
}

/** Command: open find panel. */
const openFind: Command = (state, dispatch, view) => {
  if (view) openPanel(view, false);
  return true;
};

/** Command: open find+replace panel. */
const openFindReplace: Command = (state, dispatch, view) => {
  if (view) openPanel(view, true);
  return true;
};

/**
 * Create the search/replace plugins.
 *
 * Returns the prosemirror-search plugin plus a keymap plugin for
 * opening/closing the search panel.
 */
export function createSearchPlugins(): Plugin[] {
  return [
    search(),
    keymap({
      "Mod-f": openFind,
      "Mod-h": openFindReplace,
      F3: findNext,
      "Shift-F3": findPrev,
    }),
  ];
}

/**
 * Destroy the search panel DOM.
 * Call this when the editor is being destroyed.
 */
export function destroySearchPanel(): void {
  if (panelState.panelEl) {
    panelState.panelEl.remove();
    panelState.panelEl = null;
    panelState.searchInput = null;
    panelState.replaceInput = null;
    panelState.matchCountEl = null;
    panelState.replaceRow = null;
    panelState.view = null;
  }
}
