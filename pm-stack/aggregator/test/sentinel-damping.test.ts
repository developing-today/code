import { describe, expect, test } from "bun:test";
import {
  applySentinelProbe,
  normalizeSentinelEntry,
  sentinelPostOwed,
  SENTINEL_FAILURE_THRESHOLD,
  type SentinelServiceEntry,
} from "../src/sentinel-damping.ts";

// #9049 flap damping. Every test names its oracle: the exact state and
// post-owed outcome the sentinel must produce for a probe sequence.

const T0 = "2026-11-12T22:00:00.000Z";
const T1 = "2026-11-12T22:00:30.000Z";
const T2 = "2026-11-12T22:01:00.000Z";
const T3 = "2026-11-12T22:01:30.000Z";
const T4 = "2026-11-12T22:02:00.000Z";
const T5 = "2026-11-12T22:02:30.000Z";
const T6 = "2026-11-12T22:03:00.000Z";

const UP = { state: "up" as const, detail: "HTTP 200" };
const DOWN = { state: "down" as const, detail: "probe failed: The operation timed out" };

function upEntry(overrides: Partial<SentinelServiceEntry> = {}): SentinelServiceEntry {
  return {
    state: "up",
    since: T0,
    postedState: "up",
    detail: "HTTP 200",
    pendingFailures: 0,
    pendingSince: null,
    ...overrides,
  };
}

