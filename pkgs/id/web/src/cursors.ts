/**
 * Collaborative cursor plugin for ProseMirror.
 * Tracks and displays remote user cursors/selections.
 * 
 * Note on inactive tabs: Browsers heavily throttle setInterval/setTimeout
 * in background tabs. We handle this by:
 * 1. Using a visibility change listener to refresh on tab focus
 * 2. Storing timestamps and computing opacity on-demand during render
 */

import { Plugin, PluginKey, EditorState, Transaction } from 'prosemirror-state';
import { Decoration, DecorationSet, EditorView } from 'prosemirror-view';

// Timing constants
const FADE_START_MS = 30_000;  // Start fading after 30s of inactivity
const FADE_END_MS = 60_000;   // Fully faded at 60s
const HIDE_MS = 300_000;      // Hide completely after 5 minutes (300s)
const REFRESH_INTERVAL_MS = 5_000; // Refresh every 5s when active

export interface CursorInfo {
  clientID: string | number;
  head: number;
  anchor: number;
  name?: string;
  color?: string;
  lastUpdate: number;
}

// Colors for different users (cycles through these)
const CURSOR_COLORS = [
  '#ff6b6b', // red
  '#4ecdc4', // teal  
  '#ffe66d', // yellow
  '#95e1d3', // mint
  '#a29bfe', // purple
  '#fd79a8', // pink
  '#00b894', // green
  '#e17055', // orange
];

function getColorForClient(clientID: string | number): string {
  const hash = String(clientID).split('').reduce((a, b) => {
    return ((a << 5) - a + b.charCodeAt(0)) | 0;
  }, 0);
  return CURSOR_COLORS[Math.abs(hash) % CURSOR_COLORS.length];
}

export const cursorPluginKey = new PluginKey<CursorPluginState>('cursors');

interface CursorPluginState {
  cursors: Map<string | number, CursorInfo>;
}

/**
 * Calculate opacity based on time since last update.
 * Returns 1.0 for active, fades to 0.3, then 0 after HIDE_MS.
 */
function getOpacityForAge(ageMs: number): number {
  if (ageMs < FADE_START_MS) return 1.0;
  if (ageMs >= HIDE_MS) return 0;
  if (ageMs >= FADE_END_MS) return 0.3;
  
  // Linear fade from 1.0 to 0.3 between FADE_START and FADE_END
  const fadeProgress = (ageMs - FADE_START_MS) / (FADE_END_MS - FADE_START_MS);
  return 1.0 - (fadeProgress * 0.7);
}

/**
 * Create the cursor decoration for a remote user.
 */
function createCursorDecorations(state: EditorState, cursors: Map<string | number, CursorInfo>, myClientID: string | number | null): DecorationSet {
  const decorations: Decoration[] = [];
  const docSize = state.doc.content.size;
  const now = Date.now();

  cursors.forEach((cursor, clientID) => {
    // Don't show our own cursor
    if (clientID === myClientID) return;
    
    const ageMs = now - cursor.lastUpdate;
    
    // Hide completely after HIDE_MS
    if (ageMs >= HIDE_MS) return;
    const opacity = getOpacityForAge(ageMs);
    
    // Skip if opacity is 0 (hidden)
    if (opacity <= 0) return;

    const color = cursor.color || getColorForClient(clientID);
    const name = cursor.name || `User ${String(clientID).slice(-4)}`;
    
    // Clamp positions to valid range
    const head = Math.max(0, Math.min(cursor.head, docSize));
    const anchor = Math.max(0, Math.min(cursor.anchor, docSize));

    // Create cursor line decoration (widget)
    const cursorWidget = document.createElement('span');
    cursorWidget.className = 'collab-cursor';
    cursorWidget.style.borderColor = color;
    cursorWidget.style.opacity = String(opacity);
    cursorWidget.setAttribute('data-client-id', String(clientID));
    
    const label = document.createElement('span');
    label.className = 'collab-cursor-label';
    label.style.backgroundColor = color;
    label.style.color = isLightColor(color) ? '#000' : '#fff';
    label.textContent = name;
    cursorWidget.appendChild(label);

    decorations.push(
      Decoration.widget(head, cursorWidget, { 
        side: 1,
        key: `cursor-${clientID}` 
      })
    );

    // If there's a selection (head !== anchor), highlight it
    if (head !== anchor) {
      const from = Math.min(head, anchor);
      const to = Math.max(head, anchor);
      // Adjust selection opacity based on cursor age
      const selectionOpacity = Math.round(0x80 * opacity).toString(16).padStart(2, '0');
      decorations.push(
        Decoration.inline(from, to, {
          class: 'collab-selection',
          style: `background-color: ${color}${selectionOpacity};`,
        }, {
          key: `selection-${clientID}`
        })
      );
    }
  });

  return DecorationSet.create(state.doc, decorations);
}

function isLightColor(color: string): boolean {
  const hex = color.replace('#', '');
  const r = parseInt(hex.slice(0, 2), 16);
  const g = parseInt(hex.slice(2, 4), 16);
  const b = parseInt(hex.slice(4, 6), 16);
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return luminance > 0.5;
}

export type SendCursorFn = (head: number, anchor: number) => void;

/**
 * Create a ProseMirror plugin for collaborative cursors.
 * 
 * @param myClientID - This client's ID (to filter out own cursor)
 * @param sendCursor - Callback to send cursor updates to server
 */
