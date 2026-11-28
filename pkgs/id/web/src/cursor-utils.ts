/**
 * Cursor utility functions - pure functions extracted for testability.
 * These functions have no side effects and can be unit tested in isolation.
 */

// ============================================================================
// Timing Constants
// ============================================================================

export const FADE_START_MS = 30_000; // Start fading after 30s of inactivity
export const FADE_END_MS = 60_000; // Fully faded at 60s
export const HIDE_MS = 300_000; // Hide completely after 5 minutes (300s)
export const LINE_THRESHOLD_PX = 5; // Cursors within this many pixels are "same line"

// ============================================================================
// Opacity Calculation
// ============================================================================

/**
 * Calculate opacity based on time since last update.
 * Returns 1.0 for active, fades to 0.3, then 0 after HIDE_MS.
 *
 * Timeline:
 * - 0 to 30s: opacity 1.0 (fully visible)
 * - 30s to 60s: linear fade from 1.0 to 0.3
 * - 60s to 5m: opacity 0.3 (minimum visible)
 * - 5m+: opacity 0 (hidden)
 */
export function getOpacityForAge(ageMs: number): number {
  if (ageMs < FADE_START_MS) return 1.0;
  if (ageMs >= HIDE_MS) return 0;
  if (ageMs >= FADE_END_MS) return 0.3;

  // Linear fade from 1.0 to 0.3 between FADE_START and FADE_END
  const fadeProgress = (ageMs - FADE_START_MS) / (FADE_END_MS - FADE_START_MS);
  return 1.0 - fadeProgress * 0.7;
}

// ============================================================================
// Strobe Duration Calculation
// ============================================================================

/**
 * Calculate strobe duration based on opacity.
 * Returns 0 if strobing should be disabled.
 *
 * Formula: duration_ms = 1000 + ((1.0 - opacity) / 0.7) * 2000
 * - opacity 1.0 → 1000ms (fast strobe)
 * - opacity 0.65 → 2000ms (medium)
 * - opacity 0.3 → 3000ms (slow)
 * - opacity ≤0.3 → 0 (no strobe)
 */
export function getStrobeDurationMs(opacity: number): number {
  if (opacity <= 0.3) return 0;
  return 1000 + ((1.0 - opacity) / 0.7) * 2000;
}

// ============================================================================
// Color Utilities
// ============================================================================

/**
 * Determine if a hex color is light (for choosing text color).
 * Uses relative luminance calculation.
 */
export function isLightColor(color: string): boolean {
  let hex = color.replace("#", "");
  // CSS shorthand hex (#fff) doubles each digit; without expansion the
  // slices below misalign and the last component parses as NaN (#846).
  if (/^[0-9a-fA-F]{3}$/.test(hex)) {
    hex = hex
      .split("")
      .map((digit) => digit + digit)
      .join("");
  }
  // Anything that is not exactly six hex digits is not a color we can
  // judge; treat it as dark rather than letting a NaN luminance decide.
  if (!/^[0-9a-fA-F]{6}$/.test(hex)) return false;
  const r = parseInt(hex.slice(0, 2), 16);
  const g = parseInt(hex.slice(2, 4), 16);
  const b = parseInt(hex.slice(4, 6), 16);
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return luminance > 0.5;
}

// Colors for different users - high contrast, deterministic by client ID hash
// Selected for visibility against both light and dark backgrounds
export const CURSOR_COLORS = [
  // Reds/Oranges
  "#e63946", // red
  "#f77f00", // orange
  "#ff006e", // magenta-red
  // Yellows/Greens
  "#ffbe0b", // yellow
  "#06d6a0", // mint-green
  "#2a9d8f", // teal
  "#38b000", // green
  // Blues
  "#00b4d8", // cyan
  "#0077b6", // blue
  "#3a86ff", // bright blue
  // Purples/Pinks
  "#8338ec", // purple
  "#c77dff", // lavender
  "#ff5da2", // pink
  "#9d4edd", // violet
  // Earth tones
  "#fb8500", // amber
  "#d62828", // crimson
];

/**
 * Get a consistent color for a client ID.
 * Uses a hash function to map client IDs to colors.
 */
export function getColorForClient(clientID: string | number): string {
  // A plain loop: no intermediate array of characters per call.
  const str = String(clientID);
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    hash = ((hash << 5) - hash + str.charCodeAt(i)) | 0;
  }
  return CURSOR_COLORS[Math.abs(hash) % CURSOR_COLORS.length];
}

// ============================================================================
// Cursor Grouping for Tooltip Stacking
// ============================================================================

/**
 * Data structure for cursor merging calculations
 */
