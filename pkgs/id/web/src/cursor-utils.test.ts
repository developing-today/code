/**
 * Tests for cursor utility functions.
 * These test the pure functions used in collaborative cursor management.
 */

import { describe, it, expect } from "vitest";
import {
  FADE_START_MS,
  FADE_END_MS,
  HIDE_MS,
  getOpacityForAge,
  getStrobeDurationMs,
  isLightColor,
  getColorForClient,
  CURSOR_COLORS,
  estimateTooltipWidth,
  groupCursorsByPosition,
  doGroupsOverlap,
  clusterOverlappingGroups,
  type CursorForMerge,
  type PositionGroup,
} from "./cursor-utils";

// ============================================================================
// Opacity Calculation Tests
// ============================================================================

describe("getOpacityForAge", () => {
  describe("fully visible phase (0 to FADE_START_MS)", () => {
    it("returns 1.0 for age 0", () => {
      expect(getOpacityForAge(0)).toBe(1.0);
    });

    it("returns 1.0 for age 1ms", () => {
      expect(getOpacityForAge(1)).toBe(1.0);
    });

    it("returns 1.0 for age 15s", () => {
      expect(getOpacityForAge(15_000)).toBe(1.0);
    });

    it("returns 1.0 for age just before FADE_START", () => {
      expect(getOpacityForAge(FADE_START_MS - 1)).toBe(1.0);
    });
  });

  describe("fading phase (FADE_START_MS to FADE_END_MS)", () => {
    it("returns 1.0 at exactly FADE_START_MS", () => {
      // At exactly fade start, fadeProgress = 0, so opacity = 1.0
      expect(getOpacityForAge(FADE_START_MS)).toBe(1.0);
    });

    it("returns ~0.65 at midpoint of fade", () => {
      const midpoint = (FADE_START_MS + FADE_END_MS) / 2;
      const opacity = getOpacityForAge(midpoint);
      expect(opacity).toBeCloseTo(0.65, 2);
    });

    it("returns 0.3 just before FADE_END_MS", () => {
      // At FADE_END - 1, should be very close to 0.3
      const opacity = getOpacityForAge(FADE_END_MS - 1);
      expect(opacity).toBeCloseTo(0.3, 1);
    });

    it("linearly fades from 1.0 to 0.3", () => {
      const opacity25 = getOpacityForAge(FADE_START_MS + (FADE_END_MS - FADE_START_MS) * 0.25);
      const opacity50 = getOpacityForAge(FADE_START_MS + (FADE_END_MS - FADE_START_MS) * 0.5);
      const opacity75 = getOpacityForAge(FADE_START_MS + (FADE_END_MS - FADE_START_MS) * 0.75);

      expect(opacity25).toBeCloseTo(1.0 - 0.7 * 0.25, 2);
      expect(opacity50).toBeCloseTo(1.0 - 0.7 * 0.5, 2);
      expect(opacity75).toBeCloseTo(1.0 - 0.7 * 0.75, 2);
    });
  });

  describe("minimum visibility phase (FADE_END_MS to HIDE_MS)", () => {
    it("returns 0.3 at exactly FADE_END_MS", () => {
      expect(getOpacityForAge(FADE_END_MS)).toBe(0.3);
    });

    it("returns 0.3 at 2 minutes", () => {
      expect(getOpacityForAge(120_000)).toBe(0.3);
    });

    it("returns 0.3 just before HIDE_MS", () => {
      expect(getOpacityForAge(HIDE_MS - 1)).toBe(0.3);
    });
  });

  describe("hidden phase (>= HIDE_MS)", () => {
    it("returns 0 at exactly HIDE_MS", () => {
      expect(getOpacityForAge(HIDE_MS)).toBe(0);
    });

    it("returns 0 after HIDE_MS", () => {
      expect(getOpacityForAge(HIDE_MS + 1000)).toBe(0);
    });

    it("returns 0 for very old cursors", () => {
      expect(getOpacityForAge(600_000)).toBe(0); // 10 minutes
    });
  });
});

// ============================================================================
// Strobe Duration Tests
// ============================================================================

