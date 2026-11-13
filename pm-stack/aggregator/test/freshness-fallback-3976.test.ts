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

  test("the browser asset refreshes on every fallback poll, not only when the cursor moved", async () => {
    // A consumed notice whose fragment reread fails transiently must
    // be retried by the next cadence poll (Codex review on PR #9193);
    // gating the refresh on cursor movement strands the view until a
    // later notice happens to arrive.
    const asset = await readFile(join(import.meta.dir, "..", "src", "assets", "freshness-client.js"), "utf8");
    expect(asset).toContain("every fallback");
    expect(asset).not.toContain("if (moved)");
  });

  test("the browser asset refreshes even when the journal poll itself fails", async () => {
    // The fragment endpoints are independent of the freshness
    // journal: a journal outage (HTTP error or rejected fetch) must
    // not freeze the dashboard for the whole transport outage
    // (Codex review round 2 on PR #9193). Oracle: in the asset, the
    // poll's refresh dispatch sits after the try/catch, not inside
    // the successful-response branch.
    const asset = await readFile(join(import.meta.dir, "..", "src", "assets", "freshness-client.js"), "utf8");
    expect(asset).toContain("even when the journal poll fails");
    const pollStart = asset.indexOf("async function poll()");
    const refreshAt = asset.indexOf("refresh();", pollStart);
    const catchAt = asset.indexOf("catch (e)", pollStart);
    expect(catchAt).toBeGreaterThan(pollStart);
    expect(refreshAt).toBeGreaterThan(catchAt);
  });

  test("dashrefresh listeners use the Datastar colon form the vendored bundle binds", async () => {
    // The vendored Datastar v1 binds data-on:<event> (colon form);
    // the dash form resolves to an unregistered plugin and never
    // fires, leaving every dashrefresh dispatch inert (Codex review
    // round 2 on PR #9193; see themeToggle() in render.ts). Oracle:
    // every fragment container source carries the colon form and no
    // source carries the dash form.
    for (const rel of ["render.ts", "server.ts", "freshness-delivery.ts"]) {
      const src = await readFile(join(import.meta.dir, "..", "src", rel), "utf8");
      expect(src).toContain("data-on:dashrefresh__window");
      expect(src).not.toContain("data-on-dashrefresh__window");
    }
  });

  test("the layout ships the freshness client and the server serves it", async () => {
    const render = await readFile(join(import.meta.dir, "..", "src", "render.ts"), "utf8");
    const server = await readFile(join(import.meta.dir, "..", "src", "server.ts"), "utf8");
    expect(render).toContain("/assets/freshness-client.js");
    expect(server).toContain('path === "/assets/freshness-client.js"');
  });
});
