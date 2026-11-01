/**
 * Tests for the go-to-line plugin.
 *
 * Tests plugin creation, dialog lifecycle, and command execution.
 */

import type { Node as PMNode, Schema } from "prosemirror-model";
import { EditorState } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { afterEach, describe, expect, it } from "vitest";
import { rawSchema, richSchema } from "./editor";
import { createGotoLinePlugin, destroyGotoLineDialog } from "./goto-line";

// ── Helpers ────────────────────────────────────────────────────────

/** Create an EditorState with the goto-line plugin. */
function createStateWithGotoLine(docContent?: string): EditorState {
  const plugin = createGotoLinePlugin();
  const doc = docContent
    ? rawSchema.node("doc", null, [rawSchema.node("code_block", null, docContent ? [rawSchema.text(docContent)] : [])])
    : undefined;
  return EditorState.create({
    schema: rawSchema,
    doc,
    plugins: [plugin],
  });
}

/** Create an EditorView in a container div with the goto-line plugin. */
function createViewWithGotoLine(docContent?: string): {
  view: EditorView;
  container: HTMLElement;
} {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const state = createStateWithGotoLine(docContent);
  const view = new EditorView(container, { state });
  return { view, container };
}

/**
 * Trigger the Mod-g keymap handler on a view.
 * Uses `call(plugin, ...)` to fix the `this` context that TypeScript complains about.
 */
function triggerGotoLine(view: EditorView): boolean {
  const plugin = view.state.plugins[0];
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- handleKeyDown this context workaround
  const handler = (plugin.props as any).handleKeyDown;
  if (!handler) return false;
  return handler.call(plugin, view, new KeyboardEvent("keydown", { key: "g", ctrlKey: true }));
}

// ── Tracked resources for cleanup ──────────────────────────────────

const views: EditorView[] = [];
const containers: HTMLElement[] = [];

afterEach(() => {
  for (const view of views) {
    view.destroy();
  }
  views.length = 0;
  destroyGotoLineDialog();
  for (const container of containers) {
    container.remove();
  }
  containers.length = 0;
});

// ── createGotoLinePlugin ───────────────────────────────────────────

describe("createGotoLinePlugin", () => {
  it("returns a plugin object", () => {
    const plugin = createGotoLinePlugin();
    expect(plugin.spec).toBeDefined();
  });

  it("plugin has props (keymap plugins expose handleKeyDown)", () => {
    const plugin = createGotoLinePlugin();
    expect(plugin.props).toBeDefined();
    // Keymap plugins register a handleKeyDown prop
    expect(plugin.props.handleKeyDown).toBeDefined();
  });

  it("can be added to an EditorState without errors", () => {
    const state = createStateWithGotoLine("hello");
    expect(state).toBeDefined();
    expect(state.doc.textContent).toBe("hello");
  });
});

// ── destroyGotoLineDialog ──────────────────────────────────────────

describe("destroyGotoLineDialog", () => {
  it("can be called without error when no dialog exists", () => {
    expect(() => destroyGotoLineDialog()).not.toThrow();
  });

  it("can be called multiple times safely", () => {
    expect(() => {
      destroyGotoLineDialog();
      destroyGotoLineDialog();
      destroyGotoLineDialog();
    }).not.toThrow();
  });

  it("removes dialog DOM after it has been created", () => {
    const { view, container } = createViewWithGotoLine("hello\nworld");
    views.push(view);
    containers.push(container);

    // Execute the goto-line command to create the dialog
    triggerGotoLine(view);

    // Dialog should exist
    const dialogBefore = container.querySelector(".goto-line-dialog");
    expect(dialogBefore).not.toBeNull();

    // Destroy should remove it
    destroyGotoLineDialog();
    const dialogAfter = container.querySelector(".goto-line-dialog");
    expect(dialogAfter).toBeNull();
  });
});

// ── Go-to-line command ─────────────────────────────────────────────