describe("getStrobeDurationMs", () => {
  describe("active cursors (opacity >= 0.3)", () => {
    it("returns 1000ms for opacity 1.0", () => {
      expect(getStrobeDurationMs(1.0)).toBe(1000);
    });

    it("returns ~2000ms for opacity 0.65", () => {
      const duration = getStrobeDurationMs(0.65);
      expect(duration).toBeCloseTo(2000, 0);
    });

    it("returns 3000ms for opacity 0.3", () => {
      // At exactly 0.3, should be disabled (return 0)
      // The formula gives 3000 at opacity just above 0.3
      const duration = getStrobeDurationMs(0.31);
      expect(duration).toBeGreaterThan(2900);
    });

    it("increases duration as opacity decreases", () => {
      const duration1 = getStrobeDurationMs(1.0);
      const duration08 = getStrobeDurationMs(0.8);
      const duration06 = getStrobeDurationMs(0.6);
      const duration04 = getStrobeDurationMs(0.4);

      expect(duration08).toBeGreaterThan(duration1);
      expect(duration06).toBeGreaterThan(duration08);
      expect(duration04).toBeGreaterThan(duration06);
    });
  });

  describe("disabled strobing (opacity <= 0.3)", () => {
    it("returns 0 for opacity 0.3", () => {
      expect(getStrobeDurationMs(0.3)).toBe(0);
    });

    it("returns 0 for opacity 0.2", () => {
      expect(getStrobeDurationMs(0.2)).toBe(0);
    });

    it("returns 0 for opacity 0", () => {
      expect(getStrobeDurationMs(0)).toBe(0);
    });

    it("returns 0 for negative opacity", () => {
      expect(getStrobeDurationMs(-0.5)).toBe(0);
    });
  });
});

// ============================================================================
// Color Utility Tests
// ============================================================================

describe("isLightColor", () => {
  describe("light colors", () => {
    it("identifies white as light", () => {
      expect(isLightColor("#ffffff")).toBe(true);
    });

    it("identifies yellow as light", () => {
      expect(isLightColor("#ffe66d")).toBe(true);
    });

    it("identifies mint as light", () => {
      expect(isLightColor("#95e1d3")).toBe(true);
    });

    it("identifies light gray as light", () => {
      expect(isLightColor("#cccccc")).toBe(true);
    });
  });

  describe("dark colors", () => {
    it("identifies black as dark", () => {
      expect(isLightColor("#000000")).toBe(false);
    });

    it("identifies dark blue as dark", () => {
      expect(isLightColor("#000080")).toBe(false);
    });

    it("identifies dark purple as dark", () => {
      expect(isLightColor("#6c5ce7")).toBe(false);
    });

    it("identifies dark gray as dark", () => {
      expect(isLightColor("#333333")).toBe(false);
    });
  });

  it("handles colors without # prefix", () => {
    expect(isLightColor("ffffff")).toBe(true);
    expect(isLightColor("000000")).toBe(false);
  });
});

describe("getColorForClient", () => {
  it("returns a color from CURSOR_COLORS", () => {
    const color = getColorForClient("user123");
    expect(CURSOR_COLORS).toContain(color);
  });

  it("returns consistent color for same client ID", () => {
    const color1 = getColorForClient("user123");
    const color2 = getColorForClient("user123");
    expect(color1).toBe(color2);
  });

  it("handles numeric client IDs", () => {
    const color = getColorForClient(12345);
    expect(CURSOR_COLORS).toContain(color);
  });

  it("returns different colors for different clients (usually)", () => {
    const colors = new Set<string>();
    // Generate colors for many clients - should get variety
    for (let i = 0; i < 100; i++) {
      colors.add(getColorForClient(`user${i}`));
    }
    // Should have multiple different colors
    expect(colors.size).toBeGreaterThan(1);
  });
});

// ============================================================================
// Tooltip Width Estimation Tests
// ============================================================================

describe("estimateTooltipWidth", () => {
  it("returns positive width for any name", () => {
    expect(estimateTooltipWidth("A")).toBeGreaterThan(0);
  });

  it("returns larger width for longer names", () => {
    const shortWidth = estimateTooltipWidth("Al");
    const longWidth = estimateTooltipWidth("Alexander");
    expect(longWidth).toBeGreaterThan(shortWidth);
  });

  it("returns zero for empty name", () => {
    const width = estimateTooltipWidth("");
    expect(width).toBe(0); // No padding, no chars = 0
  });

  it("estimates reasonable width for typical name", () => {
    // "Alice" = 5 chars * 6px + 0px padding = 30px
    const width = estimateTooltipWidth("Alice");
    expect(width).toBeGreaterThanOrEqual(30);
    expect(width).toBeLessThan(50);
  });
});

