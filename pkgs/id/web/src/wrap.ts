/**
 * Word wrap toggle for the ProseMirror editor.
 *
 * Provides a ProseMirror plugin that manages wrap state and applies
 * CSS classes to toggle between wrapped (pre-wrap) and unwrapped (pre)
 * display modes.
 *
 * Uses @chenglou/pretext for text measurement — calculating content
 * dimensions without DOM reflow. This enables accurate layout info
 * when wrapping is toggled (e.g., scroll width for unwrapped mode).
 *
 * Default: wrap ON (matches CSS pre-wrap baseline).
 * Toggle: Alt+Z (VS Code convention).
 */

import { layout, prepare } from "@chenglou/pretext";
import { keymap } from "prosemirror-keymap";
import { type Command, Plugin, PluginKey } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";

// ── Types ──────────────────────────────────────────────────────────

export interface WrapState {
  /** Whether word wrapping is enabled */
  enabled: boolean;
}

/** Options for creating the wrap plugin */
export interface WrapPluginOptions {
  /** Initial wrap state. Default: true (wrapped) */
  defaultEnabled?: boolean;
}

/** Result of measuring text content with pretext */
export interface MeasureResult {
  /** Number of visual lines at the given width */
  lineCount: number;
  /** Total height in pixels */
  height: number;
}

// ── Plugin Key ─────────────────────────────────────────────────────

/**
 * Plugin key for accessing wrap state from any editor state.
 *
 * Usage: `wrapPluginKey.getState(editorState)?.enabled`
 */
export const wrapPluginKey = new PluginKey<WrapState>("wrap");

// ── CSS Classes ────────────────────────────────────────────────────

const WRAP_CLASS = "id-editor-wrap";
const NOWRAP_CLASS = "id-editor-nowrap";

// ── Commands ───────────────────────────────────────────────────────

/**
 * Toggle word wrap on/off.
 *
 * ProseMirror command — can be bound to a keymap or called directly.
 * Dispatches a transaction with metadata to flip the wrap state.
 */
export const toggleWrap: Command = (state, dispatch) => {
  if (dispatch) {
    const current = wrapPluginKey.getState(state);
    const enabled = current ? !current.enabled : false;
    dispatch(state.tr.setMeta(wrapPluginKey, { enabled }));
  }
  return true;
};

// ── Pretext Measurement ────────────────────────────────────────────

/**
 * Measure text content dimensions using pretext.
 *
 * Uses @chenglou/pretext to calculate how text would lay out
 * at a given width — without touching the DOM. This is the
 * two-phase approach: prepare (expensive, cached) then layout
 * (cheap, pure arithmetic).
 *
 * @param text - The text content to measure
 * @param font - CSS font string (e.g., "13px monospace")
 * @param maxWidth - Available width in pixels
 * @param lineHeight - Line height in pixels
 * @returns Measurement result with lineCount and height
 */
export function measureContent(text: string, font: string, maxWidth: number, lineHeight: number): MeasureResult {
  const prepared = prepare(text, font, { whiteSpace: "pre-wrap" });
  return layout(prepared, maxWidth, lineHeight);
}

// ── View Update ────────────────────────────────────────────────────

/**
 * Apply the correct CSS class to the editor view based on wrap state.
 */
function applyWrapClass(view: EditorView, enabled: boolean): void {
  const el = view.dom;
  if (enabled) {
    el.classList.add(WRAP_CLASS);
    el.classList.remove(NOWRAP_CLASS);
  } else {
    el.classList.remove(WRAP_CLASS);
    el.classList.add(NOWRAP_CLASS);
  }
}

// ── Plugin Factory ─────────────────────────────────────────────────

/**
 * Create the word wrap toggle plugin.
 *
 * This plugin:
 * 1. Maintains wrap on/off state via plugin state
 * 2. Applies CSS class to the editor DOM for styling
 * 3. Binds Alt+Z to toggle wrap (VS Code convention)
 *
 * CSS classes applied to .ProseMirror element:
 * - `.id-editor-wrap` when wrapping is ON
 * - `.id-editor-nowrap` when wrapping is OFF
 *
 * @param options - Configuration options
 * @returns Array of ProseMirror plugins (state plugin + keymap)
 */
export function createWrapPlugins(options: WrapPluginOptions = {}): Plugin[] {
  const { defaultEnabled = true } = options;

  const wrapPlugin = new Plugin<WrapState>({
    key: wrapPluginKey,

    state: {
      init(): WrapState {
        return { enabled: defaultEnabled };
      },

      apply(tr, value): WrapState {
        const meta = tr.getMeta(wrapPluginKey) as WrapState | undefined;
        if (meta !== undefined) {
          return meta;
        }
        return value;
      },
    },

    view(editorView) {
      // Apply initial class
      applyWrapClass(editorView, defaultEnabled);

      return {
        update(view) {
          const wrapState = wrapPluginKey.getState(view.state);
          if (wrapState) {
            applyWrapClass(view, wrapState.enabled);
          }
        },
      };
    },
  });

  const wrapKeymap = keymap({
    "Alt-z": toggleWrap,
  });

  return [wrapPlugin, wrapKeymap];
}
