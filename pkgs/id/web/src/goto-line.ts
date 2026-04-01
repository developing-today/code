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

/** Count lines in the document (newline characters + 1). */
function getLineCount(view: EditorView): number {
  const text = view.state.doc.textContent;
  if (text.length === 0) return 1;
  let count = 1;
  for (let i = 0; i < text.length; i++) {
    if (text[i] === "\n") count++;
  }
  return count;
}

/** Get the document position at the start of a given line number (1-based). */
function getLineStartPos(view: EditorView, targetLine: number): number | null {
  let currentLine = 1;
  let pos = 0;

  if (targetLine <= 1) return 0;

  // Walk through the document text
  const text = view.state.doc.textContent;
  for (let i = 0; i < text.length; i++) {
    if (text[i] === "\n") {
      currentLine++;
      if (currentLine === targetLine) {
        // The position in text content needs to be mapped to doc position.
        // In a code_block (raw mode), text offset maps directly.
        // We need to account for node structure.
        pos = i + 1;
        break;
      }
    }
  }

  if (currentLine < targetLine) return null; // Line doesn't exist

  // Map text offset to doc position
  // In raw mode: doc structure is doc > code_block > text
  // The code_block starts at pos 1 (after doc open), text starts at pos 2
  // So doc position = text offset + 2
  // In rich mode: more complex, but we handle via resolve
  const docPos = pos + 2; // +1 for doc node, +1 for code_block node
  if (docPos > view.state.doc.content.size) return null;
  return docPos;
}

/** Create the goto-line dialog DOM. */
function createDialog(container: HTMLElement): HTMLElement {
  const dialog = document.createElement("div");
  dialog.className = "goto-line-dialog";
  dialog.innerHTML = `
    <label class="goto-line-label">Go to Line:</label>
    <input type="number" min="1" class="goto-line-input" id="goto-line-input" placeholder="Line #" autocomplete="off" />
  `;
  container.prepend(dialog);
  return dialog;
}

/** Open the goto-line dialog. */
function openGotoLine(view: EditorView): void {
  const container = view.dom.parentElement;
  if (!container) return;

  if (!dialogEl) {
    dialogEl = createDialog(container);
    dialogInput = dialogEl.querySelector("#goto-line-input");

    if (dialogInput) {
      dialogInput.addEventListener("keydown", (e: KeyboardEvent) => {
        if (e.key === "Enter") {
          e.preventDefault();
          const line = parseInt(dialogInput?.value ?? "", 10);
          if (!isNaN(line) && activeView) {
            const pos = getLineStartPos(activeView, line);
            if (pos !== null) {
              const tr = activeView.state.tr.setSelection(TextSelection.create(activeView.state.doc, pos));
              activeView.dispatch(tr);
              activeView.focus();
              // Scroll the cursor into view
              const scrollTr = activeView.state.tr.scrollIntoView();
              activeView.dispatch(scrollTr);
            }
          }
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
