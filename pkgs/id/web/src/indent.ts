/**
 * Indentation plugin for ProseMirror.
 *
 * Provides Tab / Shift+Tab for indent/dedent in code blocks.
 * - Tab: insert 2 spaces (or indent selected lines)
 * - Shift+Tab: remove up to 2 leading spaces from selected lines
 */

import { keymap } from "prosemirror-keymap";
import type { Command, Plugin } from "prosemirror-state";

const INDENT = "  "; // 2 spaces

/**
 * Insert indentation at cursor, or indent all selected lines.
 */
export const indentCommand: Command = (state, dispatch) => {
  const { $from, $to, from, to } = state.selection;

  // Check if we're in a code_block
  const inCode =
    $from.parent.type.name === "code_block" ||
    ($from.depth > 0 && $from.node($from.depth - 1)?.type.name === "code_block");

  if (!inCode) return false;

  // Single cursor (no selection) — just insert spaces
  if (from === to) {
    if (dispatch) {
      dispatch(state.tr.insertText(INDENT, from));
    }
    return true;
  }

  // Multi-line selection: indent each line
  if (dispatch) {
    // Find all line start positions within the selection
    let tr = state.tr;
    let offset = 0;

    // Start of the block
    const blockStart = $from.start($from.depth);
    const blockEnd = $to.end($to.depth);
    const blockText = state.doc.textBetween(blockStart, blockEnd);

    // Find newline positions to determine line starts
    const lineStarts: number[] = [blockStart]; // first line starts at blockStart
    for (let i = 0; i < blockText.length; i++) {
      if (blockText[i] === "\n") {
        lineStarts.push(blockStart + i + 1);
      }
    }

    // Only indent lines that overlap with the selection
    for (const lineStart of lineStarts) {
      if (lineStart >= from || lineStart === blockStart) {
        // Check this line is within selection
        if (lineStart <= to + offset) {
          tr = tr.insertText(INDENT, lineStart + offset);
          offset += INDENT.length;
        }
      }
    }

    dispatch(tr);
  }
  return true;
};

/**
 * Remove indentation from cursor line, or dedent all selected lines.
 */
export const dedentCommand: Command = (state, dispatch) => {
  const { $from, $to, from, to } = state.selection;

  // Check if we're in a code_block
  const inCode =
    $from.parent.type.name === "code_block" ||
    ($from.depth > 0 && $from.node($from.depth - 1)?.type.name === "code_block");

  if (!inCode) return false;

  if (dispatch) {
    const blockStart = $from.start($from.depth);
    const blockEnd = $to.end($to.depth);
    const blockText = state.doc.textBetween(blockStart, blockEnd);

    // Find all line start positions
    const lineStarts: number[] = [blockStart];
    for (let i = 0; i < blockText.length; i++) {
      if (blockText[i] === "\n") {
        lineStarts.push(blockStart + i + 1);
      }
    }

    let tr = state.tr;
    let offset = 0;

    // For single cursor, only handle current line
    const linesToDedent =
      from === to
        ? lineStarts.filter(
            (ls) =>
              ls <= from &&
              (lineStarts.indexOf(ls) === lineStarts.length - 1 || lineStarts[lineStarts.indexOf(ls) + 1] > from),
          )
        : lineStarts.filter((ls) => ls <= to);

    for (const lineStart of linesToDedent) {
      const adjustedPos = lineStart + offset;
      // Check how many leading spaces this line has
      const lineOffset = lineStart - blockStart;
      let spacesToRemove = 0;
      for (let i = 0; i < INDENT.length && lineOffset + i < blockText.length; i++) {
        if (blockText[lineOffset + i] === " ") {
          spacesToRemove++;
        } else {
          break;
        }
      }

      if (spacesToRemove > 0) {
        tr = tr.delete(adjustedPos, adjustedPos + spacesToRemove);
        offset -= spacesToRemove;
      }
    }

    if (offset !== 0) {
      dispatch(tr);
    }
  }
  return true;
};

/**
 * Create the indentation keymap plugin.
 * Tab to indent, Shift+Tab to dedent in code blocks.
 */
export function createIndentPlugin(): Plugin {
  return keymap({
    Tab: indentCommand,
    "Shift-Tab": dedentCommand,
  });
}
