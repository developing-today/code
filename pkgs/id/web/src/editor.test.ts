/**
 * Tests for editor mode functionality.
 * Tests the content mode types, schema selection, and editor initialization.
 */

import type { EditorView } from "prosemirror-view";
import { afterEach, describe, expect, it } from "vitest";
import {
  getEditorState,
  getSchema,
  getSendableSteps,
  getVersion,
  hasToolbar,
  initEditor,
  isEditable,
  rawSchema,
  richSchema,
} from "./editor";

// ============================================================================
// Content Mode Type Tests
// ============================================================================

describe("ContentMode helpers", () => {
  describe("hasToolbar", () => {
    it("returns true for rich mode", () => {
      expect(hasToolbar("rich")).toBe(true);
    });

    it("returns true for markdown mode", () => {
      expect(hasToolbar("markdown")).toBe(true);
    });

    it("returns true for plain mode", () => {
      expect(hasToolbar("plain")).toBe(true);
    });

    it("returns false for raw mode", () => {
      expect(hasToolbar("raw")).toBe(false);
    });

    it("returns false for media mode", () => {
      expect(hasToolbar("media")).toBe(false);
    });

    it("returns false for binary mode", () => {
      expect(hasToolbar("binary")).toBe(false);
    });
  });

  describe("isEditable", () => {
    it("returns true for rich mode", () => {
      expect(isEditable("rich")).toBe(true);
    });

    it("returns true for markdown mode", () => {
      expect(isEditable("markdown")).toBe(true);
    });

    it("returns true for plain mode", () => {
      expect(isEditable("plain")).toBe(true);
    });

    it("returns true for raw mode", () => {
      expect(isEditable("raw")).toBe(true);
    });

    it("returns false for media mode", () => {
      expect(isEditable("media")).toBe(false);
    });

    it("returns false for binary mode", () => {
      expect(isEditable("binary")).toBe(false);
    });
  });
});

// ============================================================================
// Schema Selection Tests
// ============================================================================

describe("getSchema", () => {
  it("returns rawSchema for raw mode", () => {
    expect(getSchema("raw")).toBe(rawSchema);
  });

  it("returns richSchema for rich mode", () => {
    expect(getSchema("rich")).toBe(richSchema);
  });

  it("returns richSchema for markdown mode", () => {
    expect(getSchema("markdown")).toBe(richSchema);
  });

  it("returns richSchema for plain mode", () => {
    expect(getSchema("plain")).toBe(richSchema);
  });

  it("returns richSchema for media mode", () => {
    expect(getSchema("media")).toBe(richSchema);
  });

  it("returns richSchema for binary mode", () => {
    expect(getSchema("binary")).toBe(richSchema);
  });
});

// ============================================================================
// Schema Structure Tests
// ============================================================================

describe("richSchema", () => {
  it("has doc node type", () => {
    expect(richSchema.nodes.doc).toBeDefined();
  });

  it("has paragraph node type", () => {
    expect(richSchema.nodes.paragraph).toBeDefined();
  });

  it("has heading node type", () => {
    expect(richSchema.nodes.heading).toBeDefined();
  });

  it("has code_block node type", () => {
    expect(richSchema.nodes.code_block).toBeDefined();
  });

  it("has list nodes", () => {
    expect(richSchema.nodes.bullet_list).toBeDefined();
    expect(richSchema.nodes.ordered_list).toBeDefined();
    expect(richSchema.nodes.list_item).toBeDefined();
  });

  it("has inline nodes", () => {
    expect(richSchema.nodes.text).toBeDefined();
    expect(richSchema.nodes.hard_break).toBeDefined();
    expect(richSchema.nodes.image).toBeDefined();
  });

  it("has formatting marks", () => {
    expect(richSchema.marks.strong).toBeDefined();
    expect(richSchema.marks.em).toBeDefined();
    expect(richSchema.marks.code).toBeDefined();
    expect(richSchema.marks.link).toBeDefined();
  });
});

