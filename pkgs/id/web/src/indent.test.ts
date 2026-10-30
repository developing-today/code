/**
 * Tests for the indent/dedent plugin.
 *
 * Tests Tab (indent) and Shift+Tab (dedent) in code_block nodes.
 */

import { EditorState, TextSelection } from "prosemirror-state";
import { describe, expect, it } from "vitest";
import { rawSchema, richSchema } from "./editor";
import { createIndentPlugin, dedentCommand, indentCommand } from "./indent";

// ── Helpers ────────────────────────────────────────────────────────

/** Create an EditorState with code_block content (rawSchema) */
function createCodeState(text: string, cursorPos?: number): EditorState {
  const doc = rawSchema.node("doc", null, [rawSchema.node("code_block", null, text ? [rawSchema.text(text)] : [])]);
  const state = EditorState.create({ doc, plugins: [createIndentPlugin()] });
  if (cursorPos !== undefined) {
    // Position inside code_block content starts at 1 (after opening tag)
    const tr = state.tr.setSelection(TextSelection.create(state.doc, cursorPos));
    return state.apply(tr);
  }
  return state;
}

/** Create an EditorState with a text selection range in code_block */
function createCodeStateWithSelection(text: string, from: number, to: number): EditorState {
  const doc = rawSchema.node("doc", null, [rawSchema.node("code_block", null, text ? [rawSchema.text(text)] : [])]);
  const state = EditorState.create({ doc, plugins: [createIndentPlugin()] });
  const tr = state.tr.setSelection(TextSelection.create(state.doc, from, to));
  return state.apply(tr);
}

/** Create an EditorState with richSchema paragraph (NOT a code_block) */
function createParagraphState(text: string): EditorState {
  const doc = richSchema.node("doc", null, [richSchema.node("paragraph", null, text ? [richSchema.text(text)] : [])]);
  return EditorState.create({ doc, plugins: [createIndentPlugin()] });
}

/** Run a command and return the new state (or null if command returned false) */
function runCommand(
  state: EditorState,
  command: (state: EditorState, dispatch?: (tr: import("prosemirror-state").Transaction) => void) => boolean,
): EditorState | null {
  let newState: EditorState | null = null;
  const result = command(state, (tr) => {
    newState = state.apply(tr);
  });
  if (!result) return null;
  return newState ?? state;
}

/** Get the text content of the first code_block in the doc */
function getCodeText(state: EditorState): string {
  return state.doc.firstChild?.textContent ?? "";
}

/** Build a code_block child node (richSchema) */
function codeBlock(text: string) {
  return richSchema.node("code_block", null, text ? [richSchema.text(text)] : []);
}

/** Build a paragraph child node (richSchema) */
function paragraph(text: string) {
  return richSchema.node("paragraph", null, text ? [richSchema.text(text)] : []);
}

/** Create an EditorState with a mixed doc and a text selection range */
function createMixedState(children: ReturnType<typeof codeBlock>[], from: number, to: number): EditorState {
  const doc = richSchema.node("doc", null, children);
  const state = EditorState.create({ doc, plugins: [createIndentPlugin()] });
  const tr = state.tr.setSelection(TextSelection.create(state.doc, from, to));
  return state.apply(tr);
}

/** Get the text content of every top-level child in the doc */
function getChildTexts(state: EditorState): string[] {
  const out: string[] = [];
  state.doc.forEach((child) => out.push(child.textContent));
  return out;
}

// ── createIndentPlugin ─────────────────────────────────────────────

describe("createIndentPlugin", () => {
  it("returns a plugin object", () => {
    const plugin = createIndentPlugin();
    expect(plugin).toBeDefined();
    expect(plugin.spec).toBeDefined();
  });

  it("integrates with EditorState", () => {
    const state = createCodeState("hello");
    expect(state).toBeDefined();
    expect(state.doc.firstChild?.type.name).toBe("code_block");
  });
});

// ── indentCommand ──────────────────────────────────────────────────

describe("indentCommand", () => {
  it("returns false outside code_block", () => {
    const state = createParagraphState("hello world");
    const result = indentCommand(state, undefined);
    expect(result).toBe(false);
  });

  it("inserts 2 spaces at cursor in code_block", () => {
    // "hello" with cursor at pos 1 (start of content, before 'h')
    // rawSchema: doc(0) > code_block(1) > text "hello"
    // pos 1 = before 'h', pos 6 = after 'o'
    const state = createCodeState("hello", 1);
    const newState = runCommand(state, indentCommand);
    expect(newState).not.toBeNull();
    expect(getCodeText(newState!)).toBe("  hello");
  });

  it("preserves existing text around cursor", () => {
    // Cursor between 'he' and 'llo' at pos 3
    const state = createCodeState("hello", 3);
    const newState = runCommand(state, indentCommand);
    expect(newState).not.toBeNull();
    expect(getCodeText(newState!)).toBe("he  llo");
  });

  it("handles cursor at start of line", () => {
    const state = createCodeState("hello", 1);
    const newState = runCommand(state, indentCommand);
    expect(newState).not.toBeNull();
    expect(getCodeText(newState!)).toBe("  hello");
  });

  it("handles cursor at end of line", () => {
    // "hello" is 5 chars, end of content = pos 6
    const state = createCodeState("hello", 6);
    const newState = runCommand(state, indentCommand);
    expect(newState).not.toBeNull();
    expect(getCodeText(newState!)).toBe("hello  ");
  });

  it("handles empty code_block", () => {
    // Empty code_block: pos 1 is inside the code_block
    const state = createCodeState("", 1);
    const newState = runCommand(state, indentCommand);
    expect(newState).not.toBeNull();
    expect(getCodeText(newState!)).toBe("  ");
  });
});

