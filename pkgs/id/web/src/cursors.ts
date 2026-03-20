/**
 * Collaborative cursor plugin for ProseMirror.
 * Tracks and displays remote user cursors/selections.
 *
 * Features:
 * - Opacity fading based on cursor age (30s fade start, 60s minimum, 5m hidden)
 * - Strobe animation that slows as cursors age (1s→3s cycle)
 * - Strobe pauses on WebSocket disconnect
 * - Hover interactions with delayed restore
 * - Reconnect cleanup (removes stale cursors after Init)
 *
 * Note on inactive tabs: Browsers heavily throttle setInterval/setTimeout
 * in background tabs. We handle this by:
 * 1. Using a visibility change listener to refresh on tab focus
 * 2. Storing timestamps and computing opacity on-demand during render
 */

import { Plugin, PluginKey, EditorState, Transaction } from "prosemirror-state";
import { Decoration, DecorationSet, EditorView } from "prosemirror-view";
import {
  FADE_START_MS,
  FADE_END_MS,
  HIDE_MS,
  LINE_THRESHOLD_PX,
  getOpacityForAge,
  getStrobeDurationMs,
  isLightColor,
  getColorForClient,
  groupCursorsByPosition,
  clusterOverlappingGroups,
  estimateTooltipWidth,
  type CursorForMerge,
  type PositionGroup,
  type MergeCluster,
} from "./cursor-utils";

// Additional timing constants (not exported from utils)
const REFRESH_INTERVAL_MS = 5_000; // Refresh every 5s when active
const RECONNECT_CLEANUP_DELAY_MS = 1_000; // Wait 1s after Init before cleanup

export interface CursorInfo {
  clientID: string | number;
  head: number;
  anchor: number;
  name?: string;
  color?: string;
  lastUpdate: number;
}

export const cursorPluginKey = new PluginKey<CursorPluginState>("cursors");

interface CursorPluginState {
  cursors: Map<string | number, CursorInfo>;
}

// ============================================================================
// Strobe State Management (CSS Hybrid Approach)
// ============================================================================

interface StrobeInfo {
  element: HTMLElement;
  baseOpacity: number;
  paused: boolean; // True when hover-paused
}

const strobeInfos = new Map<string | number, StrobeInfo>();

// ============================================================================
// Hover State Management
// ============================================================================

const HOVER_RESTORE_DELAY_MS = 1_000; // 1s delay before restoring after hover
const hoverRestoreTimers = new Map<string | number, ReturnType<typeof setTimeout>>();

/**
 * Handle mouse entering a cursor label.
 * Sets hovered state immediately (100% opacity, no strobe, bolder, elevated z-index).
 */
function handleCursorMouseEnter(clientID: string | number): void {
  // Clear any pending restore timer
  const timer = hoverRestoreTimers.get(clientID);
  if (timer) {
    clearTimeout(timer);
    hoverRestoreTimers.delete(clientID);
  }

  const info = strobeInfos.get(clientID);
  if (!info) return;

  // Mark as hover-paused
  info.paused = true;

  // Add hover class (CSS handles visual changes)
  info.element.classList.add("collab-cursor-hovered");
}

/**
 * Handle mouse leaving a cursor label.
 * Starts 1s timer before restoring normal state.
 */
function handleCursorMouseLeave(clientID: string | number): void {
  // Start delayed restore
  const timer = setTimeout(() => {
    hoverRestoreTimers.delete(clientID);
    restoreCursorFromHover(clientID);
  }, HOVER_RESTORE_DELAY_MS);

  hoverRestoreTimers.set(clientID, timer);
}

/**
 * Restore cursor to normal state after hover.
 */
function restoreCursorFromHover(clientID: string | number): void {
  const info = strobeInfos.get(clientID);
  if (!info) return;

  // Remove hover class
  info.element.classList.remove("collab-cursor-hovered");

  // Resume strobe if connected
  info.paused = false;
  if (connectionState === "connected") {
    const durationMs = getStrobeDurationMs(info.baseOpacity);
    info.element.style.setProperty(
      "--strobe-state",
      durationMs > 0 ? "running" : "paused",
    );
  }
}

/**
 * Clean up hover timer for a cursor being removed.
 */
function cleanupHoverTimer(clientID: string | number): void {
  const timer = hoverRestoreTimers.get(clientID);
  if (timer) {
    clearTimeout(timer);
    hoverRestoreTimers.delete(clientID);
  }
}

