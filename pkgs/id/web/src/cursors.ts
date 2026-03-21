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
 *
 * Architecture note: Per-view state (strobeInfos, hover timers) is stored in a
 * WeakMap keyed by EditorView to support multiple editors without interference.
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
const RECONNECT_CLEANUP_DELAY_MS = 3_000; // Wait 3s after Init before cleanup (allow slow networks)
const HOVER_RESTORE_DELAY_MS = 1_000; // 1s delay before restoring after hover

export interface CursorInfo {
  clientID: string | number;
  head: number;
  anchor: number;
  name?: string;
  color?: string;
  lastUpdate: number;
}

export const cursorPluginKey = new PluginKey<CursorPluginState>("cursors");

/**
 * Plugin state - serializable cursor data managed by ProseMirror transactions.
 */
interface CursorPluginState {
  cursors: Map<string | number, CursorInfo>;
  /** Fresh cursor IDs received since last Init (for reconnect cleanup) */
  freshCursorIDs: Set<string | number>;
  /** Connection state affects strobe animations */
  connectionState: "connected" | "disconnected";
}

// ============================================================================
// Per-View State Management
// ============================================================================

/**
 * Per-view instance state (not serializable, contains DOM references).
 * Stored in a WeakMap to support multiple editors without interference.
 */
interface CursorViewState {
  strobeInfos: Map<string | number, StrobeInfo>;
  hoverRestoreTimers: Map<string | number, ReturnType<typeof setTimeout>>;
  reconnectCleanupTimer: ReturnType<typeof setTimeout> | null;
  view: EditorView;
}

interface StrobeInfo {
  element: HTMLElement;
  baseOpacity: number;
  paused: boolean; // True when hover-paused
}

/**
 * WeakMap to store per-view state. This ensures:
 * 1. Multiple editors don't share state
 * 2. State is automatically cleaned up when view is garbage collected
 */
const viewStates = new WeakMap<EditorView, CursorViewState>();

/**
 * Get or create view state for an editor view.
 */
function getViewState(view: EditorView): CursorViewState {
  let state = viewStates.get(view);
  if (!state) {
    state = {
      strobeInfos: new Map(),
      hoverRestoreTimers: new Map(),
      reconnectCleanupTimer: null,
      view,
    };
    viewStates.set(view, state);
  }
  return state;
}

/**
 * Clean up view state when editor is destroyed.
 */
function cleanupViewState(view: EditorView): void {
  const state = viewStates.get(view);
  if (state) {
    // Clear all hover timers
    state.hoverRestoreTimers.forEach((timer) => clearTimeout(timer));
    state.hoverRestoreTimers.clear();
    // Clear reconnect timer
    if (state.reconnectCleanupTimer) {
      clearTimeout(state.reconnectCleanupTimer);
    }
    // Clear strobe tracking
    state.strobeInfos.clear();
    viewStates.delete(view);
  }
}

// ============================================================================
// Strobe and Hover State Management
// ============================================================================

/**
 * Handle mouse entering a cursor label.
 * Sets hovered state immediately (100% opacity, no strobe, bolder, elevated z-index).
 */
