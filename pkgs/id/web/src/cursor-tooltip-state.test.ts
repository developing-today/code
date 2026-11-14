/**
 * Tests for the #1790 tooltip-mode / pin / minimize state machine in
 * cursor-utils.ts (slice 1: pure logic). Oracle: the settled scope text
 * on issue #1790 — modes full/minimal/hidden, hover >= 1s in minimal,
 * pinned exempt in hidden, 1s revert chain big -> tooltip -> minimal.
 */

import { describe, it, expect } from "vitest";
import {
  HOVER_TOOLTIP_DELAY_MS,
  ENLARGE_STEP_MS,
  DEFAULT_CURSOR_DISPLAY_STATE,
  nextTooltipMode,
  getEnlargeStage,
  resolveTooltipDisplay,
  getCursorDisplayState,
  setCursorPinned,
  setCursorMinimized,
  partitionCursorsByMinimized,
  lifecycleEventsForAge,
  formatCursorLifecycleLog,
  FADE_START_MS,
  FADE_END_MS,
  type CursorDisplayStateMap,
  type CursorLifecycleEvent,
} from "./cursor-utils";

const NOW = 100_000;

describe("nextTooltipMode", () => {
  it("cycles full -> minimal -> hidden -> full", () => {
    expect(nextTooltipMode("full")).toBe("minimal");
    expect(nextTooltipMode("minimal")).toBe("hidden");
    expect(nextTooltipMode("hidden")).toBe("full");
  });
});

describe("getEnlargeStage (1s revert chain big -> tooltip -> minimal)", () => {
  it("is null when never enlarged", () => {
    expect(getEnlargeStage(null, NOW)).toBeNull();
  });
  it("is big for the first step", () => {
    expect(getEnlargeStage(NOW, NOW)).toBe("big");
    expect(getEnlargeStage(NOW - (ENLARGE_STEP_MS - 1), NOW)).toBe("big");
  });
  it("is tooltip for the second step", () => {
    expect(getEnlargeStage(NOW - ENLARGE_STEP_MS, NOW)).toBe("tooltip");
    expect(getEnlargeStage(NOW - (2 * ENLARGE_STEP_MS - 1), NOW)).toBe("tooltip");
  });
  it("rests at minimal once the chain has run", () => {
    expect(getEnlargeStage(NOW - 2 * ENLARGE_STEP_MS, NOW)).toBe("minimal");
    expect(getEnlargeStage(NOW - 60_000, NOW)).toBe("minimal");
  });
  it("treats clock skew (negative elapsed) as just enlarged", () => {
    expect(getEnlargeStage(NOW + 5_000, NOW)).toBe("big");
  });
});

describe("resolveTooltipDisplay", () => {
  it("full mode shows the tooltip", () => {
    expect(resolveTooltipDisplay({ mode: "full", now: NOW })).toBe("tooltip");
  });
  it("minimal mode shows cursor only without hover", () => {
    expect(resolveTooltipDisplay({ mode: "minimal", now: NOW })).toBe("minimal");
  });
  it("minimal mode shows the tooltip only at hover >= 1s", () => {
    expect(resolveTooltipDisplay({ mode: "minimal", hoverSince: NOW - (HOVER_TOOLTIP_DELAY_MS - 1), now: NOW })).toBe(
      "minimal",
    );
    expect(resolveTooltipDisplay({ mode: "minimal", hoverSince: NOW - HOVER_TOOLTIP_DELAY_MS, now: NOW })).toBe(
      "tooltip",
    );
  });
  it("hidden mode hides unpinned cursors but still tracks them", () => {
    expect(resolveTooltipDisplay({ mode: "hidden", now: NOW })).toBe("hidden");
  });
  it("hidden mode exempts pinned cursors", () => {
    expect(resolveTooltipDisplay({ mode: "hidden", pinned: true, now: NOW })).toBe("tooltip");
  });
  it("an explicit minimize hides the cursor in any mode, pin or not", () => {
    expect(resolveTooltipDisplay({ mode: "full", minimized: true, now: NOW })).toBe("hidden");
    expect(resolveTooltipDisplay({ mode: "full", minimized: true, pinned: true, now: NOW })).toBe("hidden");
    expect(resolveTooltipDisplay({ mode: "hidden", minimized: true, pinned: true, now: NOW })).toBe("hidden");
  });
  it("the enlarge chain overrides the minimal-mode resting display", () => {
    expect(resolveTooltipDisplay({ mode: "minimal", enlargedAt: NOW, now: NOW })).toBe("big");
    expect(resolveTooltipDisplay({ mode: "minimal", enlargedAt: NOW - ENLARGE_STEP_MS, now: NOW })).toBe("tooltip");
    expect(resolveTooltipDisplay({ mode: "minimal", enlargedAt: NOW - 2 * ENLARGE_STEP_MS, now: NOW })).toBe("minimal");
  });
  it("an elapsed enlarge chain rests at minimal in full mode and during a long minimal-mode hover", () => {
    expect(resolveTooltipDisplay({ mode: "full", enlargedAt: NOW - 5_000, now: NOW })).toBe("minimal");
    expect(resolveTooltipDisplay({ mode: "minimal", enlargedAt: NOW - 5_000, hoverSince: NOW - 2_000, now: NOW })).toBe(
      "minimal",
    );
  });
  it("hidden mode without a pin suppresses even an enlarged cursor", () => {
    expect(resolveTooltipDisplay({ mode: "hidden", enlargedAt: NOW, now: NOW })).toBe("hidden");
    expect(resolveTooltipDisplay({ mode: "hidden", pinned: true, enlargedAt: NOW, now: NOW })).toBe("big");
  });
});

