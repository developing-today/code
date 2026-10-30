/**
 * Go to Line dialog for the ProseMirror editor.
 *
 * Provides Ctrl+G / Cmd+G keybinding to jump to a specific line number.
 * Renders a minimal overlay input that auto-closes on Enter or Escape.
 */

import { keymap } from "prosemirror-keymap";
import type { Command, Plugin } from "prosemirror-state";
import { TextSelection } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";

/** State for the goto-line dialog. */
let dialogEl: HTMLElement | null = null;
let dialogInput: HTMLInputElement | null = null;
let activeView: EditorView | null = null;

/**
 * Document positions of every line start, in order.
 *
 * Line semantics (#745): each textblock contributes one line per
 * `\n`-separated segment of its text — a paragraph, heading or list item
 * is one line, and a code block contributes as many lines as it has
 * segments. Lines are computed by walking the document structure, NOT
 * from `doc.textContent`: ProseMirror concatenates textblocks with no
 * separator, so textContent-based counting merges block-boundary lines
 * and is blind to blocks after the first, and a text offset cannot be
 * mapped to a document position with a constant (every textblock adds
 * its own opening/closing overhead). A textblock at position `pos`
 * holds its text starting at `pos + 1`.
 */
function getLineStarts(view: EditorView): number[] {
  const starts: number[] = [];
  view.state.doc.descendants((node, pos) => {
    if (!node.isTextblock) return true; // descend into block containers (lists, quotes, cells…)
    const contentStart = pos + 1;
    starts.push(contentStart);

    let currentPos = contentStart;
    node.forEach((child) => {
      if (child.type.name === "hard_break") {
        starts.push(currentPos + child.nodeSize);
      } else if (child.isText) {
        const text = child.text;
        if (text) {
          for (let i = 0; i < text.length; i++) {
            if (text[i] === "\n") {
              starts.push(currentPos + i + 1);
            }
          }
        }
      }
      currentPos += child.nodeSize;
    });
    return false; // inline content cannot contain another textblock
  });
  return starts;
}

/** Count lines in the document (see getLineStarts for the semantics). */
function getLineCount(view: EditorView): number {
  return Math.max(1, getLineStarts(view).length);
}

/** Get the document position at the start of a given line number (1-based). */
function getLineStartPos(view: EditorView, targetLine: number): number | null {
  if (targetLine < 1) return null;
  return getLineStarts(view)[targetLine - 1] ?? null;
}

/** Create the goto-line dialog DOM. */
function createDialog(container: HTMLElement): HTMLElement {
  const dialog = document.createElement("div");
  dialog.className = "goto-line-dialog";
  dialog.setAttribute("role", "dialog");
  dialog.setAttribute("aria-label", "Go to line");
  dialog.innerHTML = `
    <label class="goto-line-label" for="goto-line-input">Go to Line:</label>
    <input type="number" min="1" class="goto-line-input" id="goto-line-input" placeholder="Line #" autocomplete="off" />
  `;
  container.prepend(dialog);
  return dialog;
}

/** Open the goto-line dialog. */
function openGotoLine(view: EditorView): void {
  const container = view.dom.parentElement;
  if (!container) return;

  // The dialog is a module singleton attached to one container. When the
  // editor is recreated (disconnect / open another file) the old dialog is
  // detached with the old container — and no production code calls
  // destroyGotoLineDialog() on teardown. Rebind instead of "opening" a
  // detached node (#718).
  if (dialogEl && dialogEl.parentElement !== container) {
    destroyGotoLineDialog();
  }

  if (!dialogEl) {
    dialogEl = createDialog(container);
    dialogInput = dialogEl.querySelector("#goto-line-input");

    if (dialogInput) {
      dialogInput.addEventListener("keydown", (e: KeyboardEvent) => {
        if (e.key === "Enter") {
          e.preventDefault();
          // Strict validation (#712): the value must be a whole line number
          // inside the document. Bare parseInt would treat 0/negatives as
          // line 1 and truncate fractions (2.9 -> 2); out-of-range values
          // used to close the dialog silently with no jump and no feedback.
          // Invalid input keeps the dialog open with the text selected so
          // the user can correct it; the cursor does not move.
          const raw = (dialogInput?.value ?? "").trim();
          const line = Number(raw);
          // ⚡ Bolt optimization: compute line starts once instead of traversing doc twice
          const lineStarts = activeView ? getLineStarts(activeView) : [];
          const lineCount = Math.max(1, lineStarts.length);
          const pos =
            activeView && raw !== "" && Number.isInteger(line) && line >= 1 && line <= lineCount
              ? (lineStarts[line - 1] ?? null)
              : null;
          if (pos === null || !activeView) {
            dialogInput?.select();
            return;
          }
          const tr = activeView.state.tr.setSelection(TextSelection.create(activeView.state.doc, pos));
          activeView.dispatch(tr);
          activeView.focus();
          // Scroll the cursor into view
          const scrollTr = activeView.state.tr.scrollIntoView();
          activeView.dispatch(scrollTr);
          closeGotoLine();
        } else if (e.key === "Escape") {
          e.preventDefault();
          closeGotoLine();
        }
      });
    }
  }

  activeView = view;
  dialogEl.style.display = "";

  // Show line count hint
  const totalLines = getLineCount(view);
  if (dialogInput) {
    dialogInput.placeholder = `Line # (1–${totalLines})`;
    dialogInput.max = String(totalLines);
    dialogInput.value = "";
    dialogInput.focus();
  }
}

/** Close the goto-line dialog. */
function closeGotoLine(): void {
  if (dialogEl) {
    dialogEl.style.display = "none";
  }
  if (activeView) {
    activeView.focus();
  }
}

/** Command: open the go-to-line dialog. */
const gotoLineCommand: Command = (_state, _dispatch, view) => {
  if (view) openGotoLine(view);
  return true;
};

/**
 * Create the Go to Line keymap plugin.
 * Binds Ctrl+G / Cmd+G to open the dialog.
 */
export function createGotoLinePlugin(): Plugin {
  return keymap({
    "Mod-g": gotoLineCommand,
  });
}

/**
 * Destroy the goto-line dialog DOM.
 * Call this when the editor is being destroyed.
 */
export function destroyGotoLineDialog(): void {
  if (dialogEl) {
    dialogEl.remove();
    dialogEl = null;
    dialogInput = null;
    activeView = null;
  }
}
