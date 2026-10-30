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
 * Every code_block node intersecting [from, to], with its doc position.
 * Used to apply indent/dedent line math per block instead of treating a
 * multi-block selection as one block's text (#1399).
 */
function codeBlocksInRange(doc: import("prosemirror-model").Node, from: number, to: number): { pos: number }[] {
  const blocks: { pos: number }[] = [];
  doc.nodesBetween(from, to, (node, pos) => {
    if (node.type.name === "code_block") {
      blocks.push({ pos });
      return false;
    }
    return true;
  });
  return blocks;
}

/**
 * Line starts (ascending) of one code_block's own text.
 * `pos` is the block node's doc position; content runs
 * [pos + 1, pos + nodeSize - 1].
 */
function blockLineStarts(
  doc: import("prosemirror-model").Node,
  block: import("prosemirror-model").Node,
  pos: number,
): { contentStart: number; contentEnd: number; lineStarts: number[] } {
  const contentStart = pos + 1;
  const contentEnd = pos + block.nodeSize - 1;
  const blockText = doc.textBetween(contentStart, contentEnd);
  const lineStarts: number[] = [contentStart];
  for (let i = 0; i < blockText.length; i++) {
    if (blockText[i] === "\n") {
      lineStarts.push(contentStart + i + 1);
    }
  }
  return { contentStart, contentEnd, lineStarts };
}

/**
 * Insert indentation at cursor, or indent all selected lines.
 */
export const indentCommand: Command = (state, dispatch) => {
  const { $from, from, to } = state.selection;

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

  // Multi-line selection: indent each overlapped line of every
  // intersected code_block (#1399). Non-code content is untouched.
  const blocks = codeBlocksInRange(state.doc, from, to);
  if (blocks.length === 0) return false;

  if (dispatch) {
    // Collect insert positions against the original doc, then apply in
    // descending order so earlier positions stay valid.
    const inserts: number[] = [];
    for (const { pos } of blocks) {
      const block = state.doc.nodeAt(pos);
      if (!block) continue;
      const { contentStart, contentEnd, lineStarts } = blockLineStarts(state.doc, block, pos);
      // Strict overlap: a selection merely touching the block's edge
      // covers none of its characters, so the block is skipped.
      if (contentStart >= to || contentEnd <= from) continue;
      for (const lineStart of selectedLineStarts(lineStarts, Math.max(from, contentStart), Math.min(to, contentEnd))) {
        inserts.push(lineStart);
      }
    }

    let tr = state.tr;
    for (const insertPos of inserts.sort((a, b) => b - a)) {
      tr = tr.insertText(INDENT, insertPos);
    }
    dispatch(tr);
  }
  return true;
};

/**
 * Remove indentation from cursor line, or dedent all selected lines.
 */
export const dedentCommand: Command = (state, dispatch) => {
  const { $from, from, to } = state.selection;

  // Check if we're in a code_block
  const inCode =
    $from.parent.type.name === "code_block" ||
    ($from.depth > 0 && $from.node($from.depth - 1)?.type.name === "code_block");

  if (!inCode) return false;

  if (dispatch) {
    const blocks = codeBlocksInRange(state.doc, from, to);
    if (blocks.length === 0) return false;

    // Collect deletions against the original doc, then apply in
    // descending order so earlier positions stay valid.
    const deletions: { pos: number; count: number }[] = [];
    for (const { pos } of blocks) {
      const block = state.doc.nodeAt(pos);
      if (!block) continue;
      const { contentStart, contentEnd, lineStarts } = blockLineStarts(state.doc, block, pos);
      // Strict overlap: a non-empty selection merely touching the block's
      // edge covers none of its characters, so the block is skipped. (An
      // empty selection/cursor at the edge still belongs to its line.)
      if (from === to ? contentStart > from || contentEnd < from : contentStart >= to || contentEnd <= from)
        continue;
      const blockText = state.doc.textBetween(contentStart, contentEnd);

      // Handle selected lines (for single cursor, selectedLineStarts returns just the current line)
      const linesToDedent = selectedLineStarts(lineStarts, Math.max(from, contentStart), Math.min(to, contentEnd));

      for (const lineStart of linesToDedent) {
        // Check how many leading spaces this line has, using the
        // block's own text (#1399)
        const lineOffset = lineStart - contentStart;
        let spacesToRemove = 0;
        for (let i = 0; i < INDENT.length && lineOffset + i < blockText.length; i++) {
          if (blockText[lineOffset + i] === " ") {
            spacesToRemove++;
          } else {
            break;
          }
        }

        if (spacesToRemove > 0) {
          deletions.push({ pos: lineStart, count: spacesToRemove });
        }
      }
    }

    if (deletions.length > 0) {
      let tr = state.tr;
      for (const { pos: deletePos, count } of deletions.sort((a, b) => b.pos - a.pos)) {
        tr = tr.delete(deletePos, deletePos + count);
      }
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