describe("sentinel flap damping (#9049)", () => {
  test("oracle: the threshold is 3 consecutive failed probes", () => {
    expect(SENTINEL_FAILURE_THRESHOLD).toBe(3);
  });

  test("oracle: one failed probe leaves an up service up and owes no post", () => {
    const entry = applySentinelProbe(upEntry(), DOWN, T1);
    expect(entry.state).toBe("up");
    expect(entry.pendingFailures).toBe(1);
    expect(entry.pendingSince).toBe(T1);
    expect(sentinelPostOwed(entry)).toBe(false);
  });

  test("oracle: two consecutive failed probes still leave the service up and owe no post", () => {
    let entry = applySentinelProbe(upEntry(), DOWN, T1);
    entry = applySentinelProbe(entry, DOWN, T2);
    expect(entry.state).toBe("up");
    expect(entry.pendingFailures).toBe(2);
    expect(entry.pendingSince).toBe(T1);
    expect(sentinelPostOwed(entry)).toBe(false);
  });

  test("oracle: the third consecutive failure flips down, stamps since at the streak start, and owes one down post", () => {
    let entry = applySentinelProbe(upEntry(), DOWN, T1);
    entry = applySentinelProbe(entry, DOWN, T2);
    entry = applySentinelProbe(entry, DOWN, T3);
    expect(entry.state).toBe("down");
    expect(entry.since).toBe(T1); // the outage began when probes started failing
    expect(entry.detail).toBe(DOWN.detail);
    expect(entry.pendingFailures).toBe(0);
    expect(sentinelPostOwed(entry)).toBe(true);
    // Once the down post lands, nothing further is owed.
    const posted: SentinelServiceEntry = { ...entry, postedState: "down" };
    expect(sentinelPostOwed(posted)).toBe(false);
  });

  test("oracle: a success between failures resets the streak", () => {
    let entry = applySentinelProbe(upEntry(), DOWN, T1);
    entry = applySentinelProbe(entry, DOWN, T2);
    entry = applySentinelProbe(entry, UP, T3);
    expect(entry.state).toBe("up");
    expect(entry.pendingFailures).toBe(0);
    expect(entry.pendingSince).toBeNull();
    entry = applySentinelProbe(entry, DOWN, T4);
    entry = applySentinelProbe(entry, DOWN, T5);
    expect(entry.state).toBe("up");
    expect(entry.pendingFailures).toBe(2);
    expect(sentinelPostOwed(entry)).toBe(false);
    entry = applySentinelProbe(entry, DOWN, T6);
    expect(entry.state).toBe("down");
    expect(entry.since).toBe(T4);
  });

  test("oracle: recovery after a posted down flips up on the first success and owes a recovery post", () => {
    const downPosted = upEntry({ state: "down", since: T1, postedState: "down", detail: DOWN.detail });
    const entry = applySentinelProbe(downPosted, UP, T4);
    expect(entry.state).toBe("up");
    expect(entry.since).toBe(T4);
    expect(sentinelPostOwed(entry)).toBe(true);
  });

  test("oracle: a down whose post never landed recovers with no post owed (no recovery spam)", () => {
    // The down flipped locally but the throat never accepted the post,
    // so postedState still says "up": the blip must leave no trace.
    const downUnposted = upEntry({ state: "down", since: T1, postedState: "up", detail: DOWN.detail });
    const entry = applySentinelProbe(downUnposted, UP, T2);
    expect(entry.state).toBe("up");
    expect(sentinelPostOwed(entry)).toBe(false);
  });

  test("oracle: first sighting up baselines silently; first sighting down starts a streak and posts nothing", () => {
    const up = applySentinelProbe(undefined, UP, T0);
    expect(up.state).toBe("up");
    expect(up.postedState).toBe("up");
    expect(sentinelPostOwed(up)).toBe(false);

    let entry = applySentinelProbe(undefined, DOWN, T0);
    expect(entry.state).toBe("up");
    expect(entry.pendingFailures).toBe(1);
    expect(entry.pendingSince).toBe(T0);
    expect(sentinelPostOwed(entry)).toBe(false);
    entry = applySentinelProbe(entry, DOWN, T1);
    entry = applySentinelProbe(entry, DOWN, T2);
    expect(entry.state).toBe("down");
    expect(entry.since).toBe(T0);
    expect(sentinelPostOwed(entry)).toBe(true);
  });

  test("oracle: further failures while down keep it down and owe nothing once the down post landed", () => {
    const downPosted = upEntry({ state: "down", since: T1, postedState: "down", detail: DOWN.detail });
    const entry = applySentinelProbe(downPosted, { state: "down", detail: "probe failed: still timing out" }, T5);
    expect(entry.state).toBe("down");
    expect(entry.since).toBe(T1);
    expect(entry.detail).toBe("probe failed: still timing out");
    expect(sentinelPostOwed(entry)).toBe(false);
  });

  test("oracle: an up verdict while up refreshes the detail and clears any streak bookkeeping", () => {
    const streaking = upEntry({ pendingFailures: 2, pendingSince: T1, detail: "probe failing 2/3: x" });
    const entry = applySentinelProbe(streaking, UP, T3);
    expect(entry.state).toBe("up");
    expect(entry.detail).toBe("HTTP 200");
    expect(entry.pendingFailures).toBe(0);
    expect(entry.pendingSince).toBeNull();
  });

  test("oracle: pre-damping persisted entries (no streak fields) normalize to a zero streak", () => {
    const legacy = { state: "up", since: T0, postedState: "up", detail: "HTTP 200" };
    const entry = normalizeSentinelEntry(legacy);
    expect(entry).not.toBeNull();
    expect(entry?.pendingFailures).toBe(0);
    expect(entry?.pendingSince).toBeNull();
    // And a legacy entry still folds correctly.
    const folded = applySentinelProbe(entry ?? undefined, DOWN, T1);
    expect(folded.state).toBe("up");
    expect(folded.pendingFailures).toBe(1);
  });

  test("oracle: normalize rejects values that are not sentinel entries", () => {
    expect(normalizeSentinelEntry(null)).toBeNull();
    expect(normalizeSentinelEntry("down")).toBeNull();
    expect(normalizeSentinelEntry({ state: "sideways", since: T0 })).toBeNull();
    expect(normalizeSentinelEntry({ state: "up" })).toBeNull();
  });
});