/**
 * Remove cursor from strobe tracking and clean up hover state.
 */
function unregisterCursorStrobe(clientID: string | number): void {
  cleanupHoverTimer(clientID);
  strobeInfos.delete(clientID);
}

// ============================================================================
// Connection State Management
// ============================================================================

let connectionState: "connected" | "disconnected" = "disconnected";

/**
 * Set the connection state (affects cursor strobe display).
 * When disconnected, all cursor strobing is paused.
 */
export function setConnectionState(state: "connected" | "disconnected"): void {
  connectionState = state;

  // Update all cursor strobe states
  strobeInfos.forEach((info) => {
    if (state === "disconnected") {
      // Pause strobing, set to base opacity
      info.element.style.setProperty("--strobe-state", "paused");
      info.element.style.opacity = String(info.baseOpacity);
    } else if (!info.paused) {
      // Resume strobing (unless hover-paused)
      const durationMs = getStrobeDurationMs(info.baseOpacity);
      info.element.style.setProperty(
        "--strobe-state",
        durationMs > 0 ? "running" : "paused",
      );
    }
  });

  // Cancel reconnect cleanup timer on disconnect
  if (state === "disconnected" && reconnectCleanupTimer) {
    clearTimeout(reconnectCleanupTimer);
    reconnectCleanupTimer = null;
  }
}

// ============================================================================
// Reconnect Cleanup
// ============================================================================

let reconnectCleanupTimer: ReturnType<typeof setTimeout> | null = null;
let freshCursorIDs: Set<string | number> = new Set();
let cleanupEditorView: EditorView | null = null;

/**
 * Set the editor view for cleanup operations.
 * Should be called after editor initialization.
 */
export function setCleanupEditorView(view: EditorView): void {
  cleanupEditorView = view;
}

/**
 * Mark a cursor as fresh (received update since Init).
 * Call this when receiving a cursor update from the server.
 */
export function markCursorFresh(clientID: string | number): void {
  freshCursorIDs.add(clientID);
}

/**
 * Called when Init message is received.
 * Starts a 1s timer to clean up stale cursors.
 */
export function onInitReceived(): void {
  // Cancel any existing cleanup timer
  if (reconnectCleanupTimer) {
    clearTimeout(reconnectCleanupTimer);
  }

  // Reset fresh cursor tracking
  freshCursorIDs = new Set();

  // Start cleanup timer
  reconnectCleanupTimer = setTimeout(() => {
    if (cleanupEditorView && connectionState === "connected") {
      performReconnectCleanup();
    }
    reconnectCleanupTimer = null;
  }, RECONNECT_CLEANUP_DELAY_MS);
}

/**
 * Remove cursors that weren't marked fresh after reconnect.
 */
function performReconnectCleanup(): void {
  if (!cleanupEditorView) return;

  const pluginState = cursorPluginKey.getState(cleanupEditorView.state);
  if (!pluginState) return;

  const staleCursors: (string | number)[] = [];
  pluginState.cursors.forEach((_, clientID) => {
    if (!freshCursorIDs.has(clientID)) {
      staleCursors.push(clientID);
    }
  });

  // Remove stale cursors
  staleCursors.forEach((clientID) => {
    console.log("[cursors] Removing stale cursor:", clientID);
    removeCursorInternal(cleanupEditorView!, clientID);
    unregisterCursorStrobe(clientID);
  });

  freshCursorIDs.clear();
}

// ============================================================================
// Same-Line Detection
// ============================================================================

/**
 * Check if a remote cursor position is on the same rendered line as the local selection.
 * Uses EditorView.coordsAtPos() for accurate DOM-based line detection.
 */
function isOnSameLineAsLocal(
  remoteCursorPos: number,
  localSelectionHead: number,
  view: EditorView,
): boolean {
  try {
    const localCoords = view.coordsAtPos(localSelectionHead);
    const remoteCoords = view.coordsAtPos(remoteCursorPos);

    // Compare Y positions (top of the line)
    return Math.abs(localCoords.top - remoteCoords.top) < LINE_THRESHOLD_PX;
  } catch {
    // coordsAtPos can throw if position is invalid
    return false;
  }
}

// ============================================================================
// Cursor Decoration Creation
// ============================================================================

/**
 * Wrapper for doGroupsOverlap that adapts EditorView to the callback interface.
 */
function checkGroupsOverlap(groups: PositionGroup[], view: EditorView | null): boolean {
  if (!view) return false;
  const getLeftCoord = (pos: number): number => {
    return view.coordsAtPos(pos).left;
  };
  return doGroupsOverlap(groups, getLeftCoord);
}

