/**
 * Tests for the Find/Replace search panel plugin.
 * Tests plugin creation, panel lifecycle, and destroySearchPanel safety.
 */

import { getSearchState } from "prosemirror-search";
import { EditorState, TextSelection } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { afterEach, describe, expect, it } from "vitest";
import { rawSchema } from "./editor";
import { createSearchPlugins, destroySearchPanel } from "./search-panel";

// ── Helpers ────────────────────────────────────────────────────────

/** Create an EditorState with search plugins for testing */
function createStateWithSearch(): EditorState {
  const plugins = createSearchPlugins();
  return EditorState.create({
    schema: rawSchema,
    plugins,
  });
}

/** Create an EditorView attached to a container div */
function createViewWithSearch(): { view: EditorView; container: HTMLElement } {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const state = createStateWithSearch();
  const view = new EditorView(container, { state });
  return { view, container };
}

/**
 * Trigger a keymap handler on the view's keymap plugin (plugins[1]).
 * Uses `.call(km, ...)` to fix the `this` context TypeScript complains about.
 */
function triggerSearchKey(view: EditorView, key: string, ctrlKey = true): void {
  const plugins = view.state.plugins ?? [];
  const km = plugins[1]; // keymap plugin is second
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- handleKeyDown this context workaround
  const handler = (km?.props as any)?.handleKeyDown;
  if (handler) {
    const event = new KeyboardEvent("keydown", { key, ctrlKey, bubbles: true });
    handler.call(km, view, event);
  }
}

// ── Cleanup ────────────────────────────────────────────────────────

let activeView: EditorView | null = null;
let activeContainer: HTMLElement | null = null;

afterEach(() => {
  destroySearchPanel();
  if (activeView) {
    activeView.destroy();
    activeView = null;
  }
  if (activeContainer) {
    activeContainer.remove();
    activeContainer = null;
  }
});

// ── createSearchPlugins ────────────────────────────────────────────

describe("createSearchPlugins", () => {
  it("returns an array of 3 plugins", () => {
    const plugins = createSearchPlugins();
    expect(Array.isArray(plugins)).toBe(true);
    // search + keymap + match-count sync
    expect(plugins.length).toBe(3);
  });

  it("all returned items are ProseMirror plugins", () => {
    const plugins = createSearchPlugins();
    for (const plugin of plugins) {
      // ProseMirror plugins have a spec property
      expect(plugin.spec).toBeDefined();
    }
  });

  it("first plugin is the search plugin (has getSearchState)", () => {
    const state = createStateWithSearch();
    // getSearchState should return the search state from the first plugin
    const ss = getSearchState(state);
    expect(ss).toBeDefined();
    expect(ss?.query).toBeDefined();
  });

  it("second plugin is a keymap (has handleKeyDown in props)", () => {
    const plugins = createSearchPlugins();
    const keymapPlugin = plugins[1];
    // Keymap plugins produce props with handleKeyDown
    expect(keymapPlugin.props).toBeDefined();
    expect(keymapPlugin.props.handleKeyDown).toBeDefined();
  });
});

// ── destroySearchPanel ─────────────────────────────────────────────

describe("destroySearchPanel", () => {
  it("can be called safely when no panel exists", () => {
    // Should not throw
    expect(() => destroySearchPanel()).not.toThrow();
  });

  it("can be called multiple times without error", () => {
    expect(() => {
      destroySearchPanel();
      destroySearchPanel();
      destroySearchPanel();
    }).not.toThrow();
  });

  it("removes panel DOM when panel exists", () => {
    const { view, container } = createViewWithSearch();
    activeView = view;
    activeContainer = container;

    triggerSearchKey(view, "f");

    // Panel should be created
    const panel = container.querySelector(".search-panel");
    expect(panel).not.toBeNull();

    // Destroy it
    destroySearchPanel();

    // Panel should be removed
    const panelAfter = container.querySelector(".search-panel");
    expect(panelAfter).toBeNull();
  });
});

// ── Search Plugin State ────────────────────────────────────────────

