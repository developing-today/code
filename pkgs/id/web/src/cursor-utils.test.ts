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

  it("includes padding for empty name", () => {
    const width = estimateTooltipWidth("");
    expect(width).toBeGreaterThan(0); // Should have at least padding
  });

  it("estimates reasonable width for typical name", () => {
    // "Alice" = 5 chars * 7px + 16px padding = 51px
    const width = estimateTooltipWidth("Alice");
    expect(width).toBeGreaterThan(40);
    expect(width).toBeLessThan(70);
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
      // Alice tooltip ~23px wide (1 char * 7 + 16), gap = 1000 - 23 = 977px
      const groups = [makeGroup(0, ["A"]), makeGroup(100, ["B"])];
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(false);
    });

    it("returns true when groups are close together", () => {
      // Alice at pos 0 (0px), Bob at pos 2 (20px)
      // Alice tooltip ~23px wide, gap = 20px, need 23 + 8 = 31px minimum
      const groups = [makeGroup(0, ["A"]), makeGroup(2, ["B"])];
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(true);
    });

    it("considers combined width of multiple cursors in a group", () => {
      // Group 1: Alice and Bob at pos 0, combined width ~46px
      // Group 2: Carol at pos 5 (50px)
      // Gap = 50 - 46 - 8 = -4px (overlap!)
      const groups = [makeGroup(0, ["A", "B"]), makeGroup(5, ["C"])];
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(true);
    });

    it("handles longer names correctly", () => {
      // "Alexander" at pos 0, width ~79px
      // "Bob" at pos 5 (50px)
      // Gap would be negative
      const groups = [makeGroup(0, ["Alexander"]), makeGroup(5, ["Bob"])];
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(true);
    });

    it("returns false when adequately spaced", () => {
      // Short names at far positions
      const groups = [makeGroup(0, ["A"]), makeGroup(50, ["B"])];
      // A: width ~23px at 0px, B at 500px
      // Gap = 500 - 23 - 8 = 469px (plenty of space)
      expect(doGroupsOverlap(groups, mockGetLeftCoord)).toBe(false);
    });
  });

  describe("multiple groups", () => {
    const mockGetLeftCoord = (pos: number): number => pos * 10;

    it("checks all consecutive pairs", () => {
      // A at 0, B at 100 (ok), C at 101 (overlaps with B!)
      // B width ~23px at 1000px, C at 1010px
      const groups = [
        makeGroup(0, ["A"]),
        makeGroup(100, ["B"]),
        makeGroup(101, ["C"]),
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