function handleCursorMouseEnter(
  clientID: string | number,
  viewState: CursorViewState
): void {
  // Clear any pending restore timer
  const timer = viewState.hoverRestoreTimers.get(clientID);
  if (timer) {
    clearTimeout(timer);
    viewState.hoverRestoreTimers.delete(clientID);
  }

  const info = viewState.strobeInfos.get(clientID);
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
function handleCursorMouseLeave(
  clientID: string | number,
  viewState: CursorViewState,
  getConnectionState: () => "connected" | "disconnected"
): void {
  // Start delayed restore
  const timer = setTimeout(() => {
    viewState.hoverRestoreTimers.delete(clientID);
    restoreCursorFromHover(clientID, viewState, getConnectionState);
  }, HOVER_RESTORE_DELAY_MS);

  viewState.hoverRestoreTimers.set(clientID, timer);
}

/**
 * Restore cursor to normal state after hover.
 */
function restoreCursorFromHover(
  clientID: string | number,
  viewState: CursorViewState,
  getConnectionState: () => "connected" | "disconnected"
): void {
  const info = viewState.strobeInfos.get(clientID);
  if (!info) return;

  // Remove hover class
  info.element.classList.remove("collab-cursor-hovered");

  // Resume strobe if connected
  info.paused = false;
  if (getConnectionState() === "connected") {
    const durationMs = getStrobeDurationMs(info.baseOpacity);
    info.element.style.setProperty(
      "--strobe-state",
      durationMs > 0 ? "running" : "paused"
    );
  }
}

/**
 * Clean up hover timer for a cursor being removed.
 */
function cleanupHoverTimer(
  clientID: string | number,
  viewState: CursorViewState
): void {
  const timer = viewState.hoverRestoreTimers.get(clientID);
  if (timer) {
    clearTimeout(timer);
    viewState.hoverRestoreTimers.delete(clientID);
  }
}

/**
 * Remove cursor from strobe tracking and clean up hover state.
 */
function unregisterCursorStrobe(
  clientID: string | number,
  viewState: CursorViewState
): void {
  cleanupHoverTimer(clientID, viewState);
  viewState.strobeInfos.delete(clientID);
}

/**
 * Update strobe state for all cursors when connection state changes.
 */
function updateStrobeStates(
  viewState: CursorViewState,
  connectionState: "connected" | "disconnected"
): void {
  viewState.strobeInfos.forEach((info) => {
    if (connectionState === "disconnected") {
      // Pause strobing, set to base opacity
      info.element.style.setProperty("--strobe-state", "paused");
      info.element.style.opacity = String(info.baseOpacity);
    } else if (!info.paused) {
      // Resume strobing (unless hover-paused)
      const durationMs = getStrobeDurationMs(info.baseOpacity);
      info.element.style.setProperty(
        "--strobe-state",
        durationMs > 0 ? "running" : "paused"
      );
    }
  });
}

// ============================================================================
// Connection State Management
// ============================================================================

/**
 * Set the connection state for a specific editor view.
 * When disconnected, all cursor strobing is paused.
 */
export function setConnectionState(
  view: EditorView,
  state: "connected" | "disconnected"
): void {
  const tr = view.state.tr.setMeta(cursorPluginKey, {
    type: "setConnectionState",
    connectionState: state,
  });
  view.dispatch(tr);
}

// ============================================================================
// Reconnect Cleanup
// ============================================================================

/**
 * Mark a cursor as fresh (received update since Init).
 * Call this when receiving a cursor update from the server.
 */
export function markCursorFresh(
  view: EditorView,
  clientID: string | number
): void {
  const tr = view.state.tr.setMeta(cursorPluginKey, {
    type: "markFresh",
    clientID,
  });
  view.dispatch(tr);
}

/**
 * Called when Init message is received.
 * Resets fresh cursor tracking and schedules stale cursor cleanup.
 */
export function onInitReceived(view: EditorView): void {
  const tr = view.state.tr.setMeta(cursorPluginKey, {
    type: "initReceived",
  });
  view.dispatch(tr);
}

/**
 * Remove cursors that weren't marked fresh after reconnect.
 */
function performReconnectCleanup(view: EditorView): void {
  const pluginState = cursorPluginKey.getState(view.state);
  if (!pluginState) return;

  const viewState = getViewState(view);
  const staleCursors: (string | number)[] = [];

  pluginState.cursors.forEach((_, clientID) => {
    if (!pluginState.freshCursorIDs.has(clientID)) {
      staleCursors.push(clientID);
    }
  });

  // Remove stale cursors
  staleCursors.forEach((clientID) => {
    console.log("[cursors] Removing stale cursor:", clientID);
    removeCursorInternal(view, clientID);
    unregisterCursorStrobe(clientID, viewState);
  });
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
  view: EditorView
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
 * Create a merged tooltip bar for multiple position groups that overlap
 */
function createMergedBar(
  groups: PositionGroup[],
  handleMouseEnter: (clientID: string | number) => void,
  handleMouseLeave: (clientID: string | number) => void
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
        handleMouseEnter(cursor.clientID)
      );
      segment.addEventListener("mouseleave", () =>
        handleMouseLeave(cursor.clientID)
      );

      bar.appendChild(segment);
    });
  });

  return bar;
}

/**
 * Create cursor line element with strobe animation
 */