/**
 * Create a merged tooltip bar for multiple position groups on the same line
 */
function createMergedBar(
  groups: PositionGroup[],
  handleMouseEnter: (clientID: string | number) => void,
  handleMouseLeave: (clientID: string | number) => void,
): HTMLElement {
  const bar = document.createElement("span");
  bar.className = "collab-cursor-bar";

  groups.forEach((group, groupIndex) => {
    // Add divider BETWEEN position groups (not within same position)
    if (groupIndex > 0) {
      const divider = document.createElement("span");
      divider.className = "collab-cursor-bar-divider";
      bar.appendChild(divider);
    }

    // Add all cursors in this position group (no dividers between them)
    group.cursors.forEach((cursor) => {
      const segment = document.createElement("span");
      segment.className = "collab-cursor-bar-segment";
      segment.style.backgroundColor = cursor.color;
      segment.style.color = isLightColor(cursor.color) ? "#000" : "#fff";
      segment.style.opacity = String(cursor.opacity);
      segment.textContent = cursor.name;
      segment.setAttribute("data-client-id", String(cursor.clientID));
      segment.setAttribute("data-position", String(cursor.head));

      // Hover handlers for individual segments
      segment.addEventListener("mouseenter", () =>
        handleMouseEnter(cursor.clientID),
      );
      segment.addEventListener("mouseleave", () =>
        handleMouseLeave(cursor.clientID),
      );

      bar.appendChild(segment);
    });
  });

  return bar;
}

/**
 * Create the cursor decoration for a remote user.
 * @param view - Optional EditorView for same-line detection (may be null during initial render)
 */