// ── indentCommand with selection ───────────────────────────────────

describe("indentCommand with selection", () => {
  it("indents multiple selected lines", () => {
    // "line1\nline2\nline3" — select across all 3 lines
    // Positions: doc(0) code_block(1) l(1) i(2) n(3) e(4) 1(5) \n(6) l(7) ...
    // line1 = pos 1-5, \n = pos 6, line2 = pos 7-11, \n = pos 12, line3 = pos 13-17
    const text = "line1\nline2\nline3";
    const state = createCodeStateWithSelection(text, 1, 1 + text.length);
    const newState = runCommand(state, indentCommand);
    expect(newState).not.toBeNull();
    expect(getCodeText(newState!)).toBe("  line1\n  line2\n  line3");
  });

  it("indents only lines overlapping the selection", () => {
    // Select only part of line2 (from 'l' in line2 to 'e' in line2)
    const text = "line1\nline2\nline3";
    // line2 starts at pos 7 (after \n at pos 6)
    const state = createCodeStateWithSelection(text, 7, 12);
    const newState = runCommand(state, indentCommand);
    expect(newState).not.toBeNull();
    const result = getCodeText(newState!);
    // line1 should also be indented (blockStart overlap logic)
    // and line2 should be indented since selection covers it
    // line3 should NOT be indented
    expect(result).toContain("  line2");
    expect(result.endsWith("line3")).toBe(true);
  });

  it("indents exactly the selected line, not its neighbours (#714)", () => {
    const text = "line1\nline2\nline3";
    const state = createCodeStateWithSelection(text, 7, 12);
    const newState = runCommand(state, indentCommand);
    expect(getCodeText(newState!)).toBe("line1\n  line2\nline3");
  });

  it("indents the line the selection starts in, even mid-line (#714)", () => {
    const text = "line1\nline2\nline3";
    const state = createCodeStateWithSelection(text, 9, 15);
    const newState = runCommand(state, indentCommand);
    expect(getCodeText(newState!)).toBe("line1\n  line2\n  line3");
  });

  it("does not drift into following lines as indents accumulate (#714)", () => {
    const text = "a\nb\nc\nd\ne";
    const state = createCodeStateWithSelection(text, 1, 2);
    const newState = runCommand(state, indentCommand);
    expect(getCodeText(newState!)).toBe("  a\nb\nc\nd\ne");
  });

  it("does not indent a line the selection only touches at its start (#714)", () => {
    const text = "line1\nline2\nline3";
    // Selection from the start of line1 to the start of line2 (pos 7)
    const state = createCodeStateWithSelection(text, 1, 7);
    const newState = runCommand(state, indentCommand);
    expect(getCodeText(newState!)).toBe("  line1\nline2\nline3");
  });
});

// ── dedentCommand ──────────────────────────────────────────────────

describe("dedentCommand", () => {
  it("returns false outside code_block", () => {
    const state = createParagraphState("  hello world");
    const result = dedentCommand(state, undefined);
    expect(result).toBe(false);
  });

  it("removes 2 leading spaces from current line", () => {
    // "  hello" with cursor somewhere on this line
    const state = createCodeState("  hello", 3);
    const newState = runCommand(state, dedentCommand);
    expect(newState).not.toBeNull();
    expect(getCodeText(newState!)).toBe("hello");
  });

  it("removes only 1 space when only 1 leading space exists", () => {
    const state = createCodeState(" hello", 2);
    const newState = runCommand(state, dedentCommand);
    expect(newState).not.toBeNull();
    expect(getCodeText(newState!)).toBe("hello");
  });

  it("does nothing when no leading spaces", () => {
    const state = createCodeState("hello", 3);
    // dedentCommand returns true (it's in code_block) but doesn't dispatch
    // when there are no spaces to remove (offset stays 0)
    let dispatched = false;
    const result = dedentCommand(state, (_tr) => {
      dispatched = true;
    });
    expect(result).toBe(true);
    // Dispatch should not have been called since offset === 0
    expect(dispatched).toBe(false);
  });

  it("handles empty code_block", () => {
    const state = createCodeState("", 1);
    let dispatched = false;
    const result = dedentCommand(state, (_tr) => {
      dispatched = true;
    });
    expect(result).toBe(true);
    expect(dispatched).toBe(false);
  });
});