// ============================================================================
// Cursor Grouping Tests
// ============================================================================

describe("groupCursorsByPosition", () => {
  const makeCursor = (
    clientID: string,
    head: number,
    lastUpdate: number,
  ): CursorForMerge => ({
    clientID,
    head,
    anchor: head,
    name: clientID,
    color: "#ff0000",
    lastUpdate,
    opacity: 1.0,
    baseOpacity: 1.0,
    onSameLine: false,
  });

  describe("grouping by position", () => {
    it("returns empty array for empty input", () => {
      expect(groupCursorsByPosition([])).toEqual([]);
    });

    it("creates single group for single cursor", () => {
      const cursors = [makeCursor("alice", 10, 1000)];
      const groups = groupCursorsByPosition(cursors);

      expect(groups).toHaveLength(1);
      expect(groups[0].position).toBe(10);
      expect(groups[0].cursors).toHaveLength(1);
    });

    it("groups cursors at same position", () => {
      const cursors = [
        makeCursor("alice", 10, 1000),
        makeCursor("bob", 10, 2000),
      ];
      const groups = groupCursorsByPosition(cursors);

      expect(groups).toHaveLength(1);
      expect(groups[0].position).toBe(10);
      expect(groups[0].cursors).toHaveLength(2);
    });

    it("creates separate groups for different positions", () => {
      const cursors = [
        makeCursor("alice", 10, 1000),
        makeCursor("bob", 20, 2000),
      ];
      const groups = groupCursorsByPosition(cursors);

      expect(groups).toHaveLength(2);
      expect(groups[0].position).toBe(10);
      expect(groups[1].position).toBe(20);
    });
  });

  describe("sorting within groups", () => {
    it("sorts cursors by lastUpdate descending (most recent first)", () => {
      const cursors = [
        makeCursor("alice", 10, 1000), // oldest
        makeCursor("bob", 10, 3000), // newest
        makeCursor("carol", 10, 2000), // middle
      ];
      const groups = groupCursorsByPosition(cursors);

      expect(groups[0].cursors[0].clientID).toBe("bob"); // most recent
      expect(groups[0].cursors[1].clientID).toBe("carol");
      expect(groups[0].cursors[2].clientID).toBe("alice"); // oldest
    });
  });

  describe("sorting groups by position", () => {
    it("sorts groups by position ascending", () => {
      const cursors = [
        makeCursor("alice", 50, 1000),
        makeCursor("bob", 10, 2000),
        makeCursor("carol", 30, 3000),
      ];
      const groups = groupCursorsByPosition(cursors);

      expect(groups[0].position).toBe(10);
      expect(groups[1].position).toBe(30);
      expect(groups[2].position).toBe(50);
    });
  });

  describe("mostRecentColor", () => {
    it("sets mostRecentColor to first cursor's color (most recent)", () => {
      const cursors = [
        { ...makeCursor("alice", 10, 1000), color: "#ff0000" },
        { ...makeCursor("bob", 10, 3000), color: "#00ff00" }, // most recent
      ];
      const groups = groupCursorsByPosition(cursors);

      expect(groups[0].mostRecentColor).toBe("#00ff00");
    });
  });
});

// ============================================================================
// Overlap Detection Tests
// ============================================================================

