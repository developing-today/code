/**
 * Tests for the active line highlight plugin.
 */

import { EditorState, TextSelection } from "prosemirror-state";
import { DecorationSet } from "prosemirror-view";
import { describe, expect, it } from "vitest";
import { activeLineKey, createActiveLinePlugin } from "./active-line";
import { rawSchema } from "./editor";

// ── Helper ─────────────────────────────────────────────────────────

/** Create an EditorState with the active line plugin for testing */
function createStateWithActiveLine(docContent?: string): EditorState {
  const plugin = createActiveLinePlugin();
  const doc = docContent
    ? rawSchema.node("doc", null, [rawSchema.node("code_block", null, docContent ? [rawSchema.text(docContent)] : [])])
    : undefined;
  return EditorState.create({
    schema: rawSchema,
    doc,
    plugins: [plugin],
  });
}

/** Create a state with multiple code_block nodes */
function createMultiBlockState(): EditorState {
  const plugin = createActiveLinePlugin();
  const doc = rawSchema.node("doc", null, [
    rawSchema.node("code_block", null, [rawSchema.text("first block")]),
    rawSchema.node("code_block", null, [rawSchema.text("second block")]),
  ]);
  return EditorState.create({
    schema: rawSchema,
    doc,
    plugins: [plugin],
  });
}

// ── createActiveLinePlugin ─────────────────────────────────────────

describe("createActiveLinePlugin", () => {
  it("returns a plugin object", () => {
    const plugin = createActiveLinePlugin();
    expect(plugin.spec).toBeDefined();
  });

  it("plugin key matches activeLineKey", () => {
    const plugin = createActiveLinePlugin();
    expect(plugin.spec.key).toBe(activeLineKey);
  });
});

// ── Active Line Decoration ─────────────────────────────────────────

describe("active line decoration", () => {
  it("initial state has one decoration", () => {
    const state = createStateWithActiveLine("hello");
    const decoSet = activeLineKey.getState(state);
    expect(decoSet).toBeDefined();
    // Find decorations in the doc range
    const found = decoSet!.find();
    expect(found).toHaveLength(1);
  });

  it("decoration has class id-active-line", () => {
    const state = createStateWithActiveLine("hello");
    const decoSet = activeLineKey.getState(state);
    const found = decoSet!.find();
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- Decoration.type.attrs exists at runtime
    expect((found[0] as any).type.attrs.class).toBe("id-active-line");
  });

  it("decoration is a node decoration", () => {
    const state = createStateWithActiveLine("hello");
    const decoSet = activeLineKey.getState(state);
    const found = decoSet!.find();
    // Node decorations have type.spec with node info
    // In ProseMirror, Decoration.node creates decorations where from/to span the node
    const deco = found[0];
    // Node decoration: from is before the node, to is after the node
    // For a code_block containing "hello" (5 chars):
    // doc(0) > code_block(1..7) > text "hello"
    // from=0 (before code_block), to=7 (after code_block)
    expect(deco.from).toBe(0);
    expect(deco.to).toBeGreaterThan(deco.from);
  });

  it("decoration maps to the code_block node position", () => {
    const state = createStateWithActiveLine("hello world");
    const decoSet = activeLineKey.getState(state);
    const found = decoSet!.find();
    const deco = found[0];
    // In rawSchema: doc > code_block > text
    // code_block starts at pos 0, ends at pos 0 + 1 (open) + 11 (text) + 1 (close) = 13
    expect(deco.from).toBe(0);
    expect(deco.to).toBe(13);
  });

  it("empty document still has one decoration", () => {
    const state = createStateWithActiveLine();
    const decoSet = activeLineKey.getState(state);
    const found = decoSet!.find();
    expect(found).toHaveLength(1);
  });
});

// ── Decoration Updates ─────────────────────────────────────────────

describe("decoration updates", () => {
  it("decoration updates on selection change", () => {
    const state = createMultiBlockState();
    const decoSet1 = activeLineKey.getState(state);
    const found1 = decoSet1!.find();
    // Initial cursor in first block
    expect(found1).toHaveLength(1);
    const firstBlockFrom = found1[0].from;

    // Move selection to second block
    // First block: pos 0..13 (0 + 1 open + 11 "first block" + 1 close)
    // Second block: pos 13..27 (13 + 1 open + 12 "second block" + 1 close)
    const posInSecondBlock = 14; // inside second code_block
    const tr = state.tr.setSelection(TextSelection.create(state.doc, posInSecondBlock));
    const newState = state.apply(tr);

    const decoSet2 = activeLineKey.getState(newState);
    const found2 = decoSet2!.find();
    expect(found2).toHaveLength(1);
    expect(found2[0].from).not.toBe(firstBlockFrom);
    expect(found2[0].from).toBe(13);
  });

  it("decoration updates on doc change", () => {
    const state = createStateWithActiveLine("hello");
    const decoSet1 = activeLineKey.getState(state);
    const found1 = decoSet1!.find();
    const sizeBefore = found1[0].to;

    // Insert text — this changes the document
    const tr = state.tr.insertText(" world", 6); // after "hello"
    const newState = state.apply(tr);

    const decoSet2 = activeLineKey.getState(newState);
    const found2 = decoSet2!.find();
    expect(found2).toHaveLength(1);
    // Decoration should now cover a larger range
    expect(found2[0].to).toBeGreaterThan(sizeBefore);
  });

  it("decoration persists through non-selection/non-doc transactions", () => {
    const state = createStateWithActiveLine("hello");
    const decoSet1 = activeLineKey.getState(state);
    const found1 = decoSet1!.find();

    // Create a transaction that doesn't change selection or doc
    // setMeta creates a transaction with metadata but no doc/selection change
    const tr = state.tr.setMeta("someKey", "someValue");
    const newState = state.apply(tr);

    const decoSet2 = activeLineKey.getState(newState);
    const found2 = decoSet2!.find();
    expect(found2).toHaveLength(1);
    // Decoration positions should be the same
    expect(found2[0].from).toBe(found1[0].from);
    expect(found2[0].to).toBe(found1[0].to);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any -- Decoration.type.attrs exists at runtime
    expect((found2[0] as any).type.attrs.class).toBe("id-active-line");
  });
});

// ── Plugin Key ─────────────────────────────────────────────────────

describe("activeLineKey", () => {
  it("returns undefined for state without active line plugin", () => {
    const state = EditorState.create({ schema: rawSchema });
    const decoSet = activeLineKey.getState(state);
    expect(decoSet).toBeUndefined();
  });

  it("returns DecorationSet for state with active line plugin", () => {
    const state = createStateWithActiveLine("hello");
    const decoSet = activeLineKey.getState(state);
    expect(decoSet).toBeDefined();
    // Should be a DecorationSet (has a find method)
    expect(typeof decoSet!.find).toBe("function");
  });
});