describe("search plugin state", () => {
  it("returns undefined for state without search plugin", () => {
    const state = EditorState.create({ schema: rawSchema });
    const ss = getSearchState(state);
    expect(ss).toBeUndefined();
  });

  it("returns search state for state with search plugins", () => {
    const state = createStateWithSearch();
    const ss = getSearchState(state);
    expect(ss).toBeDefined();
    expect(ss?.query).toBeDefined();
  });

  it("initial search query is empty/invalid", () => {
    const state = createStateWithSearch();
    const ss = getSearchState(state);
    expect(ss?.query.search).toBe("");
    expect(ss?.query.valid).toBe(false);
  });
});

// ── Search Panel Integration ───────────────────────────────────────

describe("search panel integration", () => {
  it("Mod-f opens a .search-panel element", () => {
    const { view, container } = createViewWithSearch();
    activeView = view;
    activeContainer = container;

    triggerSearchKey(view, "f");

    const panel = container.querySelector(".search-panel");
    expect(panel).not.toBeNull();
  });

  it("panel has a search input field", () => {
    const { view, container } = createViewWithSearch();
    activeView = view;
    activeContainer = container;

    triggerSearchKey(view, "f");

    const searchInput = container.querySelector("#search-input");
    expect(searchInput).not.toBeNull();
    expect(searchInput?.tagName.toLowerCase()).toBe("input");
  });

  it("panel has navigation buttons", () => {
    const { view, container } = createViewWithSearch();
    activeView = view;
    activeContainer = container;

    triggerSearchKey(view, "f");

    expect(container.querySelector("#search-prev")).not.toBeNull();
    expect(container.querySelector("#search-next")).not.toBeNull();
    expect(container.querySelector("#search-close")).not.toBeNull();
  });

  it("panel has match count element", () => {
    const { view, container } = createViewWithSearch();
    activeView = view;
    activeContainer = container;

    triggerSearchKey(view, "f");

    const matchCount = container.querySelector("#search-match-count");
    expect(matchCount).not.toBeNull();
  });

  it("Mod-h shows the replace row", () => {
    const { view, container } = createViewWithSearch();
    activeView = view;
    activeContainer = container;

    triggerSearchKey(view, "h");

    const replaceRow = container.querySelector("#search-replace-row") as HTMLElement | null;
    expect(replaceRow).not.toBeNull();
    // Replace row should be visible (not display:none)
    expect(replaceRow?.style.display).not.toBe("none");
  });

  it("Mod-f hides the replace row", () => {
    const { view, container } = createViewWithSearch();
    activeView = view;
    activeContainer = container;

    // Open with Mod-f (find only, no replace)
    triggerSearchKey(view, "f");

    const replaceRow = container.querySelector("#search-replace-row") as HTMLElement | null;
    expect(replaceRow).not.toBeNull();
    // Replace row should be hidden
    expect(replaceRow?.style.display).toBe("none");
  });

  it("replace row has replace input and buttons", () => {
    const { view, container } = createViewWithSearch();
    activeView = view;
    activeContainer = container;

    triggerSearchKey(view, "h");

    expect(container.querySelector("#replace-input")).not.toBeNull();
    expect(container.querySelector("#replace-one")).not.toBeNull();
    expect(container.querySelector("#replace-all")).not.toBeNull();
  });

  it("panel has case-sensitive and regex toggles", () => {
    const { view, container } = createViewWithSearch();
    activeView = view;
    activeContainer = container;

    triggerSearchKey(view, "f");

    expect(container.querySelector("#search-case")).not.toBeNull();
    expect(container.querySelector("#search-regex")).not.toBeNull();
  });

  it("panel has role=search for accessibility", () => {
    const { view, container } = createViewWithSearch();
    activeView = view;
    activeContainer = container;

    triggerSearchKey(view, "f");

    const panel = container.querySelector(".search-panel");
    expect(panel?.getAttribute("role")).toBe("search");
  });

  it("opening panel twice does not create duplicate panels", () => {
    const { view, container } = createViewWithSearch();
    activeView = view;
    activeContainer = container;

    triggerSearchKey(view, "f");
    triggerSearchKey(view, "f");

    const panels = container.querySelectorAll(".search-panel");
    expect(panels.length).toBe(1);
  });
});