export function createCursorPlugin(
  myClientID: string | number | null,
  sendCursor: SendCursorFn
): Plugin {
  let lastSentSelection = { head: -1, anchor: -1 };
  let sendTimeout: ReturnType<typeof setTimeout> | null = null;
  let refreshInterval: ReturnType<typeof setInterval> | null = null;

  return new Plugin<CursorPluginState>({
    key: cursorPluginKey,

    state: {
      init(): CursorPluginState {
        return { cursors: new Map() };
      },

      apply(tr: Transaction, pluginState: CursorPluginState): CursorPluginState {
        // Check for cursor update meta
        const cursorUpdate = tr.getMeta(cursorPluginKey);
        if (cursorUpdate) {
          const newCursors = new Map(pluginState.cursors);
          if (cursorUpdate.type === 'update') {
            // If idleSecs is provided (initial load), backdate lastUpdate
            const lastUpdate = cursorUpdate.idleSecs 
              ? Date.now() - (cursorUpdate.idleSecs * 1000)
              : Date.now();
            newCursors.set(cursorUpdate.clientID, {
              clientID: cursorUpdate.clientID,
              head: cursorUpdate.head,
              anchor: cursorUpdate.anchor,
              name: cursorUpdate.name,
              color: cursorUpdate.color,
              lastUpdate,
            });
          } else if (cursorUpdate.type === 'remove') {
            newCursors.delete(cursorUpdate.clientID);
          }
          return { cursors: newCursors };
        }

        // If document changed, we need to map cursor positions
        if (tr.docChanged) {
          const newCursors = new Map<string | number, CursorInfo>();
          pluginState.cursors.forEach((cursor, clientID) => {
            const newHead = tr.mapping.map(cursor.head);
            const newAnchor = tr.mapping.map(cursor.anchor);
            newCursors.set(clientID, {
              ...cursor,
              head: newHead,
              anchor: newAnchor,
            });
          });
          return { cursors: newCursors };
        }

        return pluginState;
      },
    },

    props: {
      decorations(state: EditorState): DecorationSet {
        const pluginState = cursorPluginKey.getState(state);
        if (!pluginState) return DecorationSet.empty;
        return createCursorDecorations(state, pluginState.cursors, myClientID);
      },
    },

    view(editorView: EditorView) {
      // Send cursor position on selection change
      const sendSelectionUpdate = (): void => {
        const { head, anchor } = editorView.state.selection;
        
        // Only send if changed
        if (head === lastSentSelection.head && anchor === lastSentSelection.anchor) {
          return;
        }
        
        lastSentSelection = { head, anchor };
        
        // Debounce sends to avoid flooding
        if (sendTimeout) clearTimeout(sendTimeout);
        sendTimeout = setTimeout(() => {
          sendCursor(head, anchor);
        }, 50);
      };

      // Refresh decorations to update cursor opacity based on age
      const refreshCursorDecorations = (): void => {
        const pluginState = cursorPluginKey.getState(editorView.state);
        if (pluginState && pluginState.cursors.size > 0) {
          // Check if any cursors need opacity updates
          const now = Date.now();
          let needsRefresh = false;
          pluginState.cursors.forEach((cursor, clientID) => {
            if (clientID !== myClientID) {
              const age = now - cursor.lastUpdate;
              // Refresh if in the fading range or just past hide threshold
              if (age >= FADE_START_MS && age < HIDE_MS + 1000) {
                needsRefresh = true;
              }
            }
          });
          if (needsRefresh) {
            // Dispatch empty transaction to trigger decoration refresh
            editorView.dispatch(editorView.state.tr);
          }
        }
      };

      // Periodically refresh decorations to update fading cursors
      // Note: This interval may be throttled in inactive tabs, which is fine -
      // we'll catch up when the tab becomes visible again
      refreshInterval = setInterval(refreshCursorDecorations, REFRESH_INTERVAL_MS);

      // Handle visibility changes to refresh cursors when tab becomes active
      // This is crucial because setInterval is heavily throttled in background tabs
      const handleVisibilityChange = (): void => {
        if (document.visibilityState === 'visible') {
          // Tab became visible - immediately refresh cursor decorations
          // This catches up on any fading that should have happened while tab was inactive
          refreshCursorDecorations();
        }
      };
      document.addEventListener('visibilitychange', handleVisibilityChange);

      return {
        update(view: EditorView, prevState: EditorState): void {
          if (!view.state.selection.eq(prevState.selection)) {
            sendSelectionUpdate();
          }
        },
        destroy(): void {
          if (sendTimeout) clearTimeout(sendTimeout);
          if (refreshInterval) clearInterval(refreshInterval);
          document.removeEventListener('visibilitychange', handleVisibilityChange);
        },
      };
    },
  });
}

/**
 * Update a remote cursor in the editor.
 * @param idleSecs - If provided, backdate lastUpdate by this many seconds (for initial load)
 */
export function updateCursor(
  view: EditorView,
  clientID: string | number,
  head: number,
  anchor: number,
  name?: string,
  idleSecs?: number
): void {
  const tr = view.state.tr.setMeta(cursorPluginKey, {
    type: 'update',
    clientID,
    head,
    anchor,
    name,
    idleSecs,
  });
  view.dispatch(tr);
}

/**
 * Remove a remote cursor from the editor.
 */
export function removeCursor(view: EditorView, clientID: string | number): void {
  const tr = view.state.tr.setMeta(cursorPluginKey, {
    type: 'remove',
    clientID,
  });
  view.dispatch(tr);
}

// Connection state for cursor display
let connectionState: 'connected' | 'disconnected' = 'disconnected';

/**
 * Set the connection state (affects cursor display).
 */
export function setConnectionState(state: 'connected' | 'disconnected'): void {
  connectionState = state;
}

/**
 * Called when Init message is received - can be used to start cleanup timers.
 */
export function onInitReceived(): void {
  // Currently a no-op, but available for future use
}
