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
 * Line starts (ascending, first is the block start) that a selection
 * overlaps: the line containing `from` through the line containing `to`.
 * A selection ending exactly at a line start does not overlap that line
 * (the standard editor rule), unless the selection is empty.
 */
export function selectedLineStarts(lineStarts: number[], from: number, to: number): number[] {
  if (lineStarts.length === 0) return [];

  // Helper to find index of the line start containing pos via binary search
  const lineIndexContaining = (pos: number): number => {
    let low = 0;
    let high = lineStarts.length - 1;
    let ans = 0;
    while (low <= high) {
      const mid = (low + high) >> 1;
      if (lineStarts[mid] <= pos) {
        ans = mid;
        low = mid + 1;
      } else {
        high = mid - 1;
      }
    }
    return ans;
  };

  // ⚡ Bolt optimization: lineStarts is sorted in ascending order.
  // Binary search for exact match instead of O(N) includes()
  const isLineStart = (pos: number): boolean => {
    let low = 0;
    let high = lineStarts.length - 1;
    while (low <= high) {
      const mid = (low + high) >> 1;
      if (lineStarts[mid] === pos) return true;
      if (lineStarts[mid] < pos) low = mid + 1;
      else high = mid - 1;
    }
    return false;
  };

  const endPos = to > from && isLineStart(to) ? to - 1 : to;
  const firstIdx = lineIndexContaining(from);
  const lastIdx = lineIndexContaining(endPos);

  // ⚡ Bolt optimization: Slice contiguous range in O(M) instead of O(N) array filter
  return lineStarts.slice(firstIdx, lastIdx + 1);
}

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
    for (const lineStart of selectedLineStarts(lineStarts, from, to)) {
      tr = tr.insertText(INDENT, lineStart + offset);
      offset += INDENT.length;
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

    // Handle selected lines (for single cursor, selectedLineStarts returns just the current line)
    const linesToDedent = selectedLineStarts(lineStarts, from, to);

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