// ── Match count refresh on document edits (#1402) ──────────────────

/** Create a view whose document is a single raw code_block holding `text`. */
function createViewWithDoc(text: string): { view: EditorView; container: HTMLElement } {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const doc = rawSchema.node("doc", null, [rawSchema.node("code_block", null, [rawSchema.text(text)])]);
  const state = EditorState.create({ schema: rawSchema, doc, plugins: createSearchPlugins() });
  const view = new EditorView(container, { state });
  return { view, container };
}

/** Open the find panel and type a query, as a user would. */
function openAndSearch(view: EditorView, container: HTMLElement, query: string): void {
  triggerSearchKey(view, "f");
  const input = container.querySelector("#search-input") as HTMLInputElement;
  input.value = query;
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

/** Text currently shown in the match count label. */
function matchCountText(container: HTMLElement): string | null | undefined {
  return container.querySelector("#search-match-count")?.textContent;
}

describe("match count refresh on document edits", () => {
  it("decreases the count when an edit removes a match", () => {
    const { view, container } = createViewWithDoc("foo foo foo");
    activeView = view;
    activeContainer = container;

    openAndSearch(view, container, "foo");
    expect(matchCountText(container)).toBe("3 matches");

    // Delete the second "foo " -> "foo foo"
    view.dispatch(view.state.tr.delete(5, 9));

    expect(matchCountText(container)).toBe("2 matches");
  });

  it("increases the count when an edit adds a match", () => {
    const { view, container } = createViewWithDoc("foo foo");
    activeView = view;
    activeContainer = container;

    openAndSearch(view, container, "foo");
    expect(matchCountText(container)).toBe("2 matches");

    view.dispatch(view.state.tr.insertText(" foo", 8));

    expect(matchCountText(container)).toBe("3 matches");
  });

  it("transitions through '1 match' and 'No results' as the document is edited", () => {
    const { view, container } = createViewWithDoc("foo foo");
    activeView = view;
    activeContainer = container;

    openAndSearch(view, container, "foo");
    expect(matchCountText(container)).toBe("2 matches");

    view.dispatch(view.state.tr.delete(5, 9));
    expect(matchCountText(container)).toBe("1 match");

    view.dispatch(view.state.tr.delete(1, 4));
    expect(matchCountText(container)).toBe("No results");

    view.dispatch(view.state.tr.insertText("foo", 1));
    expect(matchCountText(container)).toBe("1 match");
  });

  it("does not re-count on selection-only transactions", () => {
    const { view, container } = createViewWithDoc("foo foo foo");
    activeView = view;
    activeContainer = container;

    openAndSearch(view, container, "foo");
    expect(matchCountText(container)).toBe("3 matches");

    // Count writes to the label, so a redundant re-count is observable even
    // when it writes the same text back.
    const label = container.querySelector("#search-match-count") as HTMLElement;
    let text = label.textContent ?? "";
    let writes = 0;
    Object.defineProperty(label, "textContent", {
      configurable: true,
      get() {
        return text;
      },
      set(value: string) {
        writes += 1;
        text = value;
      },
    });

    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, 1, 4)));

    expect(writes).toBe(0);
    expect(matchCountText(container)).toBe("3 matches");
  });
});

// ── Editor recreation (#718) ───────────────────────────────────────

describe("search panel across editor recreation", () => {
  it("Ctrl+F in a recreated editor shows a panel in the new container without destroySearchPanel()", () => {
    const first = createViewWithSearch();
    triggerSearchKey(first.view, "f");
    expect(first.container.querySelector(".search-panel")).not.toBeNull();

    // Production teardown (main.ts disconnect) destroys only the view —
    // it never calls destroySearchPanel(). Mirror that exactly.
    first.view.destroy();
    first.container.remove();

    const second = createViewWithSearch();
    activeView = second.view;
    activeContainer = second.container;
    triggerSearchKey(second.view, "f");
    expect(second.container.querySelector(".search-panel")).not.toBeNull();
    expect(second.container.querySelector("#search-input")).not.toBeNull();
  });
});
