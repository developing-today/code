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
  const hex = color.replace("#", "");
  const r = parseInt(hex.slice(0, 2), 16);
  const g = parseInt(hex.slice(2, 4), 16);
  const b = parseInt(hex.slice(4, 6), 16);
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return luminance > 0.5;
}

// Colors for different users (cycles through these)
export const CURSOR_COLORS = [
  "#ff6b6b", // red
  "#4ecdc4", // teal
  "#ffe66d", // yellow
  "#95e1d3", // mint
  "#a29bfe", // purple
  "#fd79a8", // pink
  "#00b894", // green
  "#e17055", // orange
];

/**
 * Get a consistent color for a client ID.
 * Uses a hash function to map client IDs to colors.
 */
export function getColorForClient(clientID: string | number): string {
  const hash = String(clientID)
    .split("")
    .reduce((a, b) => {
      return ((a << 5) - a + b.charCodeAt(0)) | 0;
    }, 0);
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
  const CHAR_WIDTH = 6; // Approximate width per character at 10px font
  const PADDING = 0; // No padding (CSS has padding: 0)
  return name.length * CHAR_WIDTH + PADDING;
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
    const existing = positionMap.get(cursor.head) || [];
    existing.push(cursor);
    positionMap.set(cursor.head, existing);
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
export function doGroupsOverlap(
  groups: PositionGroup[],
  getLeftCoord: ((pos: number) => number) | null,
): boolean {
  if (groups.length <= 1 || !getLeftCoord) return false;

  const MIN_GAP = -20; // Only merge when actually overlapping by at least 20px

  for (let i = 0; i < groups.length - 1; i++) {
    const curr = groups[i];
    const next = groups[i + 1];

    // Calculate total width of current group's tooltips
    const currWidth = curr.cursors.reduce(
      (sum, c) => sum + estimateTooltipWidth(c.name),
      0,
    );

    // Get pixel distance between positions
    try {
      const currLeft = getLeftCoord(curr.position);
      const nextLeft = getLeftCoord(next.position);
      const posDiff = nextLeft - currLeft;

      if (posDiff < currWidth + MIN_GAP) {
        return true;
      }
    } catch {
      // If coords fail, assume no overlap
      continue;
    }
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

  const MIN_GAP = -20; // Only merge when actually overlapping by at least 20px
  const clusters: MergeCluster[] = [];
  let currentCluster: PositionGroup[] = [groups[0]];
  let clusterRightEdge = 0;

  // Calculate initial right edge of first group's tooltip
  try {
    const firstLeft = getLeftCoord(groups[0].position);
    const firstWidth = groups[0].cursors.reduce(
      (sum, c) => sum + estimateTooltipWidth(c.name),
      0,
    );
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
      const groupWidth = group.cursors.reduce(
        (sum, c) => sum + estimateTooltipWidth(c.name),
        0,
      );

      // Check if this group overlaps with the current cluster
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
        const width = group.cursors.reduce(
          (sum, c) => sum + estimateTooltipWidth(c.name),
          0,
        );
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
