import { describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import {
  fallbackRefreshDelayMs,
  fallbackRefreshDue,
  newPushFallbackTracker,
  noteCanonicalRefresh,
  noteStreamDown,
  noteStreamOpen,
  SSE_DOWN_REFRESH_MS,
} from "../src/freshness-fallback.ts";

// #3976 degraded mode: when SSE is down, active browser clients refresh
// canonical JSON/page data at the same 5-minute cadence as the
// collector, resume SSE without losing view state (a refresh is a
// fragment reread, never a reload), and stop polling the moment the
// stream is back. Oracle: the pure policy's due/delay answers at
// explicit timestamps.

const T0 = 1_700_000_000_000;

describe("#3976 SSE-down fallback policy", () => {
  test("the fallback cadence is the collector's 5 minutes", () => {
    expect(SSE_DOWN_REFRESH_MS).toBe(5 * 60_000);
  });

  test("never due while the stream is live", () => {
    const tracker = newPushFallbackTracker(T0);
    expect(fallbackRefreshDue(tracker, T0 + 60 * 60_000)).toBe(false);
    expect(fallbackRefreshDelayMs(tracker, T0)).toBeNull();
  });

  test("due exactly on cadence after the stream drops, and repeats", () => {
    let tracker = noteStreamDown(newPushFallbackTracker(T0), T0 + 10_000);
    expect(fallbackRefreshDue(tracker, T0 + 10_000 + SSE_DOWN_REFRESH_MS - 1)).toBe(false);
    expect(fallbackRefreshDue(tracker, T0 + 10_000 + SSE_DOWN_REFRESH_MS)).toBe(true);
    expect(fallbackRefreshDelayMs(tracker, T0 + 10_000)).toBe(SSE_DOWN_REFRESH_MS);

    tracker = noteCanonicalRefresh(tracker, T0 + 10_000 + SSE_DOWN_REFRESH_MS);
    expect(fallbackRefreshDue(tracker, T0 + 10_000 + 2 * SSE_DOWN_REFRESH_MS - 1)).toBe(false);
    expect(fallbackRefreshDue(tracker, T0 + 10_000 + 2 * SSE_DOWN_REFRESH_MS)).toBe(true);
  });

  test("repeated stream errors do not restart the cadence clock", () => {
    let tracker = noteStreamDown(newPushFallbackTracker(T0), T0);
    tracker = noteStreamDown(tracker, T0 + 4 * 60_000);
    expect(fallbackRefreshDue(tracker, T0 + SSE_DOWN_REFRESH_MS)).toBe(true);
  });

  test("stream recovery ends degraded mode immediately", () => {
    let tracker = noteStreamDown(newPushFallbackTracker(T0), T0);
    tracker = noteStreamOpen(tracker);
    expect(fallbackRefreshDue(tracker, T0 + 60 * 60_000)).toBe(false);
    expect(fallbackRefreshDelayMs(tracker, T0 + 60 * 60_000)).toBeNull();
  });

  test("the browser asset implements the same cadence without reloading", async () => {
    const asset = await readFile(join(import.meta.dir, "..", "src", "assets", "freshness-client.js"), "utf8");
    expect(asset).toContain("EventSource");
    expect(asset).toContain("/freshness/stream");
    expect(asset).toContain("/api/freshness-events.json");
    expect(asset).toContain("5 * 60_000");
    expect(asset).toContain("dashrefresh");
    expect(asset).not.toContain("location.reload");
  });

  test("the layout ships the freshness client and the server serves it", async () => {
    const render = await readFile(join(import.meta.dir, "..", "src", "render.ts"), "utf8");
    const server = await readFile(join(import.meta.dir, "..", "src", "server.ts"), "utf8");
    expect(render).toContain("/assets/freshness-client.js");
    expect(server).toContain('path === "/assets/freshness-client.js"');
  });
});
