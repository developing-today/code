import { describe, expect, test } from "bun:test";
import { normalizeHookDispatchState, shouldDispatchHealthHook } from "../src/health-hook-dispatch.ts";

describe("health hook dispatch transitions", () => {
  test("dispatches once for an outage and again for recovery, not for intake retries", () => {
    const dispatched: Record<string, "down" | "up"> = {};
    expect(shouldDispatchHealthHook(dispatched, "pm-stack-reachable", "down")).toBe(true);
    dispatched["pm-stack-reachable"] = "down";
    expect(shouldDispatchHealthHook(dispatched, "pm-stack-reachable", "down")).toBe(false);
    expect(shouldDispatchHealthHook(dispatched, "pm-stack-reachable", "up")).toBe(true);
    dispatched["pm-stack-reachable"] = "up";
    expect(shouldDispatchHealthHook(dispatched, "pm-stack-reachable", "up")).toBe(false);
  });

  test("a first up with no recorded state is a baseline, not a dispatch", () => {
    expect(shouldDispatchHealthHook({}, "opencode-working", "up")).toBe(false);
    expect(shouldDispatchHealthHook({}, "opencode-working", "down")).toBe(true);
    expect(shouldDispatchHealthHook({ "opencode-working": "up" }, "opencode-working", "down")).toBe(true);
  });

  test("normalizes persisted dispatch state and ignores malformed values", () => {
    expect(
      normalizeHookDispatchState({
        "pm-stack-reachable": "down",
        "opencode-working": "up",
        bad: "unknown",
      }),
    ).toEqual({ "pm-stack-reachable": "down", "opencode-working": "up" });
    expect(normalizeHookDispatchState(null)).toEqual({});
  });
});