describe("rawSchema", () => {
  it("has doc node type", () => {
    expect(rawSchema.nodes.doc).toBeDefined();
  });

  it("has code_block node type", () => {
    expect(rawSchema.nodes.code_block).toBeDefined();
  });

  it("has text node type", () => {
    expect(rawSchema.nodes.text).toBeDefined();
  });

  it("does NOT have paragraph node type", () => {
    expect(rawSchema.nodes.paragraph).toBeUndefined();
  });

  it("does NOT have heading node type", () => {
    expect(rawSchema.nodes.heading).toBeUndefined();
  });

  it("does NOT have list nodes", () => {
    expect(rawSchema.nodes.bullet_list).toBeUndefined();
    expect(rawSchema.nodes.ordered_list).toBeUndefined();
    expect(rawSchema.nodes.list_item).toBeUndefined();
  });

  it("has NO marks (empty marks object)", () => {
    // rawSchema.marks should be empty
    const markNames = Object.keys(rawSchema.marks);
    expect(markNames).toHaveLength(0);
  });

  it("doc content allows only code_block+", () => {
    const docSpec = rawSchema.nodes.doc.spec;
    expect(docSpec.content).toBe("code_block+");
  });

  it("code_block disallows marks", () => {
    const codeBlockSpec = rawSchema.nodes.code_block.spec;
    expect(codeBlockSpec.marks).toBe("");
  });

  it("code_block has code flag set", () => {
    const codeBlockSpec = rawSchema.nodes.code_block.spec;
    expect(codeBlockSpec.code).toBe(true);
  });

  it("code_block has language attribute defaulting to null", () => {
    const codeBlockSpec = rawSchema.nodes.code_block.spec;
    expect(codeBlockSpec.attrs).toBeDefined();
    expect(codeBlockSpec.attrs?.language).toBeDefined();
    expect(codeBlockSpec.attrs?.language.default).toBeNull();
  });
});

// ============================================================================
// Schema Compatibility Tests
// ============================================================================

describe("schema compatibility", () => {
  it("rawSchema can create empty doc", () => {
    const doc = rawSchema.topNodeType.createAndFill();
    expect(doc).toBeDefined();
    expect(doc?.type.name).toBe("doc");
  });

  it("richSchema can create empty doc", () => {
    const doc = richSchema.topNodeType.createAndFill();
    expect(doc).toBeDefined();
    expect(doc?.type.name).toBe("doc");
  });

  it("rawSchema empty doc has code_block child", () => {
    const doc = rawSchema.topNodeType.createAndFill();
    expect(doc?.content.childCount).toBeGreaterThan(0);
    expect(doc?.content.child(0).type.name).toBe("code_block");
  });

  it("richSchema empty doc has paragraph child", () => {
    const doc = richSchema.topNodeType.createAndFill();
    expect(doc?.content.childCount).toBeGreaterThan(0);
    expect(doc?.content.child(0).type.name).toBe("paragraph");
  });
});

// ============================================================================
// initEditor Integration Tests
// ============================================================================
// Note: Visual rendering tests (CSS classes, layout) are covered by E2E tests