describe("go-to-line command", () => {
  it("command returns true via keymap handler", () => {
    const { view, container } = createViewWithGotoLine("hello\nworld");
    views.push(view);
    containers.push(container);

    // The keymap plugin exposes handleKeyDown; Mod-g maps to the command
    // Simulate Ctrl+G (or Cmd+G on Mac)
    const handled = triggerGotoLine(view);
    // handleKeyDown returns true when the keymap matches
    expect(handled).toBe(true);
  });

  it("associates its label with the line input", () => {
    const { view, container } = createViewWithGotoLine("line1\nline2");
    views.push(view);
    containers.push(container);

    triggerGotoLine(view);

    const label = container.querySelector<HTMLLabelElement>(".goto-line-label");
    const input = container.querySelector<HTMLInputElement>("#goto-line-input");
    expect(label?.htmlFor).toBe("goto-line-input");
    expect(input?.labels?.[0]).toBe(label);
  });

  it("creates a .goto-line-dialog element in the container", () => {
    const { view, container } = createViewWithGotoLine("line1\nline2\nline3");
    views.push(view);
    containers.push(container);

    // Trigger the command
    triggerGotoLine(view);

    const dialog = container.querySelector(".goto-line-dialog");
    expect(dialog).not.toBeNull();
  });

  it("dialog contains an input element", () => {
    const { view, container } = createViewWithGotoLine("line1\nline2");
    views.push(view);
    containers.push(container);

    triggerGotoLine(view);

    const input = container.querySelector(".goto-line-input");
    expect(input).not.toBeNull();
    expect(input?.tagName.toLowerCase()).toBe("input");
  });

  it("dialog contains a label", () => {
    const { view, container } = createViewWithGotoLine("line1\nline2");
    views.push(view);
    containers.push(container);

    triggerGotoLine(view);

    const label = container.querySelector(".goto-line-label");
    expect(label).not.toBeNull();
    expect(label?.textContent).toBe("Go to Line:");
  });

  it("input placeholder shows line count", () => {
    const { view, container } = createViewWithGotoLine("line1\nline2\nline3");
    views.push(view);
    containers.push(container);

    triggerGotoLine(view);

    const input = container.querySelector(".goto-line-input") as HTMLInputElement;
    expect(input).not.toBeNull();
    // 3 lines in doc → placeholder should contain "3"
    expect(input.placeholder).toContain("3");
  });

  it("opening command again reuses existing dialog", () => {
    const { view, container } = createViewWithGotoLine("hello");
    views.push(view);
    containers.push(container);

    triggerGotoLine(view);
    triggerGotoLine(view);

    // Should still be just one dialog
    const dialogs = container.querySelectorAll(".goto-line-dialog");
    expect(dialogs).toHaveLength(1);
  });
});

// ── Dialog input interaction ───────────────────────────────────────

describe("dialog input interaction", () => {
  it("Escape key hides the dialog", () => {
    const { view, container } = createViewWithGotoLine("hello\nworld");
    views.push(view);
    containers.push(container);

    triggerGotoLine(view);

    const dialog = container.querySelector(".goto-line-dialog") as HTMLElement;
    expect(dialog).not.toBeNull();

    const input = container.querySelector(".goto-line-input") as HTMLInputElement;
    // Simulate pressing Escape on the input
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));

    expect(dialog.style.display).toBe("none");
  });

  it("Enter key hides the dialog", () => {
    const { view, container } = createViewWithGotoLine("hello\nworld");
    views.push(view);
    containers.push(container);

    triggerGotoLine(view);

    const dialog = container.querySelector(".goto-line-dialog") as HTMLElement;
    const input = container.querySelector(".goto-line-input") as HTMLInputElement;
    input.value = "1";

    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(dialog.style.display).toBe("none");
  });

  it("Enter with valid line number moves cursor", () => {
    const { view, container } = createViewWithGotoLine("first\nsecond\nthird");
    views.push(view);
    containers.push(container);

    triggerGotoLine(view);

    const input = container.querySelector(".goto-line-input") as HTMLInputElement;
    input.value = "2";

    // Before: cursor is at default position
    const _posBefore = view.state.selection.from;

    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

    // After: cursor should have moved (to line 2 start)
    // In rawSchema: doc > code_block > text "first\nsecond\nthird"
    // The code_block sits at pos 0 and its text starts at pos 1, so
    // line 2 (text offset 6, after "first\n") starts at doc pos 7.
    // (#745: the old textContent + 2 mapping landed one char into the
    // line, at pos 8.)
    const posAfter = view.state.selection.from;
    expect(posAfter).toBe(7);
  });

  it.each([
    ["0", "zero"],
    ["-3", "negative"],
    ["2.9", "fractional"],
    ["999", "out of range"],
    ["", "empty"],
  ])("Enter with %s (%s) does not move the cursor and keeps the dialog open", (value) => {
    const { view, container } = createViewWithGotoLine("first\nsecond\nthird");
    views.push(view);
    containers.push(container);

    triggerGotoLine(view);

    const dialog = container.querySelector(".goto-line-dialog") as HTMLElement;
    const input = container.querySelector(".goto-line-input") as HTMLInputElement;
    input.value = value;
    const posBefore = view.state.selection.from;

    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

    // #712: invalid input must not be silently reinterpreted (0 -> line 1,
    // 2.9 -> line 2) or swallowed (999) — no move, dialog stays open.
    expect(view.state.selection.from).toBe(posBefore);
    expect(dialog.style.display).not.toBe("none");
  });

  it("Enter with line 1 moves cursor to the first line start", () => {
    const { view, container } = createViewWithGotoLine("first\nsecond");
    views.push(view);
    containers.push(container);

    triggerGotoLine(view);

    const input = container.querySelector(".goto-line-input") as HTMLInputElement;
    input.value = "1";

    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

    // Line 1 starts at the code_block's content start: the block sits
    // at pos 0, so its text starts at doc pos 1 (#745 doc-walk mapping).
    const posAfter = view.state.selection.from;
    expect(posAfter).toBe(1);
  });
});