export interface CursorForMerge {
  clientID: string | number;
  head: number;
  anchor: number;
  name: string;
  color: string;
  lastUpdate: number;
  opacity: number;
  baseOpacity: number;
  onSameLine: boolean;
  isOwnCursor?: boolean;
  /** Resolved #1790 tooltip display for this cursor, set by cursors.ts. */
  tooltipDisplay?: TooltipDisplay;
}

/**
 * Group of cursors at the same document position
 */
export interface PositionGroup {
  position: number;
  cursors: CursorForMerge[]; // Sorted by lastUpdate desc (most recent first)
  mostRecentColor: string;
}

/**
 * Estimate tooltip width in pixels.
 * Used for overlap detection.
 */
export function estimateTooltipWidth(name: string): number {
  const CHAR_WIDTH = 6.3; // Approximate width per character at 9px monospace font
  return name.length * CHAR_WIDTH;
}

/**
 * Group cursors by exact document position and sort appropriately.
 *
 * Rules:
 * - Cursors at same position are grouped together
 * - Within each group, cursors are sorted by lastUpdate (most recent first)
 * - Groups are sorted by position ascending
 */
export function groupCursorsByPosition(cursors: CursorForMerge[]): PositionGroup[] {
  const positionMap = new Map<number, CursorForMerge[]>();

  // Group by exact position
  cursors.forEach((cursor) => {
    let existing = positionMap.get(cursor.head);
    if (!existing) {
      existing = [];
      positionMap.set(cursor.head, existing);
    }
    existing.push(cursor);
  });

  // Convert to PositionGroup[], sort each group by activity
  const groups: PositionGroup[] = [];
  positionMap.forEach((groupCursors, position) => {
    // Sort by lastUpdate descending (most recent = first/leftmost)
    groupCursors.sort((a, b) => b.lastUpdate - a.lastUpdate);
    groups.push({
      position,
      cursors: groupCursors,
      mostRecentColor: groupCursors[0].color,
    });
  });

  // Sort groups by position ascending
  groups.sort((a, b) => a.position - b.position);

  return groups;
}

/**
 * Check if tooltips in consecutive position groups would overlap.
 * Uses coordinate callback to get pixel positions.
 *
 * @param groups - Position groups sorted by position ascending
 * @param getLeftCoord - Function to get left pixel coordinate for a position
 * @returns true if any consecutive groups would have overlapping tooltips
 */
export function doGroupsOverlap(groups: PositionGroup[], getLeftCoord: ((pos: number) => number) | null): boolean {
  if (groups.length <= 1 || !getLeftCoord) return false;

  const MIN_GAP = 2; // Only merge when tooltips would actually overlap (with 2px buffer)

  for (let i = 0; i < groups.length - 1; i++) {
    const curr = groups[i];
    const next = groups[i + 1];

    // Calculate total width of current group's tooltips
    const currWidth = curr.cursors.reduce((sum, c) => sum + estimateTooltipWidth(c.name), 0);

    // Get pixel distance between positions
    try {
      const currLeft = getLeftCoord(curr.position);
      const nextLeft = getLeftCoord(next.position);
      const posDiff = nextLeft - currLeft;

      if (posDiff < currWidth + MIN_GAP) {
        return true;
      }
    } catch {}
  }
  return false;
}

/**
 * A cluster of position groups that should be merged into a single bar.
 */
export interface MergeCluster {
  groups: PositionGroup[];
  leftmostPosition: number;
}

/**
 * Cluster position groups into merge clusters based on actual overlap.
 * Only groups whose tooltips would actually overlap are merged together.
 * Non-overlapping groups remain as separate single-group clusters.
 *
 * @param groups - Position groups sorted by position ascending
 * @param getLeftCoord - Function to get left pixel coordinate for a position
 * @returns Array of merge clusters
 */