function createCursorLine(
  cursor: CursorForMerge,
  isMerged: boolean,
  connectionState: "connected" | "disconnected",
  allClientIDs?: (string | number)[]
): HTMLElement {
  const cursorLine = document.createElement("span");
  cursorLine.className = isMerged
    ? "collab-cursor collab-cursor-merged"
    : "collab-cursor";

  if (cursor.onSameLine) {
    cursorLine.classList.add("collab-cursor-same-line");
  }

  cursorLine.style.borderColor = cursor.color;

  if (isMerged && allClientIDs) {
    cursorLine.setAttribute("data-client-ids", allClientIDs.join(","));
  } else {
    cursorLine.setAttribute("data-client-id", String(cursor.clientID));
  }

  // Set up CSS custom properties for strobe animation
  const effectiveOpacity = cursor.onSameLine ? 1.0 : cursor.baseOpacity;
  const strobeDurationMs = getStrobeDurationMs(effectiveOpacity);
  const shouldStrobe = strobeDurationMs > 0 && connectionState === "connected";

  cursorLine.style.setProperty("--strobe-duration", `${strobeDurationMs}ms`);
  cursorLine.style.setProperty(
    "--strobe-state",
    shouldStrobe ? "running" : "paused"
  );
  cursorLine.style.setProperty("--base-opacity", String(cursor.opacity));
  cursorLine.style.opacity = String(cursor.opacity);

  return cursorLine;
}

/**
 * Create standalone cursor with attached label
 */
function createStandaloneCursor(
  cursor: CursorForMerge,
  connectionState: "connected" | "disconnected",
  viewState: CursorViewState,
  handleMouseEnter: (clientID: string | number) => void,
  handleMouseLeave: (clientID: string | number) => void
): HTMLElement {
  const cursorWidget = createCursorLine(cursor, false, connectionState);

  // Register for strobe tracking
  viewState.strobeInfos.set(cursor.clientID, {
    element: cursorWidget,
    baseOpacity: cursor.opacity,
    paused: false,
  });

  const label = document.createElement("span");
  label.className = "collab-cursor-label";
  label.style.backgroundColor = cursor.color;
  label.style.color = isLightColor(cursor.color) ? "#000" : "#fff";
  label.textContent = cursor.name;

  // Add hover event listeners
  label.addEventListener("mouseenter", () => handleMouseEnter(cursor.clientID));
  label.addEventListener("mouseleave", () => handleMouseLeave(cursor.clientID));

  cursorWidget.appendChild(label);
  return cursorWidget;
}

/**
 * Create the cursor decoration for a remote user.
 * @param view - EditorView for same-line detection and view state access
 */
