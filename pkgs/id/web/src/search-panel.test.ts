/**
 * Tests for the Find/Replace search panel plugin.
 * Tests plugin creation, panel lifecycle, and destroySearchPanel safety.
 */

import { getSearchState } from "prosemirror-search";
import { EditorState } from "prosemirror-state";
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
  it("returns an array of 2 plugins", () => {
    const plugins = createSearchPlugins();
    expect(Array.isArray(plugins)).toBe(true);
    expect(plugins.length).toBe(2);
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