function createCursorDecorations(
  state: EditorState,
  cursors: Map<string | number, CursorInfo>,
  myClientID: string | number | null,
  view: EditorView | null,
): DecorationSet {
  const decorations: Decoration[] = [];
  const docSize = state.doc.content.size;
  const now = Date.now();
  const localHead = state.selection.head;

  // First pass: collect all visible cursors with computed properties
  const visibleCursors: CursorForMerge[] = [];

  cursors.forEach((cursor, clientID) => {
    // Don't show our own cursor
    if (clientID === myClientID) return;

    const ageMs = now - cursor.lastUpdate;

    // Hide completely after HIDE_MS
    if (ageMs >= HIDE_MS) return;
    const baseOpacity = getOpacityForAge(ageMs);

    // Skip if base opacity is 0 (hidden)
    if (baseOpacity <= 0) return;

    const color = cursor.color || getColorForClient(clientID);
    const name = cursor.name || `${String(clientID).slice(-4)}`;

    // Clamp positions to valid range
    const head = Math.max(0, Math.min(cursor.head, docSize));
    const anchor = Math.max(0, Math.min(cursor.anchor, docSize));

    // Check if this cursor is on the same line as the local cursor
    const onSameLine =
      view !== null && isOnSameLineAsLocal(head, localHead, view);
    const opacity = onSameLine ? 1.0 : baseOpacity;

    visibleCursors.push({
      clientID,
      head,
      anchor,
      name,
      color,
      lastUpdate: cursor.lastUpdate,
      opacity,
      baseOpacity,
      onSameLine,
    });
  });

  // Group cursors by rendered line (Y position)
  const lineGroups = new Map<number, CursorForMerge[]>();

  if (view) {
    visibleCursors.forEach((cursor) => {
      try {
        const coords = view.coordsAtPos(cursor.head);
        // Round Y to group cursors on same visual line
        const lineY = Math.round(coords.top / LINE_THRESHOLD_PX) * LINE_THRESHOLD_PX;

        const existing = lineGroups.get(lineY) || [];
        existing.push(cursor);
        lineGroups.set(lineY, existing);
      } catch {
        // If coordsAtPos fails, put in a separate group
        const existing = lineGroups.get(-1) || [];
        existing.push(cursor);
        lineGroups.set(-1, existing);
      }
    });
  } else {
    // No view available, treat each cursor individually
    visibleCursors.forEach((cursor, idx) => {
      lineGroups.set(idx, [cursor]);
    });
  }

  // Process each line group
  lineGroups.forEach((lineCursors) => {
    // Group by position within this line
    const positionGroups = groupCursorsByPosition(lineCursors);

    // Check if groups overlap and need merging
    const needsMerging =
      positionGroups.length > 1 && checkGroupsOverlap(positionGroups, view);

    if (needsMerging) {
      // Create merged bar for overlapping tooltips
      // Position at the leftmost cursor position
      const leftmostPos = positionGroups[0].position;

      // Create cursor lines for each position (but merged tooltip)
      positionGroups.forEach((group) => {
        // Create cursor line(s) for this position group
        // Use most recent user's color for the cursor line
        const cursorLine = document.createElement("span");
        cursorLine.className = "collab-cursor collab-cursor-merged";
        cursorLine.style.borderColor = group.mostRecentColor;
        cursorLine.setAttribute(
          "data-client-ids",
          group.cursors.map((c) => c.clientID).join(","),
        );

        // Register first cursor for strobe (others share the line)
        const firstCursor = group.cursors[0];
        const strobeDurationMs = getStrobeDurationMs(
          firstCursor.onSameLine ? 1.0 : firstCursor.baseOpacity,
        );
        const shouldStrobe =
          strobeDurationMs > 0 && connectionState === "connected";
        cursorLine.style.setProperty(
          "--strobe-duration",
          `${strobeDurationMs}ms`,
        );
        cursorLine.style.setProperty(
          "--strobe-state",
          shouldStrobe ? "running" : "paused",
        );
        cursorLine.style.setProperty(
          "--base-opacity",
          String(firstCursor.opacity),
        );
        cursorLine.style.opacity = String(firstCursor.opacity);

        if (firstCursor.onSameLine) {
          cursorLine.classList.add("collab-cursor-same-line");
        }

        // Register all cursors in this group for strobe tracking
        group.cursors.forEach((cursor) => {
          strobeInfos.set(cursor.clientID, {
            element: cursorLine,
            baseOpacity: cursor.opacity,
            paused: false,
          });
        });

        decorations.push(
          Decoration.widget(group.position, cursorLine, {
            side: 1,
            key: `cursor-merged-${group.position}`,
          }),
        );
      });

      // Create merged tooltip bar at leftmost position
      const mergedBar = createMergedBar(
        positionGroups,
        handleCursorMouseEnter,
        handleCursorMouseLeave,
      );

      // Create a container for the bar (positioned at leftmost cursor)
      const barContainer = document.createElement("span");
      barContainer.className = "collab-cursor-bar-container";
      barContainer.appendChild(mergedBar);

      decorations.push(
        Decoration.widget(leftmostPos, barContainer, {
          side: 1,
          key: `cursor-bar-${leftmostPos}`,
        }),
      );
    } else {
      // No merging needed - create individual cursor decorations
      positionGroups.forEach((group) => {
        group.cursors.forEach((cursor) => {
          // Create cursor line decoration (widget)
          const cursorWidget = document.createElement("span");
          cursorWidget.className = "collab-cursor";
          if (cursor.onSameLine) {
            cursorWidget.classList.add("collab-cursor-same-line");
          }
          cursorWidget.style.borderColor = cursor.color;
          cursorWidget.setAttribute("data-client-id", String(cursor.clientID));

          // Set up CSS custom properties for strobe animation
          const effectiveOpacityForStrobe = cursor.onSameLine
            ? 1.0
            : cursor.baseOpacity;
          const strobeDurationMs = getStrobeDurationMs(effectiveOpacityForStrobe);
          const shouldStrobe =
            strobeDurationMs > 0 && connectionState === "connected";
          cursorWidget.style.setProperty(
            "--strobe-duration",
            `${strobeDurationMs}ms`,
          );
          cursorWidget.style.setProperty(
            "--strobe-state",
            shouldStrobe ? "running" : "paused",
          );
          cursorWidget.style.setProperty(
            "--base-opacity",
            String(cursor.opacity),
          );
          cursorWidget.style.opacity = String(cursor.opacity);

          // Register for strobe tracking
          strobeInfos.set(cursor.clientID, {
            element: cursorWidget,
            baseOpacity: cursor.opacity,
            paused: false,
          });

          const label = document.createElement("span");
          label.className = "collab-cursor-label";
          label.style.backgroundColor = cursor.color;
          label.style.color = isLightColor(cursor.color) ? "#000" : "#fff";
          label.textContent = cursor.name;

          // Add hover event listeners for interactive cursor behavior
          label.addEventListener("mouseenter", () =>
            handleCursorMouseEnter(cursor.clientID),
          );
          label.addEventListener("mouseleave", () =>
            handleCursorMouseLeave(cursor.clientID),
          );

          cursorWidget.appendChild(label);

          decorations.push(
            Decoration.widget(cursor.head, cursorWidget, {
              side: 1,
              key: `cursor-${cursor.clientID}`,
            }),
          );
        });
      });
    }

    // Add selection decorations for all cursors on this line
    lineCursors.forEach((cursor) => {
      if (cursor.head !== cursor.anchor) {
        const from = Math.min(cursor.head, cursor.anchor);
        const to = Math.max(cursor.head, cursor.anchor);
        const selectionOpacity = Math.round(0x80 * cursor.opacity)
          .toString(16)
          .padStart(2, "0");
        decorations.push(
          Decoration.inline(
            from,
            to,
            {
              class: "collab-selection",
              style: `background-color: ${cursor.color}${selectionOpacity};`,
            },
            {
              key: `selection-${cursor.clientID}`,
            },
          ),
        );
      }
    });
  });

  return DecorationSet.create(state.doc, decorations);
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
  sendCursor: SendCursorFn,
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

      apply(
        tr: Transaction,
        pluginState: CursorPluginState,
      ): CursorPluginState {
        // Check for cursor update meta
        const cursorUpdate = tr.getMeta(cursorPluginKey);
        if (cursorUpdate) {
          const newCursors = new Map(pluginState.cursors);
          if (cursorUpdate.type === "update") {
            // If idleSecs is provided (initial load), backdate lastUpdate
            const lastUpdate = cursorUpdate.idleSecs
              ? Date.now() - cursorUpdate.idleSecs * 1000
              : Date.now();
            newCursors.set(cursorUpdate.clientID, {
              clientID: cursorUpdate.clientID,
              head: cursorUpdate.head,
              anchor: cursorUpdate.anchor,
              name: cursorUpdate.name,
              color: cursorUpdate.color,
              lastUpdate,
            });
          } else if (cursorUpdate.type === "remove") {
            newCursors.delete(cursorUpdate.clientID);
            unregisterCursorStrobe(cursorUpdate.clientID);
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
        // Use cleanupEditorView for same-line detection (may be null during first render)
        return createCursorDecorations(
          state,
          pluginState.cursors,
          myClientID,
          cleanupEditorView,
        );
      },
    },

    view(editorView: EditorView) {
      // Store view for cleanup operations
      setCleanupEditorView(editorView);

      // Send cursor position on selection change
      const sendSelectionUpdate = (): void => {
        const { head, anchor } = editorView.state.selection;

        // Only send if changed
        if (
          head === lastSentSelection.head &&
          anchor === lastSentSelection.anchor
        ) {
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
      refreshInterval = setInterval(
        refreshCursorDecorations,
        REFRESH_INTERVAL_MS,
      );

      // Handle visibility changes to refresh cursors when tab becomes active
      // This is crucial because setInterval is heavily throttled in background tabs
      const handleVisibilityChange = (): void => {
        if (document.visibilityState === "visible") {
          // Tab became visible - immediately refresh cursor decorations
          // This catches up on any fading that should have happened while tab was inactive
          refreshCursorDecorations();
        }
      };
      document.addEventListener("visibilitychange", handleVisibilityChange);

      return {
        update(view: EditorView, prevState: EditorState): void {
          if (!view.state.selection.eq(prevState.selection)) {
            sendSelectionUpdate();
          }
        },
        destroy(): void {
          if (sendTimeout) clearTimeout(sendTimeout);
          if (refreshInterval) clearInterval(refreshInterval);
          if (reconnectCleanupTimer) clearTimeout(reconnectCleanupTimer);
          document.removeEventListener(
            "visibilitychange",
            handleVisibilityChange,
          );
          cleanupEditorView = null;
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
  idleSecs?: number,
): void {
  const tr = view.state.tr.setMeta(cursorPluginKey, {
    type: "update",
    clientID,
    head,
    anchor,
    name,
    idleSecs,
  });
  view.dispatch(tr);
}

/**
 * Remove a remote cursor from the editor (internal use).
 */
function removeCursorInternal(
  view: EditorView,
  clientID: string | number,
): void {
  const tr = view.state.tr.setMeta(cursorPluginKey, {
    type: "remove",
    clientID,
  });
  view.dispatch(tr);
}

/**
 * Remove a remote cursor from the editor.
 */
export function removeCursor(
  view: EditorView,
  clientID: string | number,
): void {
  removeCursorInternal(view, clientID);
  unregisterCursorStrobe(clientID);
}