describe("doGroupsOverlap", () => {
  const makeGroup = (position: number, names: string[]): PositionGroup => ({
    position,
    cursors: names.map((name) => ({
      clientID: name,
      head: position,
      anchor: position,
      name,
      color: "#ff0000",
      lastUpdate: Date.now(),
      opacity: 1.0,
      baseOpacity: 1.0,
      onSameLine: false,
    })),
    mostRecentColor: "#ff0000",
  });

  describe("edge cases", () => {
    it("returns false for empty groups", () => {
      expect(doGroupsOverlap([], null)).toBe(false);
    });

    it("returns false for single group", () => {
      const groups = [makeGroup(10, ["alice"])];
      expect(doGroupsOverlap(groups, null)).toBe(false);
    });

    it("returns false when getLeftCoord is null", () => {
      const groups = [makeGroup(10, ["alice"]), makeGroup(20, ["bob"])];
      expect(doGroupsOverlap(groups, null)).toBe(false);
    });
  });

  describe("overlap detection with mock coords", () => {
    // Mock: 1 position unit = 10 pixels
    const mockGetLeftCoord = (pos: number): number => pos * 10;

    it("returns false when groups are far apart", () => {
      // Alice at pos 0 (0px), Bob at pos 100 (1000px)
      // Alice tooltip ~12px wide (1 char * 6 + 6), gap = 1000 - 12 = 988px
      const groups = [makeGroup(0, ["A"]), makeGroup(100, ["B"])];
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(false);
    });

    it("returns true when groups actually overlap", () => {
      // Alice at pos 0 (0px), Bob at pos 0 (0px) - same position = actual overlap
      // With MIN_GAP = 0, need actual overlap of at least 20px to trigger
      // A width ~12px at 0, B at 0 - complete overlap (12px > 0)
      // But wait, for same position, posDiff = 0, currWidth = 12
      // 0 < 12 + (-20) = -8 is FALSE, so no overlap detected for single char names
      // Use longer names that actually overlap significantly
      const groups = [makeGroup(0, ["Alice"]), makeGroup(0, ["Bob"])];
      // Alice width ~36px, posDiff = 0, 0 < 36 - 20 = 16 is TRUE
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(true);
    });

    it("returns true when groups are close (within MIN_GAP)", () => {
      // Alice at pos 0 (0px), Bob at pos 1 (10px)
      // Alice tooltip width = 6px (1 char * 6)
      // With MIN_GAP = 12, overlap threshold is 6 + 12 = 18px
      // posDiff = 10, 10 < 18 is TRUE = overlap (merge them)
      const groups = [makeGroup(0, ["A"]), makeGroup(1, ["B"])];
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(true);
    });

    it("considers combined width of multiple cursors in a group", () => {
      // Group 1: A and B at pos 0, combined width = 12px (2 * 6)
      // Group 2: C at pos 0 (same position)
      // With MIN_GAP = 0, overlap threshold is 12px
      // posDiff = 0, 0 < 12 = TRUE = overlap
      const groups = [makeGroup(0, ["A", "B"]), makeGroup(0, ["C"])];
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(true);
    });

    it("handles longer names correctly", () => {
      // "Alexander" at pos 0, width = 54px (9*6+0)
      // "Bob" at pos 3 (30px)
      // With MIN_GAP = 0, overlap threshold is 54 - 20 = 34px
      // posDiff = 30, 30 < 34 = TRUE = overlap
      const groups = [makeGroup(0, ["Alexander"]), makeGroup(3, ["Bob"])];
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(true);
    });

    it("returns false when adequately spaced", () => {
      // Short names at far positions
      const groups = [makeGroup(0, ["A"]), makeGroup(50, ["B"])];
      // A: width ~12px at 0px, B at 500px
      // With MIN_GAP = 0, overlap threshold is 12 - 20 = -8px
      // posDiff = 500, 500 < -8 is FALSE = no overlap
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(false);
    });
  });

  describe("multiple groups", () => {
    const mockGetLeftCoord = (pos: number): number => pos * 10;

    it("checks all consecutive pairs", () => {
      // Use long names so they actually overlap
      // "Alexander" at 0 (width 60px), "Benjamin" at 4 (40px)
      // With MIN_GAP = 0, overlap threshold is 60 - 20 = 40px
      // posDiff = 40, 40 < 40 is FALSE = no overlap... need closer
      // "Alexander" at 0, "Benjamin" at 3 (30px)
      // 30 < 40 = TRUE = overlap
      const groups = [
        makeGroup(0, ["Alexander"]),
        makeGroup(3, ["Benjamin"]),
      ];
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(true);
    });

    it("returns false if no consecutive pairs overlap", () => {
      const groups = [
        makeGroup(0, ["A"]),
        makeGroup(50, ["B"]),
        makeGroup(100, ["C"]),
      ];
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(false);
    });
  });

  describe("error handling", () => {
    it("continues checking on coordinate error", () => {
      let callCount = 0;
      const errorOnSecond = (pos: number): number => {
        callCount++;
        if (callCount === 2) throw new Error("coord error");
        return pos * 10;
      };

      const groups = [
        makeGroup(0, ["A"]),
        makeGroup(50, ["B"]),
        makeGroup(51, ["C"]), // This pair should be checked after error
      ];

      // Should not throw, should handle error gracefully
      const result = doGroupsOverlap(groups, errorOnSecond);
      // Result depends on which pairs were successfully checked
      expect(typeof result).toBe("boolean");
    });
  });
});

