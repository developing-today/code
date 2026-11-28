import { describe, expect, test } from "bun:test";
import { EventEmitter } from "node:events";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  FLEET_FEED_CADENCE_MS,
  QUOTA_CADENCE_MS,
  type SchedulerAudit,
} from "../src/pm-core-scheduler.ts";
import { createPmCoreJobs, startPmCoreSchedulers } from "../src/pm-core-wiring.ts";

type Timer = ReturnType<typeof setInterval>;

function fakeTimers() {
  const scheduled: Array<{ callback: () => void; delayMs: number; timer: Timer }> = [];
  const cleared: Timer[] = [];
  return {
    scheduled,
    cleared,
    setInterval: (callback: () => void, delayMs: number) => {
      const timer = scheduled.length + 1 as unknown as Timer;
      scheduled.push({ callback, delayMs, timer });
      return timer;
    },
    clearInterval: (timer: Timer) => {
      cleared.push(timer);
    },
  };
}

async function flush(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("pm-core wiring (#8570)", () => {
  test("startup starts both schedulers with the quota and fleet-feed cadences", () => {
    const timers = fakeTimers();
    const runtime = startPmCoreSchedulers({
      feedFile: "/state/fleet-feed.json",
      jobDeps: {
        loadQuotaMain: async () => async () => 0,
        loadFleetFeedMain: async () => async () => {},
      },
      writeAudit: () => {},
      setInterval: timers.setInterval,
      clearInterval: timers.clearInterval,
      signalTarget: new EventEmitter(),
      resignal: () => {},
    });

    expect(timers.scheduled.map((entry) => entry.delayMs)).toEqual([
      QUOTA_CADENCE_MS,
      FLEET_FEED_CADENCE_MS,
    ]);
    runtime.shutdown();
  });

  test("shutdown stops further ticks and is idempotent", () => {
    const timers = fakeTimers();
    const runtime = startPmCoreSchedulers({
      feedFile: "/state/fleet-feed.json",
      jobDeps: {
        loadQuotaMain: async () => async () => 0,
        loadFleetFeedMain: async () => async () => {},
      },
      writeAudit: () => {},
      setInterval: timers.setInterval,
      clearInterval: timers.clearInterval,
      signalTarget: new EventEmitter(),
      resignal: () => {},
    });

    runtime.shutdown();
    expect(timers.cleared).toEqual(timers.scheduled.map((entry) => entry.timer));
    runtime.shutdown();
    expect(timers.cleared).toHaveLength(2);
  });

  test("a quota tick invokes the gatherer and audits ok on a zero exit", async () => {
    const timers = fakeTimers();
    const audits: SchedulerAudit[] = [];
    let gatherCalls = 0;
    const runtime = startPmCoreSchedulers({
      feedFile: "/state/fleet-feed.json",
      jobDeps: {
        loadQuotaMain: async () => async () => {
          gatherCalls += 1;
          return 0;
        },
        loadFleetFeedMain: async () => async () => {},
      },
      writeAudit: (record) => {
        audits.push(record);
      },
      setInterval: timers.setInterval,
      clearInterval: timers.clearInterval,
      signalTarget: new EventEmitter(),
      resignal: () => {},
    });

    timers.scheduled[0].callback();
    await flush();
    expect(gatherCalls).toBe(1);
    expect(audits.map((audit) => [audit.job, audit.status])).toEqual([["quota", "ok"]]);
    runtime.shutdown();
  });

  test("a non-zero gatherer exit becomes an error audit, not a silent ok", async () => {
    const timers = fakeTimers();
    const audits: SchedulerAudit[] = [];
    const runtime = startPmCoreSchedulers({
      feedFile: "/state/fleet-feed.json",
      jobDeps: {
        loadQuotaMain: async () => async () => 1,
        loadFleetFeedMain: async () => async () => {},
      },
      writeAudit: (record) => {
        audits.push(record);
      },
      setInterval: timers.setInterval,
      clearInterval: timers.clearInterval,
      signalTarget: new EventEmitter(),
      resignal: () => {},
    });

    timers.scheduled[0].callback();
    await flush();
    expect(audits).toHaveLength(1);
    expect(audits[0].job).toBe("quota");
    expect(audits[0].status).toBe("error");
    expect(audits[0].error).toContain("exit 1");
    runtime.shutdown();
  });

  test("the fleet-feed job loads the collector lazily and pins FLEET_FEED_OUT to the feed file first", async () => {
    const env: Record<string, string | undefined> = {};
    let loaderCalls = 0;
    let pinSeenAtLoad: string | undefined;
    let collectorCalls = 0;
    const jobs = createPmCoreJobs({
      feedFile: "/state/fleet-feed.json",
      env,
      loadQuotaMain: async () => async () => 0,
      loadFleetFeedMain: async () => {
        loaderCalls += 1;
        pinSeenAtLoad = env.FLEET_FEED_OUT;
        return async () => {
          collectorCalls += 1;
        };
      },
    });

    // Constructing the jobs must not import the collector: its module
    // top level resolves argv and can exit the process.
    expect(loaderCalls).toBe(0);
    await jobs.runFleetFeed();
    expect(loaderCalls).toBe(1);
    expect(pinSeenAtLoad).toBe("/state/fleet-feed.json");
    expect(env.FLEET_FEED_OUT).toBe("/state/fleet-feed.json");
    expect(collectorCalls).toBe(1);
  });

  test("an operator-provided FLEET_FEED_OUT pin always wins over the feed file", async () => {
    const env: Record<string, string | undefined> = { FLEET_FEED_OUT: "/preset/feed.json" };
    let pinSeenAtLoad: string | undefined;
    const jobs = createPmCoreJobs({
      feedFile: "/state/fleet-feed.json",
      env,
      loadFleetFeedMain: async () => {
        pinSeenAtLoad = env.FLEET_FEED_OUT;
        return async () => {};
      },
    });

    await jobs.runFleetFeed();
    expect(pinSeenAtLoad).toBe("/preset/feed.json");
    expect(env.FLEET_FEED_OUT).toBe("/preset/feed.json");
  });

  test("SIGTERM stops the schedulers and re-sends the signal", () => {
    const timers = fakeTimers();
    const signals = new EventEmitter();
    const resent: string[] = [];
    const runtime = startPmCoreSchedulers({
      feedFile: "/state/fleet-feed.json",
      jobDeps: {
        loadQuotaMain: async () => async () => 0,
        loadFleetFeedMain: async () => async () => {},
      },
      writeAudit: () => {},
      setInterval: timers.setInterval,
      clearInterval: timers.clearInterval,
      signalTarget: signals,
      resignal: (signal) => {
        resent.push(signal);
      },
    });

    signals.emit("SIGTERM");
    expect(timers.cleared).toHaveLength(2);
    expect(resent).toEqual(["SIGTERM"]);
    // Handlers are detached: a later SIGINT does nothing further.
    signals.emit("SIGINT");
    expect(resent).toEqual(["SIGTERM"]);
    runtime.shutdown();
  });

  test("the default audit writer appends JSONL records beside the feed file", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-core-wiring-"));
    const timers = fakeTimers();
    const runtime = startPmCoreSchedulers({
      feedFile: join(dir, "fleet-feed.json"),
      jobDeps: {
        loadQuotaMain: async () => async () => 0,
        loadFleetFeedMain: async () => async () => {},
      },
      setInterval: timers.setInterval,
      clearInterval: timers.clearInterval,
      signalTarget: new EventEmitter(),
      resignal: () => {},
    });

    timers.scheduled[0].callback();
    await flush();
    const lines = (await readFile(join(dir, "pm-core-scheduler-audit.jsonl"), "utf8"))
      .trim()
      .split("\n");
    expect(lines).toHaveLength(1);
    const record = JSON.parse(lines[0]) as SchedulerAudit;
    expect(record.job).toBe("quota");
    expect(record.status).toBe("ok");
    runtime.shutdown();
  });
});
