/**
 * Tests for editor mode functionality.
 * Tests the content mode types and schema selection.
 */

import { describe, expect, it } from "vitest";
import { getSchema, hasToolbar, isEditable, rawSchema, richSchema } from "./editor";

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