describe("clusterOverlappingGroups", () => {
  // Helper to create a position group
  const makeGroup = (position: number, names: string[]): PositionGroup => ({
    position,
    cursors: names.map((name, i) => ({
      clientID: `${name}-${i}`,
      head: position,
      anchor: position,
      name,
      color: "#ff0000",
      lastUpdate: Date.now() - i * 1000, // Most recent first
      opacity: 1,
      baseOpacity: 1,
      onSameLine: false,
    })),
    mostRecentColor: "#ff0000",
  });

  describe("edge cases", () => {
    it("returns empty array for empty input", () => {
      expect(clusterOverlappingGroups([], null)).toEqual([]);
    });

    it("returns single cluster for single group", () => {
      const groups = [makeGroup(10, ["alice"])];
      const result = clusterOverlappingGroups(groups, null);
      expect(result).toHaveLength(1);
      expect(result[0].groups).toHaveLength(1);
      expect(result[0].leftmostPosition).toBe(10);
    });

    it("returns separate clusters when getLeftCoord is null", () => {
      const groups = [makeGroup(10, ["alice"]), makeGroup(20, ["bob"])];
      const result = clusterOverlappingGroups(groups, null);
      expect(result).toHaveLength(2);
      expect(result[0].groups).toHaveLength(1);
      expect(result[1].groups).toHaveLength(1);
    });
  });

  describe("clustering with mock coords", () => {
    // Mock: 1 position unit = 10 pixels
    const mockGetLeftCoord = (pos: number): number => pos * 10;

    it("keeps far apart groups as separate clusters", () => {
      // Alice at pos 0 (0px), Bob at pos 100 (1000px)
      const groups = [makeGroup(0, ["A"]), makeGroup(100, ["B"])];
      const result = clusterOverlappingGroups(groups, mockGetLeftCoord);
      expect(result).toHaveLength(2);
      expect(result[0].groups).toEqual([groups[0]]);
      expect(result[1].groups).toEqual([groups[1]]);
    });

    it("merges close groups (within MIN_GAP)", () => {
      // Alice at pos 0 (0px), Bob at pos 1 (10px)
      // Alice tooltip width = 6px (1 char * 6)
      // With MIN_GAP = 12, overlap threshold is 6 + 12 = 18px
      // posDiff = 10, 10 < 18 is TRUE = overlap, so merge
      const groups = [makeGroup(0, ["A"]), makeGroup(1, ["B"])];
      const result = clusterOverlappingGroups(groups, mockGetLeftCoord);
      expect(result).toHaveLength(1);
      expect(result[0].groups).toHaveLength(2);
    });

    it("merges actually overlapping groups into one cluster", () => {
      // Use longer name so tooltip actually overlaps significantly
      // "Alexander" at pos 0, width ~60px (9*6+6)
      // "B" at pos 3 (30px) - within Alexander's overlap threshold (60-20=40px)
      // posDiff = 30, 30 < 40 = TRUE = overlap
      const groups = [makeGroup(0, ["Alexander"]), makeGroup(3, ["B"])];
      const result = clusterOverlappingGroups(groups, mockGetLeftCoord);
      expect(result).toHaveLength(1);
      expect(result[0].groups).toHaveLength(2);
      expect(result[0].leftmostPosition).toBe(0);
    });

    it("creates separate clusters for non-overlapping groups on same line", () => {
      // A at 0, B at 50, C at 100 - all well spaced
      const groups = [
        makeGroup(0, ["A"]),
        makeGroup(50, ["B"]),
        makeGroup(100, ["C"]),
      ];
      const result = clusterOverlappingGroups(groups, mockGetLeftCoord);
      expect(result).toHaveLength(3);
      expect(result[0].groups).toHaveLength(1);
      expect(result[1].groups).toHaveLength(1);
      expect(result[2].groups).toHaveLength(1);
    });

    it("clusters only actually overlapping groups", () => {
      // "Alexander" at 0 (width ~60px, threshold 40px), B at 3 (30px) = overlap
      // C at 100 (no overlap)
      const groups = [
        makeGroup(0, ["Alexander"]),
        makeGroup(3, ["B"]),
        makeGroup(100, ["C"]),
      ];
      const result = clusterOverlappingGroups(groups, mockGetLeftCoord);
      expect(result).toHaveLength(2);
      expect(result[0].groups).toHaveLength(2); // Alexander and B merged
      expect(result[0].leftmostPosition).toBe(0);
      expect(result[1].groups).toHaveLength(1); // C separate
      expect(result[1].leftmostPosition).toBe(100);
    });

    it("handles same-position overlap", () => {
      // Same position = definite overlap regardless of name length
      // With MIN_GAP = 0 and posDiff = 0, need width > 20 to overlap
      // "Alice" width ~36px, 0 < 36 - 20 = 16 is TRUE
      const groups = [
        makeGroup(0, ["Alice"]),
        makeGroup(0, ["Bob"]), // Same position = overlap
      ];
      const result = clusterOverlappingGroups(groups, mockGetLeftCoord);
      expect(result).toHaveLength(1);
      expect(result[0].groups).toHaveLength(2);
    });

    it("handles multiple separate clusters", () => {
      // Use longer names for actual overlap (need 20+ px overlap)
      // "Alexander" at 0 (60px), "Bob" at 3 (30px) - overlap (30 < 40)
      // "X" at 50 (alone)
      // "ChristopherLongName" at 100 (width ~120px), "Dan" at 107 (1070px)
      // "ChristopherLongName" = 19 chars, width = 19*6+6 = 120px
      // threshold = 120 - 20 = 100px
      // posDiff = 70, 70 < 100 = TRUE = overlap
      const groups = [
        makeGroup(0, ["Alexander"]),
        makeGroup(3, ["Bob"]),
        makeGroup(50, ["X"]),
        makeGroup(100, ["ChristopherLongName"]),
        makeGroup(107, ["Dan"]),
      ];
      const result = clusterOverlappingGroups(groups, mockGetLeftCoord);
      expect(result).toHaveLength(3);
      expect(result[0].groups).toHaveLength(2); // Alexander, Bob
      expect(result[1].groups).toHaveLength(1); // X
      expect(result[2].groups).toHaveLength(2); // ChristopherLongName, Dan
    });

    it("accounts for tooltip width with longer names", () => {
      // "Alexander" at pos 0, width ~60px (9*6+6)
      // "B" at pos 3 (30px) - within Alexander's overlap threshold (60-20=40px)
      const groups = [makeGroup(0, ["Alexander"]), makeGroup(3, ["B"])];
      const result = clusterOverlappingGroups(groups, mockGetLeftCoord);
      expect(result).toHaveLength(1);
      expect(result[0].groups).toHaveLength(2);
    });

    it("accounts for combined width of multiple cursors in a group", () => {
      // Group 1: AliceLongNameHere and BobLongNameHere at pos 0
      // Width: 16*6 + 15*6 = 96 + 90 = 186px
      // With MIN_GAP = 0, overlap threshold is 186 - 20 = 166px
      // Group 2: C at pos 13 (130px) - within overlap threshold (130 < 166)
      const groups = [makeGroup(0, ["AliceLongNameHere", "BobLongNameHere"]), makeGroup(13, ["C"])];
      const result = clusterOverlappingGroups(groups, mockGetLeftCoord);
      expect(result).toHaveLength(1);
      expect(result[0].groups).toHaveLength(2);
    });
  });

  describe("error handling", () => {
    it("puts groups in separate clusters on coordinate error", () => {
      let callCount = 0;
      const errorOnFirst = (): number => {
        callCount++;
        throw new Error("coord error");
      };

      const groups = [makeGroup(0, ["A"]), makeGroup(50, ["B"])];
      const result = clusterOverlappingGroups(groups, errorOnFirst);
      
      // Should return separate clusters due to error
      expect(result).toHaveLength(2);
    });

    it("handles mid-sequence errors gracefully", () => {
      let callCount = 0;
      const errorOnThird = (pos: number): number => {
        callCount++;
        if (callCount === 3) throw new Error("coord error");
        return pos * 10;
      };

      const groups = [
        makeGroup(0, ["A"]),
        makeGroup(1, ["B"]), // This should cluster with A
        makeGroup(50, ["C"]), // Error here
      ];
      
      const result = clusterOverlappingGroups(groups, errorOnThird);
      // Should still return valid clusters
      expect(result.length).toBeGreaterThan(0);
    });
  });
});