export function clusterOverlappingGroups(
  groups: PositionGroup[],
  getLeftCoord: ((pos: number) => number) | null,
): MergeCluster[] {
  if (groups.length === 0) return [];
  if (groups.length === 1 || !getLeftCoord) {
    // Each group is its own cluster
    return groups.map((g) => ({
      groups: [g],
      leftmostPosition: g.position,
    }));
  }

  const MIN_GAP = 2; // Only merge when tooltips would actually overlap (with 2px buffer)
  const clusters: MergeCluster[] = [];
  let currentCluster: PositionGroup[] = [groups[0]];
  let clusterRightEdge = 0;

  // Calculate initial right edge of first group's tooltip
  try {
    const firstLeft = getLeftCoord(groups[0].position);
    const firstWidth = groups[0].cursors.reduce((sum, c) => sum + estimateTooltipWidth(c.name), 0);
    clusterRightEdge = firstLeft + firstWidth;
  } catch {
    // If coords fail, put everything in separate clusters
    return groups.map((g) => ({
      groups: [g],
      leftmostPosition: g.position,
    }));
  }

  for (let i = 1; i < groups.length; i++) {
    const group = groups[i];

    try {
      const groupLeft = getLeftCoord(group.position);
      const groupWidth = group.cursors.reduce((sum, c) => sum + estimateTooltipWidth(c.name), 0);

      // Check if this group overlaps with the current cluster      // Check if this group overlaps with the current cluster
      if (groupLeft < clusterRightEdge + MIN_GAP) {
        // Overlaps - add to current cluster
        currentCluster.push(group);
        // Extend cluster right edge
        clusterRightEdge = Math.max(clusterRightEdge, groupLeft + groupWidth);
      } else {
        // No overlap - finish current cluster and start new one
        clusters.push({
          groups: currentCluster,
          leftmostPosition: currentCluster[0].position,
        });
        currentCluster = [group];
        clusterRightEdge = groupLeft + groupWidth;
      }
    } catch {
      // If coords fail, put this group in its own cluster
      clusters.push({
        groups: currentCluster,
        leftmostPosition: currentCluster[0].position,
      });
      currentCluster = [group];
      try {
        const left = getLeftCoord(group.position);
        const width = group.cursors.reduce((sum, c) => sum + estimateTooltipWidth(c.name), 0);
        clusterRightEdge = left + width;
      } catch {
        clusterRightEdge = 0;
      }
    }
  }

  // Don't forget the last cluster
  clusters.push({
    groups: currentCluster,
    leftmostPosition: currentCluster[0].position,
  });

  return clusters;
}

// ============================================================================
// Tooltip Visibility Modes & Per-Cursor Display State (#1790, slice 1)
// ============================================================================
//
// Pure state machine for the cursor-presence remainder settled on #1790.
// This slice is logic only: cursors.ts wiring (slice 2) and lifecycle
// logging (slice 3) consume these functions. Oracle for every rule is the
// #1790 issue text:
// - modes: full / minimal / hidden. Minimal = cursor only, tooltip on
//   hover >= 1s. Hidden = tracked but not shown, pinned cursors exempt.
// - click-to-enlarge runs a 1s revert chain: big -> tooltip -> minimal,
//   one step per ENLARGE_STEP_MS; "minimal" is the chain's resting end
//   state until the caller clears `enlargedAt` on the next interaction.
// - minimize is an explicit per-cursor action: a minimized cursor is not
//   shown in any mode (pin exempts only from the global hidden mode, not
//   from an explicit minimize) and is restorable from the minimized list
//   produced by partitionCursorsByMinimized.

export type TooltipMode = "full" | "minimal" | "hidden";

/** What one cursor should render as, after mode/pin/minimize/enlarge. */
export type TooltipDisplay = "big" | "tooltip" | "minimal" | "hidden";

/** Minimal mode: the tooltip appears only after hovering this long. */
export const HOVER_TOOLTIP_DELAY_MS = 1_000;

/** Click-to-enlarge: each step of the revert chain lasts this long. */
export const ENLARGE_STEP_MS = 1_000;

export interface CursorDisplayState {
  pinned: boolean;
  minimized: boolean;
  /** Epoch ms of the click that enlarged the tooltip, or null. */
  enlargedAt: number | null;
  /** Epoch ms when hover started, or null when not hovering. */
  hoverSince: number | null;
}

export const DEFAULT_CURSOR_DISPLAY_STATE: CursorDisplayState = {
  pinned: false,
  minimized: false,
  enlargedAt: null,
  hoverSince: null,
};

export type CursorDisplayStateMap = ReadonlyMap<string | number, CursorDisplayState>;

/** The mode toggle cycles full -> minimal -> hidden -> full. */
export function nextTooltipMode(mode: TooltipMode): TooltipMode {
  if (mode === "full") return "minimal";
  if (mode === "minimal") return "hidden";
  return "full";
}

/**
 * Stage of the click-to-enlarge revert chain at `now`.
 * Returns null when the cursor was never enlarged.
 * Boundaries: [0,1s) big, [1s,2s) tooltip, >=2s minimal.
 * A negative elapsed time (clock skew) reads as "big" (just enlarged).
 */
export function getEnlargeStage(enlargedAt: number | null, now: number): TooltipDisplay | null {
  if (enlargedAt === null) return null;
  const elapsed = now - enlargedAt;
  if (elapsed < ENLARGE_STEP_MS) return "big";
  if (elapsed < 2 * ENLARGE_STEP_MS) return "tooltip";
  return "minimal";
}

export interface TooltipVisibilityInput {
  mode: TooltipMode;
  pinned?: boolean;
  minimized?: boolean;
  enlargedAt?: number | null;
  hoverSince?: number | null;
  now: number;
}