function createCursorDecorations(
  state: EditorState,
  pluginState: CursorPluginState,
  myClientID: string | number | null,
  view: EditorView | null
): DecorationSet {
  const decorations: Decoration[] = [];
  const docSize = state.doc.content.size;
  const now = Date.now();
  const localHead = state.selection.head;

  // Normalize myClientID to number for comparison (handles string/number mismatch)
  const myClientIDNum = myClientID !== null ? Number(myClientID) : null;

  // Get view state for strobe tracking (if view available)
  const viewState = view ? getViewState(view) : null;
  const connectionState = pluginState.connectionState;

  // Create hover handlers bound to this view's state
  const handleMouseEnter = viewState
    ? (clientID: string | number): void =>
        handleCursorMouseEnter(clientID, viewState)
    : (): void => {};
  const handleMouseLeave = viewState
    ? (clientID: string | number): void =>
        handleCursorMouseLeave(clientID, viewState, () => connectionState)
    : (): void => {};

  // First pass: collect all visible cursors with computed properties
  const visibleCursors: CursorForMerge[] = [];

  pluginState.cursors.forEach((cursor, clientID) => {
    // Normalize clientID for comparison
    const clientIDNum = Number(clientID);

    // Check if this is our own cursor
    const isOwnCursor = myClientIDNum !== null && clientIDNum === myClientIDNum;

    const ageMs = now - cursor.lastUpdate;

    // Hide completely after HIDE_MS (but not own cursor)
    if (!isOwnCursor && ageMs >= HIDE_MS) return;

    // Get base opacity (own cursor is always full opacity)
    const baseOpacity = isOwnCursor ? 1.0 : getOpacityForAge(ageMs);

    // Skip if base opacity is 0 (hidden) - doesn't apply to own cursor
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
      isOwnCursor,
    });
  });

  // Separate own cursor from remote cursors
  const ownCursor = visibleCursors.find((c) => c.isOwnCursor);
  const remoteCursors = visibleCursors.filter((c) => !c.isOwnCursor);

  // Create own cursor decoration (larger line, no tooltip)
  if (ownCursor && view) {
    const ownCursorLine = document.createElement("span");
    ownCursorLine.className = "collab-cursor collab-cursor-own";
    ownCursorLine.style.borderColor = ownCursor.color;
    ownCursorLine.setAttribute("data-client-id", String(ownCursor.clientID));

    decorations.push(
      Decoration.widget(ownCursor.head, ownCursorLine, {
        side: 1,
        key: `cursor-own-${ownCursor.clientID}`,
      })
    );
  }

  // Group remote cursors by rendered line (Y position)
  const lineGroups = new Map<number, CursorForMerge[]>();

  if (view) {
    remoteCursors.forEach((cursor) => {
      try {
        const coords = view.coordsAtPos(cursor.head);
        // Round Y to group cursors on same visual line
        const lineY =
          Math.round(coords.top / LINE_THRESHOLD_PX) * LINE_THRESHOLD_PX;

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
    remoteCursors.forEach((cursor, idx) => {
      lineGroups.set(idx, [cursor]);
    });
  }

  // Helper to get left coordinate
  const getLeftCoord = view
    ? (pos: number): number => view.coordsAtPos(pos).left
    : null;

  // Process each line group
  lineGroups.forEach((lineCursors) => {
    // Group by position within this line
    const positionGroups = groupCursorsByPosition(lineCursors);

    // Cluster overlapping groups - only merge those that actually overlap
    const clusters = clusterOverlappingGroups(positionGroups, getLeftCoord);

    clusters.forEach((cluster) => {
      if (cluster.groups.length > 1) {
        // Multiple groups overlap - create merged bar attached to leftmost cursor
        const leftmostPos = cluster.leftmostPosition;

        // Find the leftmost group (we'll attach the bar to its cursor line)
        const leftmostGroup = cluster.groups.find((g) => g.position === leftmostPos);
        let leftmostCursorLine: HTMLElement | null = null;

        // Create cursor lines for each position group (no attached labels)
        cluster.groups.forEach((group) => {
          const firstCursor = group.cursors[0];
          const allClientIDs = group.cursors.map((c) => c.clientID);
          const cursorLine = createCursorLine(
            firstCursor,
            true,
            connectionState,
            allClientIDs
          );
          cursorLine.style.borderColor = group.mostRecentColor;

          // Track the leftmost cursor line for bar attachment
          if (group === leftmostGroup) {
            leftmostCursorLine = cursorLine;
          }

          // Register all cursors in this group for strobe tracking
          if (viewState) {
            group.cursors.forEach((cursor) => {
              viewState.strobeInfos.set(cursor.clientID, {
                element: cursorLine,
                baseOpacity: cursor.opacity,
                paused: false,
              });
            });
          }

          decorations.push(
            Decoration.widget(group.position, cursorLine, {
              side: 1,
              key: `cursor-merged-${group.position}`,
            })
          );
        });

        // Create merged tooltip bar and attach to leftmost cursor line
        // (appending to cursor line ensures proper positioning via position: relative)
        const mergedBar = createMergedBar(
          cluster.groups,
          handleMouseEnter,
          handleMouseLeave
        );

        if (leftmostCursorLine) {
          (leftmostCursorLine as HTMLElement).appendChild(mergedBar);
        }
      } else {
        // Single group (no overlap with other groups)
        const group = cluster.groups[0];

        if (group.cursors.length === 1) {
          // Single cursor at this position - simple case
          const cursor = group.cursors[0];
          const cursorWidget = viewState
            ? createStandaloneCursor(
                cursor,
                connectionState,
                viewState,
                handleMouseEnter,
                handleMouseLeave
              )
            : createCursorLine(cursor, false, connectionState);

          decorations.push(
            Decoration.widget(cursor.head, cursorWidget, {
              side: 1,
              key: `cursor-${cursor.clientID}`,
            })
          );
        } else {
          // Multiple cursors at exact same position - create single cursor line with merged bar
          // Use most recent cursor's color for the line
          const firstCursor = group.cursors[0]; // Already sorted by lastUpdate desc
          const allClientIDs = group.cursors.map((c) => c.clientID);
          const cursorLine = createCursorLine(
            firstCursor,
            true,
            connectionState,
            allClientIDs
          );
          cursorLine.style.borderColor = group.mostRecentColor;

          // Register all cursors for strobe tracking
          if (viewState) {
            group.cursors.forEach((cursor) => {
              viewState.strobeInfos.set(cursor.clientID, {
                element: cursorLine,
                baseOpacity: cursor.opacity,
                paused: false,
              });
            });
          }

          // Create merged bar for same-position cursors and append to cursor line
          // (appending to cursor line ensures proper positioning via position: relative)
          const mergedBar = createMergedBar(
            [group], // Single group, so no dividers
            handleMouseEnter,
            handleMouseLeave
          );
          cursorLine.appendChild(mergedBar);

          decorations.push(
            Decoration.widget(group.position, cursorLine, {
              side: 1,
              key: `cursor-merged-${group.position}`,
            })
          );
        }
      }
    });

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
            }
          )
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
  sendCursor: SendCursorFn
): Plugin {
  let lastSentSelection = { head: -1, anchor: -1 };
  let sendTimeout: ReturnType<typeof setTimeout> | null = null;
  let refreshInterval: ReturnType<typeof setInterval> | null = null;
  let currentView: EditorView | null = null;

  return new Plugin<CursorPluginState>({
    key: cursorPluginKey,

    state: {
      init(): CursorPluginState {
        return {
          cursors: new Map(),
          freshCursorIDs: new Set(),
          connectionState: "disconnected",
        };
      },

      apply(
        tr: Transaction,
        pluginState: CursorPluginState
      ): CursorPluginState {
        // Check for cursor update meta
        const cursorUpdate = tr.getMeta(cursorPluginKey);
        if (cursorUpdate) {
          switch (cursorUpdate.type) {
            case "update": {
              const newCursors = new Map(pluginState.cursors);
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
              return { ...pluginState, cursors: newCursors };
            }

            case "remove": {
              const newCursors = new Map(pluginState.cursors);
              newCursors.delete(cursorUpdate.clientID);
              // Note: strobe cleanup is handled by the caller
              return { ...pluginState, cursors: newCursors };
            }

            case "setConnectionState": {
              const newState = cursorUpdate.connectionState as
                | "connected"
                | "disconnected";
              // Update strobe states for all cursors
              if (currentView) {
                const viewState = getViewState(currentView);
                updateStrobeStates(viewState, newState);

                // Cancel reconnect cleanup timer on disconnect
                if (
                  newState === "disconnected" &&
                  viewState.reconnectCleanupTimer
                ) {
                  clearTimeout(viewState.reconnectCleanupTimer);
                  viewState.reconnectCleanupTimer = null;
                }
              }
              return { ...pluginState, connectionState: newState };
            }

            case "markFresh": {
              const newFreshIDs = new Set(pluginState.freshCursorIDs);
              newFreshIDs.add(cursorUpdate.clientID);
              return { ...pluginState, freshCursorIDs: newFreshIDs };
            }

            case "initReceived": {
              // Reset fresh cursor tracking and schedule cleanup
              if (currentView) {
                const viewState = getViewState(currentView);
                const view = currentView;

                // Cancel any existing cleanup timer
                if (viewState.reconnectCleanupTimer) {
                  clearTimeout(viewState.reconnectCleanupTimer);
                }

                // Start cleanup timer
                viewState.reconnectCleanupTimer = setTimeout(() => {
                  const currentPluginState = cursorPluginKey.getState(
                    view.state
                  );
                  if (
                    currentPluginState &&
                    currentPluginState.connectionState === "connected"
                  ) {
                    performReconnectCleanup(view);
                  }
                  viewState.reconnectCleanupTimer = null;
                }, RECONNECT_CLEANUP_DELAY_MS);
              }
              return { ...pluginState, freshCursorIDs: new Set() };
            }

            default:
              return pluginState;
          }
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
          return { ...pluginState, cursors: newCursors };
        }

        return pluginState;
      },
    },

    props: {
      decorations(state: EditorState): DecorationSet {
        const pluginState = cursorPluginKey.getState(state);
        if (!pluginState) return DecorationSet.empty;
        return createCursorDecorations(
          state,
          pluginState,
          myClientID,
          currentView
        );
      },
    },

    view(editorView: EditorView) {
      // Store view reference for use in decorations and state transitions
      currentView = editorView;
      const viewState = getViewState(editorView);

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
        REFRESH_INTERVAL_MS
      );

      // Handle visibility changes to refresh cursors when tab becomes active
      // This is crucial because setInterval is heavily throttled in background tabs
      const handleVisibilityChange = (): void => {
        if (document.visibilityState === "visible") {
          // Tab became visible - immediately refresh cursor decorations
          // This catches up on any fading that should have happened while tab was inactive
          refreshCursorDecorations();
          
          // Also re-send our cursor position to trigger a sync with other clients
          // This helps ensure our cursor is visible to others after being inactive
          const { head, anchor } = editorView.state.selection;
          // Reset lastSentSelection to force re-send
          lastSentSelection = { head: -1, anchor: -1 };
          sendSelectionUpdate();
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
          document.removeEventListener(
            "visibilitychange",
            handleVisibilityChange
          );
          cleanupViewState(editorView);
          currentView = null;
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
  clientID: string | number
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
  clientID: string | number
): void {
  const viewState = getViewState(view);
  removeCursorInternal(view, clientID);
  unregisterCursorStrobe(clientID, viewState);
}