describe("initEditor", () => {
  const views: EditorView[] = [];

  afterEach(() => {
    for (const view of views) {
      view.destroy();
    }
    views.length = 0;
  });

  it("creates an editor in raw mode with empty doc", () => {
    const container = document.createElement("div");
    const result = initEditor(container);
    views.push(result.view);

    expect(result.view).toBeDefined();
    expect(result.schema).toBeDefined();
    expect(result.clientID).toBeTypeOf("number");
    expect(result.mode).toBe("raw");
  });

  it("returns rawSchema for raw mode", () => {
    const container = document.createElement("div");
    const result = initEditor(container);
    views.push(result.view);

    expect(result.schema).toBe(rawSchema);
  });

  it("returns richSchema for rich mode", () => {
    const container = document.createElement("div");
    const result = initEditor(container, undefined, 0, "rich");
    views.push(result.view);

    expect(result.schema).toBe(richSchema);
  });

  it("generates unique clientIDs", () => {
    const container1 = document.createElement("div");
    const container2 = document.createElement("div");
    const e1 = initEditor(container1);
    const e2 = initEditor(container2);
    views.push(e1.view, e2.view);

    expect(e1.clientID).not.toBe(e2.clientID);
  });

  it("parses initial doc from JSON", () => {
    const container = document.createElement("div");
    const initialDoc = {
      type: "doc",
      content: [
        {
          type: "code_block",
          content: [{ type: "text", text: "hello" }],
        },
      ],
    };
    const result = initEditor(container, initialDoc);
    views.push(result.view);

    expect(result.view.state.doc.textContent).toContain("hello");
  });

  it("falls back to empty doc on invalid JSON", () => {
    const container = document.createElement("div");
    const result = initEditor(container, { type: "invalid" });
    views.push(result.view);

    expect(result.view.state.doc).toBeDefined();
    expect(result.view.state.doc.type.name).toBe("doc");
  });

  it("uses collabVersion for collab plugin", () => {
    const container = document.createElement("div");
    const result = initEditor(container, undefined, 5);
    views.push(result.view);

    expect(getVersion(result.view.state)).toBe(5);
  });

  it("adds id-editor-raw class for raw mode", () => {
    const container = document.createElement("div");
    const result = initEditor(container);
    views.push(result.view);

    expect(result.view.dom.classList.contains("id-editor-raw")).toBe(true);
  });

  it("adds id-editor-rich class for rich mode", () => {
    const container = document.createElement("div");
    const result = initEditor(container, undefined, 0, "rich");
    views.push(result.view);

    expect(result.view.dom.classList.contains("id-editor-rich")).toBe(true);
  });

  it("sets spellcheck to false", () => {
    const container = document.createElement("div");
    const result = initEditor(container);
    views.push(result.view);

    expect(result.view.dom.getAttribute("spellcheck")).toBe("false");
  });

  it("can be destroyed without error", () => {
    const container = document.createElement("div");
    const result = initEditor(container);

    expect(() => result.view.destroy()).not.toThrow();
    // Don't push to views since it's already destroyed
  });
});

// ============================================================================
// getEditorState Tests
// ============================================================================

describe("getEditorState", () => {
  const views: EditorView[] = [];

  afterEach(() => {
    for (const view of views) {
      view.destroy();
    }
    views.length = 0;
  });

  it("returns version, doc, and steps fields", () => {
    const container = document.createElement("div");
    const { view } = initEditor(container);
    views.push(view);

    const state = getEditorState(view);
    expect(state).toHaveProperty("version");
    expect(state).toHaveProperty("doc");
    expect(state).toHaveProperty("steps");
  });

  it("returns initial collab version", () => {
    const container = document.createElement("div");
    const { view } = initEditor(container, undefined, 0);
    views.push(view);

    const state = getEditorState(view);
    expect(state.version).toBe(0);
  });

  it("returns doc as JSON", () => {
    const container = document.createElement("div");
    const { view } = initEditor(container);
    views.push(view);

    const state = getEditorState(view);
    expect(state.doc).toBeTypeOf("object");
    expect((state.doc as Record<string, unknown>).type).toBe("doc");
  });

  it("returns null steps when no local changes", () => {
    const container = document.createElement("div");
    const { view } = initEditor(container);
    views.push(view);

    const state = getEditorState(view);
    expect(state.steps).toBeNull();
  });
});

// ============================================================================
// getSendableSteps Tests
// ============================================================================

describe("getSendableSteps", () => {
  const views: EditorView[] = [];

  afterEach(() => {
    for (const view of views) {
      view.destroy();
    }
    views.length = 0;
  });

  it("returns null when no pending steps", () => {
    const container = document.createElement("div");
    const { view } = initEditor(container);
    views.push(view);

    const result = getSendableSteps(view);
    expect(result).toBeNull();
  });

  it("returns steps after local change", () => {
    const container = document.createElement("div");
    const { view } = initEditor(container);
    views.push(view);

    // Insert text at the beginning of the document
    const tr = view.state.tr.insertText("test text", 1);
    view.dispatch(tr);

    const result = getSendableSteps(view);
    expect(result).not.toBeNull();
    expect(result).toHaveProperty("version");
    expect(result).toHaveProperty("steps");
    expect(result).toHaveProperty("clientID");
    expect(Array.isArray(result!.steps)).toBe(true);
    expect(result!.steps.length).toBeGreaterThan(0);
  });
});