// ── dedentCommand with selection ───────────────────────────────────

describe("dedentCommand with selection", () => {
  it("dedents multiple selected lines", () => {
    const text = "  line1\n  line2\n  line3";
    const state = createCodeStateWithSelection(text, 1, 1 + text.length);
    const newState = runCommand(state, dedentCommand);
    expect(newState).not.toBeNull();
    expect(getCodeText(newState!)).toBe("line1\nline2\nline3");
  });

  it("dedents exactly the selected line, not earlier lines (#714)", () => {
    // line3 starts at pos 17 in "  line1\n  line2\n  line3"
    const text = "  line1\n  line2\n  line3";
    const state = createCodeStateWithSelection(text, 17, 23);
    const newState = runCommand(state, dedentCommand);
    expect(getCodeText(newState!)).toBe("  line1\n  line2\nline3");
  });

  it("handles mixed indentation across lines", () => {
    // line1 has 2 spaces, line2 has 1 space, line3 has no spaces
    const text = "  line1\n line2\nline3";
    const state = createCodeStateWithSelection(text, 1, 1 + text.length);
    const newState = runCommand(state, dedentCommand);
    expect(newState).not.toBeNull();
    expect(getCodeText(newState!)).toBe("line1\nline2\nline3");
  });
});

// ── multi-block selections (#1399) ──────────────────────────────────

describe("indentCommand with multi-block selection (#1399)", () => {
  it("indents every overlapped code_block and leaves prose untouched", () => {
    // doc: code_block("x\ny") + paragraph("mid") + code_block("p\nq")
    const children = [codeBlock("x\ny"), paragraph("mid"), codeBlock("p\nq")];
    const doc = richSchema.node("doc", null, children);
    // Selection starts inside the first code_block, ends at doc end
    const state = createMixedState(children, 1, doc.content.size);
    const newState = runCommand(state, indentCommand);
    expect(newState).not.toBeNull();
    expect(getChildTexts(newState!)).toEqual(["  x\n  y", "mid", "  p\n  q"]);
  });

  it("indents the code_block when the selection runs into a paragraph", () => {
    // doc: code_block("a\nb") + paragraph("mid")
    const children = [codeBlock("a\nb"), paragraph("mid")];
    const doc = richSchema.node("doc", null, children);
    const state = createMixedState(children, 1, doc.content.size);
    const newState = runCommand(state, indentCommand);
    expect(newState).not.toBeNull();
    expect(getChildTexts(newState!)).toEqual(["  a\n  b", "mid"]);
  });

  it("returns false when the selection overlaps no code_block", () => {
    // Selection entirely inside the paragraph
    const children = [codeBlock("x\ny"), paragraph("mid"), codeBlock("p\nq")];
    // paragraph content is "mid" at pos 6-8
    const state = createMixedState(children, 6, 9);
    let dispatched = false;
    const result = indentCommand(state, (_tr) => {
      dispatched = true;
    });
    expect(result).toBe(false);
    expect(dispatched).toBe(false);
    expect(getChildTexts(state)).toEqual(["x\ny", "mid", "p\nq"]);
  });
});

describe("dedentCommand with multi-block selection (#1399)", () => {
  it("dedents every overlapped code_block and leaves prose untouched", () => {
    const children = [codeBlock("  x\n  y"), paragraph("mid"), codeBlock("  p\n  q")];
    const doc = richSchema.node("doc", null, children);
    const state = createMixedState(children, 1, doc.content.size);
    const newState = runCommand(state, dedentCommand);
    expect(newState).not.toBeNull();
    expect(getChildTexts(newState!)).toEqual(["x\ny", "mid", "p\nq"]);
  });

  it("dedents the code_block when the selection runs into a paragraph", () => {
    const children = [codeBlock("  a\n  b"), paragraph("mid")];
    const doc = richSchema.node("doc", null, children);
    const state = createMixedState(children, 1, doc.content.size);
    const newState = runCommand(state, dedentCommand);
    expect(newState).not.toBeNull();
    expect(getChildTexts(newState!)).toEqual(["a\nb", "mid"]);
  });

  it("returns false when the selection overlaps no code_block", () => {
    const children = [codeBlock("  x"), paragraph("mid")];
    const state = createMixedState(children, 6, 9);
    let dispatched = false;
    const result = dedentCommand(state, (_tr) => {
      dispatched = true;
    });
    expect(result).toBe(false);
    expect(dispatched).toBe(false);
    expect(getChildTexts(state)).toEqual(["  x", "mid"]);
  });
});
