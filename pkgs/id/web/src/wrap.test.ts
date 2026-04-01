/**
 * Tests for the word wrap toggle plugin.
 */

import { EditorState } from "prosemirror-state";
import { describe, expect, it } from "vitest";
import { rawSchema } from "./editor";
import {
  createWrapPlugins,
  type MeasureResult,
  measureContent,
  toggleWrap,
  type WrapState,
  wrapPluginKey,
} from "./wrap";

// ── Helper ─────────────────────────────────────────────────────────

/** Create an EditorState with wrap plugins for testing */
function createStateWithWrap(defaultEnabled = true): EditorState {
  const plugins = createWrapPlugins({ defaultEnabled });
  return EditorState.create({
    schema: rawSchema,
    plugins,
  });
}

// ── Plugin State ───────────────────────────────────────────────────

describe("createWrapPlugins", () => {
  it("returns an array of plugins", () => {
    const plugins = createWrapPlugins();
    expect(Array.isArray(plugins)).toBe(true);
    expect(plugins.length).toBe(2); // state plugin + keymap
  });

  it("all returned items are ProseMirror plugins", () => {
    const plugins = createWrapPlugins();
    for (const plugin of plugins) {
      // ProseMirror plugins have a spec property
      expect(plugin.spec).toBeDefined();
    }
  });
});

describe("wrap plugin state", () => {
  it("defaults to enabled (wrap ON)", () => {
    const state = createStateWithWrap();
    const wrapState = wrapPluginKey.getState(state);
    expect(wrapState).toBeDefined();
    expect(wrapState?.enabled).toBe(true);
  });

  it("respects defaultEnabled: false", () => {
    const state = createStateWithWrap(false);
    const wrapState = wrapPluginKey.getState(state);
    expect(wrapState).toBeDefined();
    expect(wrapState?.enabled).toBe(false);
  });

  it("respects defaultEnabled: true explicitly", () => {
    const state = createStateWithWrap(true);
    const wrapState = wrapPluginKey.getState(state);
    expect(wrapState?.enabled).toBe(true);
  });
});

// ── Toggle Command ─────────────────────────────────────────────────

describe("toggleWrap command", () => {
  it("returns true (command is always applicable)", () => {
    const state = createStateWithWrap();
    const result = toggleWrap(state, undefined);
    expect(result).toBe(true);
  });

  it("toggles from enabled to disabled", () => {
    let state = createStateWithWrap(true);

    // Dispatch the toggle
    toggleWrap(state, (tr) => {
      state = state.apply(tr);
    });

    const wrapState = wrapPluginKey.getState(state);
    expect(wrapState?.enabled).toBe(false);
  });

  it("toggles from disabled to enabled", () => {
    let state = createStateWithWrap(false);

    toggleWrap(state, (tr) => {
      state = state.apply(tr);
    });

    const wrapState = wrapPluginKey.getState(state);
    expect(wrapState?.enabled).toBe(true);
  });

  it("can toggle multiple times", () => {
    let state = createStateWithWrap(true);

    // Toggle 1: on -> off
    toggleWrap(state, (tr) => {
      state = state.apply(tr);
    });
    expect(wrapPluginKey.getState(state)?.enabled).toBe(false);

    // Toggle 2: off -> on
    toggleWrap(state, (tr) => {
      state = state.apply(tr);
    });
    expect(wrapPluginKey.getState(state)?.enabled).toBe(true);

    // Toggle 3: on -> off
    toggleWrap(state, (tr) => {
      state = state.apply(tr);
    });
    expect(wrapPluginKey.getState(state)?.enabled).toBe(false);
  });

  it("works without dispatch (dry run / applicability check)", () => {
    const state = createStateWithWrap(true);
    // No dispatch — just checking if command applies
    const result = toggleWrap(state, undefined);
    expect(result).toBe(true);
    // State should be unchanged
    expect(wrapPluginKey.getState(state)?.enabled).toBe(true);
  });

  it("does not affect document content", () => {
    let state = createStateWithWrap(true);
    const docBefore = state.doc.toJSON();

    toggleWrap(state, (tr) => {
      state = state.apply(tr);
    });

    expect(state.doc.toJSON()).toEqual(docBefore);
  });
});