describe("per-cursor display state", () => {
  const empty: CursorDisplayStateMap = new Map();
  it("defaults to the resting state", () => {
    expect(getCursorDisplayState(empty, "a")).toEqual(DEFAULT_CURSOR_DISPLAY_STATE);
  });
  it("setCursorPinned returns a new map and does not mutate the input", () => {
    const next = setCursorPinned(empty, "a", true);
    expect(next).not.toBe(empty);
    expect(empty.size).toBe(0);
    expect(getCursorDisplayState(next, "a").pinned).toBe(true);
    expect(getCursorDisplayState(next, "b").pinned).toBe(false);
  });
  it("pin and minimize compose on the same cursor", () => {
    const pinned = setCursorPinned(empty, 7, true);
    const both = setCursorMinimized(pinned, 7, true);
    const state = getCursorDisplayState(both, 7);
    expect(state.pinned).toBe(true);
    expect(state.minimized).toBe(true);
  });
  it("restore (minimized=false) returns the cursor to the visible set", () => {
    const minimized = setCursorMinimized(empty, "a", true);
    const restored = setCursorMinimized(minimized, "a", false);
    expect(getCursorDisplayState(restored, "a").minimized).toBe(false);
  });
});

describe("partitionCursorsByMinimized (minimize/restore lists)", () => {
  it("splits visible and minimized cursors, preserving order", () => {
    const cursors = [{ clientID: "a" }, { clientID: "b" }, { clientID: "c" }, { clientID: "d" }];
    let states: CursorDisplayStateMap = setCursorMinimized(new Map(), "b", true);
    states = setCursorMinimized(states, "d", true);
    const { visible, minimized } = partitionCursorsByMinimized(cursors, states);
    expect(visible.map((c) => c.clientID)).toEqual(["a", "c"]);
    expect(minimized.map((c) => c.clientID)).toEqual(["b", "d"]);
  });
  it("with no minimized cursors everything is visible", () => {
    const cursors = [{ clientID: 1 }, { clientID: 2 }];
    const { visible, minimized } = partitionCursorsByMinimized(cursors, new Map());
    expect(visible).toHaveLength(2);
    expect(minimized).toHaveLength(0);
  });
});

describe("cursor lifecycle logging (#1790 slice 3)", () => {
  it("reports first-inactive at FADE_START_MS and minimum-reached at FADE_END_MS", () => {
    const logged = new Set<CursorLifecycleEvent>();
    expect(lifecycleEventsForAge(FADE_START_MS - 1, logged)).toEqual([]);
    expect(lifecycleEventsForAge(FADE_START_MS, logged)).toEqual(["first-inactive"]);
    logged.add("first-inactive");
    expect(lifecycleEventsForAge(FADE_END_MS - 1, logged)).toEqual([]);
    expect(lifecycleEventsForAge(FADE_END_MS, logged)).toEqual(["minimum-reached"]);
  });

  it("reports both events for a cursor first observed past the minimum", () => {
    expect(lifecycleEventsForAge(FADE_END_MS + 1, new Set())).toEqual(["first-inactive", "minimum-reached"]);
  });

  it("never repeats an event already logged", () => {
    const logged = new Set<CursorLifecycleEvent>(["first-inactive", "minimum-reached"]);
    expect(lifecycleEventsForAge(FADE_END_MS + 1, logged)).toEqual([]);
  });

  it("formats one structured log line, with the name when known", () => {
    expect(formatCursorLifecycleLog("removed", 42)).toBe("[cursor-lifecycle] removed client=42");
    expect(formatCursorLifecycleLog("hover", "abc", "drew")).toBe("[cursor-lifecycle] hover client=abc name=drew");
  });
});
