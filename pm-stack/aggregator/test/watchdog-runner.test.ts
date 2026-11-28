import { describe, expect, test } from "bun:test";
import { createWatchdogHeartbeatRecorder, finishWatchdogGapSync } from "../src/watchdog-runner.ts";

describe("watchdog runner", () => {
  test("records every successful scheduled pass even when the prior pass completed quickly", async () => {
    const writes: number[] = [];
    const recorder = createWatchdogHeartbeatRecorder(async (atMs) => {
      writes.push(atMs);
    });

    await recorder.request(1_000);
    await recorder.scheduled(1_001);

    // Oracle: both successful passes append distinct heartbeat timestamps;
    // scheduled liveness is not suppressed by request-path throttling.
    expect(writes).toEqual([1_000, 1_001]);

    const scheduledWrites: number[] = [];
    const afterScheduled = createWatchdogHeartbeatRecorder(async (atMs) => {
      scheduledWrites.push(atMs);
    });
    await afterScheduled.scheduled(2_000);
    await afterScheduled.request(2_001);
    // Oracle: the scheduled append satisfies the recent-heartbeat window for
    // later request derivations, preventing a duplicate request-path append.
    expect(scheduledWrites).toEqual([2_000]);
  });

  test("request pass returns before gap persistence but records heartbeat only after successful sync", async () => {
    let resolveSync!: () => void;
    const pendingSync = new Promise<void>((resolve) => {
      resolveSync = resolve;
    });
    const heartbeats: string[] = [];
    const warnings: string[] = [];
    const items = ["derived item"];
    let heartbeatRecorded!: () => void;
    const recorded = new Promise<void>((resolve) => {
      heartbeatRecorded = resolve;
    });

    const returned = await finishWatchdogGapSync({
      items,
      sync: pendingSync,
      mode: "request",
      recordHeartbeat: async () => {
        heartbeats.push("recorded");
        heartbeatRecorded();
      },
      onError: (error) => warnings.push(error instanceof Error ? error.message : String(error)),
    });

    // Oracle: an intentionally unresolved persistence promise cannot hold
    // the request result, and no heartbeat is recorded before sync succeeds.
    expect(returned).toBe(items);
    expect(heartbeats).toEqual([]);

    resolveSync();
    await recorded;
    expect(heartbeats).toEqual(["recorded"]);

    // Failures stay observed and never become unhandled rejections or a false
    // success heartbeat.
    const failed = finishWatchdogGapSync({
      items,
      sync: Promise.reject(new Error("persistence failed")),
      mode: "request",
      recordHeartbeat: async () => {
        heartbeats.push("unexpected");
      },
      onError: (error) => warnings.push(error instanceof Error ? error.message : String(error)),
    });
    await expect(failed).resolves.toBe(items);
    await Bun.sleep(0);
    expect(heartbeats).toEqual(["recorded"]);
    expect(warnings).toEqual(["persistence failed"]);
  });

  test("scheduled pass records heartbeat only after gap persistence succeeds", async () => {
    let resolveSync!: () => void;
    const sync = new Promise<void>((resolve) => {
      resolveSync = resolve;
    });
    const heartbeats: string[] = [];
    const items = ["scheduled item"];
    let complete!: () => void;
    const completed = new Promise<void>((resolve) => {
      complete = resolve;
    });

    const pass = finishWatchdogGapSync({
      items,
      sync,
      mode: "scheduled",
      recordHeartbeat: async () => {
        heartbeats.push("recorded");
        complete();
      },
      onError: () => {},
    });
    expect(heartbeats).toEqual([]);
    resolveSync();
    await completed;
    await expect(pass).resolves.toBe(items);
    expect(heartbeats).toEqual(["recorded"]);

    await finishWatchdogGapSync({
      items,
      sync: Promise.reject(new Error("scheduled persistence failed")),
      mode: "scheduled",
      recordHeartbeat: async () => {
        heartbeats.push("unexpected");
      },
      onError: () => {},
    });
    expect(heartbeats).toEqual(["recorded"]);
  });
});