// ── Plugin Key ─────────────────────────────────────────────────────

describe("wrapPluginKey", () => {
  it("returns undefined for state without wrap plugin", () => {
    const state = EditorState.create({ schema: rawSchema });
    const wrapState = wrapPluginKey.getState(state);
    expect(wrapState).toBeUndefined();
  });

  it("returns WrapState for state with wrap plugin", () => {
    const state = createStateWithWrap();
    const wrapState = wrapPluginKey.getState(state);
    expect(wrapState).toBeDefined();
    expect(typeof wrapState?.enabled).toBe("boolean");
  });
});

// ── Pretext Measurement ────────────────────────────────────────────

// Pretext requires OffscreenCanvas or a DOM canvas with 2D context for
// text measurement. happy-dom (our test env) doesn't support either.
// These tests run in real browsers (e.g., Playwright E2E).
const hasCanvas = (() => {
  try {
    if (typeof OffscreenCanvas !== "undefined") return true;
    if (typeof document !== "undefined") {
      const ctx = document.createElement("canvas").getContext("2d");
      return ctx !== null;
    }
    return false;
  } catch {
    return false;
  }
})();

describe.skipIf(!hasCanvas)("measureContent", () => {
  it("returns lineCount and height", () => {
    const result = measureContent("hello world", "13px monospace", 1000, 18);
    expect(result).toHaveProperty("lineCount");
    expect(result).toHaveProperty("height");
    expect(typeof result.lineCount).toBe("number");
    expect(typeof result.height).toBe("number");
  });

  it("single line of short text returns lineCount 1", () => {
    // Very wide width — should fit on one line
    const result = measureContent("hello", "13px monospace", 10000, 18);
    expect(result.lineCount).toBe(1);
  });

  it("empty text returns lineCount 0 or 1", () => {
    const result = measureContent("", "13px monospace", 1000, 18);
    // Empty text may be 0 or 1 lines depending on implementation
    expect(result.lineCount).toBeGreaterThanOrEqual(0);
    expect(result.lineCount).toBeLessThanOrEqual(1);
  });

  it("returns positive height for non-empty text", () => {
    const result = measureContent("hello\nworld\nfoo", "13px monospace", 1000, 18);
    expect(result.height).toBeGreaterThan(0);
  });

  it("height increases with more lines", () => {
    const oneLine = measureContent("hello", "13px monospace", 10000, 18);
    const threeLines = measureContent("hello\nworld\nfoo", "13px monospace", 10000, 18);
    expect(threeLines.height).toBeGreaterThan(oneLine.height);
  });

  it("narrow width causes more line wrapping", () => {
    const wide = measureContent("hello world this is a long line", "13px monospace", 10000, 18);
    const narrow = measureContent("hello world this is a long line", "13px monospace", 50, 18);
    expect(narrow.lineCount).toBeGreaterThanOrEqual(wide.lineCount);
  });

  it("handles multiline text with newlines", () => {
    const text = "line one\nline two\nline three";
    const result = measureContent(text, "13px monospace", 10000, 18);
    // With wide enough width, should have at least 3 lines (one per \n-delimited line)
    expect(result.lineCount).toBeGreaterThanOrEqual(3);
  });

  it("handles text with tabs", () => {
    const result = measureContent("hello\tworld", "13px monospace", 10000, 18);
    expect(result.lineCount).toBeGreaterThanOrEqual(1);
    expect(result.height).toBeGreaterThan(0);
  });

  it("handles unicode text", () => {
    const result = measureContent("Hello 世界 🌍", "13px monospace", 10000, 18);
    expect(result.lineCount).toBeGreaterThanOrEqual(1);
    expect(result.height).toBeGreaterThan(0);
  });
});

// ── Type Exports ───────────────────────────────────────────────────

describe("type exports", () => {
  it("WrapState interface shape", () => {
    const state: WrapState = { enabled: true };
    expect(state.enabled).toBe(true);
  });

  it("MeasureResult interface shape", () => {
    const result: MeasureResult = { lineCount: 5, height: 90 };
    expect(result.lineCount).toBe(5);
    expect(result.height).toBe(90);
  });
});
