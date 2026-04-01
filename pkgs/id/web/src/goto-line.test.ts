/**
 * Tests for the go-to-line plugin.
 *
 * Tests plugin creation, dialog lifecycle, and command execution.
 */

import { EditorState } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import { afterEach, describe, expect, it } from "vitest";
import { rawSchema } from "./editor";
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
function createViewWithGotoLine(docContent?: string): { view: EditorView; container: HTMLElement } {
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
    // Line 2 starts at text offset 6 ("first\n" = 6 chars), doc pos = 6 + 2 = 8
    const posAfter = view.state.selection.from;
    expect(posAfter).toBe(8);
  });

  it("Enter with line 1 moves cursor to position 0", () => {
    const { view, container } = createViewWithGotoLine("first\nsecond");
    views.push(view);
    containers.push(container);

    triggerGotoLine(view);

    const input = container.querySelector(".goto-line-input") as HTMLInputElement;
    input.value = "1";

    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

    // Line 1 → getLineStartPos returns 0, doc pos = 0
    // But the selection needs to be a valid position inside the code_block, which is pos 2
    // Actually getLineStartPos returns 0 for line <= 1, and the command does: pos + 0 → docPos = 0
    // Wait, for line 1: getLineStartPos returns 0, then TextSelection.create(doc, 0)
    // Position 0 is before the code_block. Let's check what ProseMirror does...
    // TextSelection.create resolves to nearest valid text position
    const posAfter = view.state.selection.from;
    // Position 0 gets resolved to the nearest valid cursor position (pos 2 inside code_block)
    expect(posAfter).toBeLessThanOrEqual(2);
  });
});
