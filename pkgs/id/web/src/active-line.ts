/**
 * Active line highlight plugin for ProseMirror.
 *
 * Highlights the line containing the cursor with a subtle background color.
 * Uses node decorations that apply a CSS class to the active block node.
 */

import type { EditorState } from "prosemirror-state";
import { Plugin, PluginKey } from "prosemirror-state";
import { Decoration, DecorationSet } from "prosemirror-view";

export const activeLineKey = new PluginKey<DecorationSet>("activeLine");

/**
 * Get the line decoration for the current cursor position.
 * Applies a node decoration on the innermost block containing the cursor.
 */
function getActiveLineDecoration(state: EditorState): DecorationSet {
  const { $head } = state.selection;
  const decorations: Decoration[] = [];

  // Walk up to find the innermost block node
  for (let depth = $head.depth; depth > 0; depth--) {
    const node = $head.node(depth);
    if (node.isBlock) {
      const from = $head.before(depth);
      const to = $head.after(depth);
      decorations.push(
        Decoration.node(from, to, {
          class: "id-active-line",
        }),
      );
      break;
    }
  }

  return DecorationSet.create(state.doc, decorations);
}

/**
 * Create the active line highlight plugin.
 *
 * Adds a CSS class `id-active-line` to the block node containing the cursor.
 * Style this class in CSS for the visual effect.
 */
export function createActiveLinePlugin(): Plugin {
  return new Plugin({
    key: activeLineKey,
    state: {
      init(_, state) {
        return getActiveLineDecoration(state);
      },
      apply(tr, decorations, _oldState, newState) {
        // Only update when selection changes
        if (tr.selectionSet || tr.docChanged) {
          return getActiveLineDecoration(newState);
        }
        return decorations.map(tr.mapping, tr.doc);
      },
    },
    props: {
      decorations(state) {
        return activeLineKey.getState(state) ?? DecorationSet.empty;
      },
    },
  });
}