/**
 * Resolve what one cursor renders as. Precedence (first match wins):
 * 1. hidden mode without a pin -> "hidden" (tracked, not shown)
 * 2. explicitly minimized -> "hidden" (restorable from the minimized list)
 * 3. an active enlarge chain -> its stage (big / tooltip / minimal)
 * 4. minimal mode -> "tooltip" once hover reaches HOVER_TOOLTIP_DELAY_MS,
 *    otherwise "minimal" (cursor only)
 * 5. full mode (and pinned cursors exempted from hidden mode) -> "tooltip"
 */
export function resolveTooltipDisplay(input: TooltipVisibilityInput): TooltipDisplay {
  const pinned = input.pinned ?? false;
  const minimized = input.minimized ?? false;
  const enlargedAt = input.enlargedAt ?? null;
  const hoverSince = input.hoverSince ?? null;

  if (input.mode === "hidden" && !pinned) return "hidden";
  if (minimized) return "hidden";

  const stage = getEnlargeStage(enlargedAt, input.now);
  if (stage !== null) return stage;

  if (input.mode === "minimal") {
    return hoverSince !== null && input.now - hoverSince >= HOVER_TOOLTIP_DELAY_MS ? "tooltip" : "minimal";
  }
  return "tooltip";
}

/** Read one cursor's display state, defaulting to the resting state. */
export function getCursorDisplayState(states: CursorDisplayStateMap, clientID: string | number): CursorDisplayState {
  return states.get(clientID) ?? DEFAULT_CURSOR_DISPLAY_STATE;
}

function withCursorDisplayState(
  states: CursorDisplayStateMap,
  clientID: string | number,
  patch: Partial<CursorDisplayState>,
): Map<string | number, CursorDisplayState> {
  const next = new Map(states);
  next.set(clientID, { ...getCursorDisplayState(states, clientID), ...patch });
  return next;
}

/** Pin/unpin one cursor (pin lives on the enlarged tooltip in the UI). */
export function setCursorPinned(
  states: CursorDisplayStateMap,
  clientID: string | number,
  pinned: boolean,
): Map<string | number, CursorDisplayState> {
  return withCursorDisplayState(states, clientID, { pinned });
}

/** Minimize/restore one cursor. Minimized cursors form the restore list. */
export function setCursorMinimized(
  states: CursorDisplayStateMap,
  clientID: string | number,
  minimized: boolean,
): Map<string | number, CursorDisplayState> {
  return withCursorDisplayState(states, clientID, { minimized });
}

/**
 * Split cursors into the visible set and the minimized (restore) list,
 * preserving input order in both. "Minimized" here is the explicit
 * per-cursor flag only; mode-based hiding is resolveTooltipDisplay's job.
 */
export function partitionCursorsByMinimized<T extends { clientID: string | number }>(
  cursors: T[],
  states: CursorDisplayStateMap,
): { visible: T[]; minimized: T[] } {
  const visible: T[] = [];
  const minimized: T[] = [];
  for (const cursor of cursors) {
    if (getCursorDisplayState(states, cursor.clientID).minimized) {
      minimized.push(cursor);
    } else {
      visible.push(cursor);
    }
  }
  return { visible, minimized };
}

// ============================================================================
// Cursor Lifecycle Logging (#1790, slice 3)
// ============================================================================
//
// The lifecycle events the issue names: a cursor going inactive for the
// first time (age reaches FADE_START_MS), reaching the fade minimum
// (FADE_END_MS), being removed, and hover/unhover. Age is measured from
// the cursor's last relayed update (lastUpdate): cursor frames arrive over
// the websocket, and removal is driven by the server's connection
// liveness (socket close / pong timeout, see pkgs/id/src/web/collab.rs),
// never by local pointer idleness.

export type CursorLifecycleEvent = "first-inactive" | "minimum-reached" | "removed" | "hover" | "unhover";

/**
 * Age-based lifecycle events that should be logged for a cursor of age
 * `ageMs`, given the events already logged for it. Pure: the caller owns
 * the `alreadyLogged` set and the actual log sink.
 */
export function lifecycleEventsForAge(
  ageMs: number,
  alreadyLogged: ReadonlySet<CursorLifecycleEvent>,
): CursorLifecycleEvent[] {
  const events: CursorLifecycleEvent[] = [];
  if (ageMs >= FADE_START_MS && !alreadyLogged.has("first-inactive")) {
    events.push("first-inactive");
  }
  if (ageMs >= FADE_END_MS && !alreadyLogged.has("minimum-reached")) {
    events.push("minimum-reached");
  }
  return events;
}

/** One structured lifecycle log line (client console and tests share it). */
export function formatCursorLifecycleLog(
  event: CursorLifecycleEvent,
  clientID: string | number,
  name?: string,
): string {
  return `[cursor-lifecycle] ${event} client=${String(clientID)}${name ? ` name=${name}` : ""}`;
}
