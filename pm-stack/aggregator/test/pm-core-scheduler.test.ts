import { describe, expect, test } from "bun:test";
import {
  createPmCoreScheduler,
  FLEET_FEED_CADENCE_MS,
  QUOTA_CADENCE_MS,
  type SchedulerAudit,
} from "../src/pm-core-scheduler.ts";

describe("pm-core scheduler (#8570)", () => {
  test("registers the quota and fleet-feed cadences without reading audit state", () => {
    const timers: number[] = [];
    const audits: SchedulerAudit[] = [];
    const scheduler = createPmCoreScheduler({
      runQuota: () => {},
      runFleetFeed: () => {},
      writeAudit: (record) => {
        audits.push(record);
      },
      setInterval: (_callback, delayMs) => {
        timers.push(delayMs);
        return delayMs as unknown as ReturnType<typeof setInterval>;
      },
      clearInterval: () => {},
    });

    scheduler.start();
    scheduler.start();
    expect(timers).toEqual([QUOTA_CADENCE_MS, FLEET_FEED_CADENCE_MS]);
    expect(audits).toEqual([]);
    scheduler.stop();
  });

  test("single-flights a job and writes success audit after it completes", async () => {
    let release!: () => void;
    let calls = 0;
    const audits: SchedulerAudit[] = [];
    const scheduler = createPmCoreScheduler({
      runQuota: () => {
        calls += 1;
        return new Promise<void>((resolve) => {
          release = resolve;
        });
      },
      runFleetFeed: () => {},
      writeAudit: (record) => {
        audits.push(record);
      },
      now: (() => {
        let tick = 0;
        return () => new Date(`2026-11-23T00:00:0${tick++}Z`);
      })(),
    });

    const first = scheduler.runNow("quota");
    const second = scheduler.runNow("quota");
    expect(scheduler.running("quota")).toBe(true);
    expect(calls).toBe(1);
    release();
    await Promise.all([first, second]);
    expect(scheduler.running("quota")).toBe(false);
    expect(audits).toHaveLength(1);
    expect(audits[0].status).toBe("ok");
    expect(audits[0].job).toBe("quota");
  });

  test("records a failed job without stopping the other scheduler", async () => {
    const audits: SchedulerAudit[] = [];
    const scheduler = createPmCoreScheduler({
      runQuota: async () => {
        throw new Error("provider unavailable");
      },
      runFleetFeed: () => {},
      writeAudit: (record) => {
        audits.push(record);
      },
    });

    await scheduler.runNow("quota");
    await scheduler.runNow("fleet-feed");
    expect(audits.map((audit) => [audit.job, audit.status])).toEqual([
      ["quota", "error"],
      ["fleet-feed", "ok"],
    ]);
    expect(audits[0].error).toBe("provider unavailable");
  });
});