// ── Line counting outside a single code_block (#745) ───────────────

describe("go-to-line across textblocks (#745)", () => {
  /** Create a view from a pre-built doc node (any schema). */
  function createViewFromDoc(
    schema: Schema,
    doc: PMNode,
  ): {
    view: EditorView;
    container: HTMLElement;
  } {
    const container = document.createElement("div");
    document.body.appendChild(container);
    const state = EditorState.create({
      schema,
      doc,
      plugins: [createGotoLinePlugin()],
    });
    const view = new EditorView(container, { state });
    views.push(view);
    containers.push(container);
    return { view, container };
  }

  function gotoLine(container: HTMLElement, value: string): void {
    const input = container.querySelector(".goto-line-input") as HTMLInputElement;
    input.value = value;
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  }

  it("rich doc of three paragraphs reports 3 lines and jumps to the second paragraph", () => {
    const doc = richSchema.node("doc", null, [
      richSchema.node("paragraph", null, [richSchema.text("alpha")]),
      richSchema.node("paragraph", null, [richSchema.text("beta")]),
      richSchema.node("paragraph", null, [richSchema.text("gamma")]),
    ]);
    const { view, container } = createViewFromDoc(richSchema, doc);

    triggerGotoLine(view);
    const input = container.querySelector(".goto-line-input") as HTMLInputElement;
    // textContent is "alphabetagamma" (no separators) — the old count said 1.
    expect(input.placeholder).toContain("3");
    expect(input.max).toBe("3");

    gotoLine(container, "2");
    // Second paragraph sits at pos 7 (first is 1 + 5 + 1 wide); its text starts at 8.
    expect(view.state.selection.from).toBe(8);
  });

  it("raw doc of two code_blocks counts every line and jumps into the second block", () => {
    const doc = rawSchema.node("doc", null, [
      rawSchema.node("code_block", null, [rawSchema.text("a\nb")]),
      rawSchema.node("code_block", null, [rawSchema.text("c\nd")]),
    ]);
    const { view, container } = createViewFromDoc(rawSchema, doc);

    triggerGotoLine(view);
    const input = container.querySelector(".goto-line-input") as HTMLInputElement;
    expect(input.max).toBe("4");

    gotoLine(container, "3");
    // Second block sits at pos 5; its text ("c") starts at pos 6.
    expect(view.state.selection.from).toBe(6);

    triggerGotoLine(view);
    gotoLine(container, "4");
    // "d" follows "c\n" inside the second block: pos 8.
    expect(view.state.selection.from).toBe(8);
  });

  it("mixed rich doc: paragraph + multi-line code_block + paragraph", () => {
    const doc = richSchema.node("doc", null, [
      richSchema.node("paragraph", null, [richSchema.text("intro")]),
      richSchema.node("code_block", null, [richSchema.text("x\ny")]),
      richSchema.node("paragraph", null, [richSchema.text("outro")]),
    ]);
    const { view, container } = createViewFromDoc(richSchema, doc);

    triggerGotoLine(view);
    const input = container.querySelector(".goto-line-input") as HTMLInputElement;
    expect(input.max).toBe("4");

    gotoLine(container, "3");
    // code_block sits at pos 7, text starts at 8; line 3 is "y" at pos 10.
    expect(view.state.selection.from).toBe(10);

    triggerGotoLine(view);
    gotoLine(container, "4");
    // Trailing paragraph sits at pos 12; its text starts at pos 13.
    expect(view.state.selection.from).toBe(13);
  });
});

// ── Editor recreation (#718) ───────────────────────────────────────

describe("goto-line dialog across editor recreation", () => {
  it("Ctrl+G in a recreated editor shows a dialog in the new container without destroyGotoLineDialog()", () => {
    const first = createViewWithGotoLine("first\nsecond");
    triggerGotoLine(first.view);
    expect(first.container.querySelector(".goto-line-dialog")).not.toBeNull();

    // Production teardown (main.ts disconnect) destroys only the view —
    // it never calls destroyGotoLineDialog(). Mirror that exactly.
    first.view.destroy();
    first.container.remove();

    const second = createViewWithGotoLine("first\nsecond");
    views.push(second.view);
    containers.push(second.container);
    triggerGotoLine(second.view);
    expect(second.container.querySelector(".goto-line-dialog")).not.toBeNull();
    expect(second.container.querySelector(".goto-line-input")).not.toBeNull();
  });
});
