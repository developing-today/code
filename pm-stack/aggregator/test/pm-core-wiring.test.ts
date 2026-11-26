import { describe, expect, test } from "bun:test";
import { EventEmitter } from "node:events";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { FLEET_FEED_CADENCE_MS, QUOTA_CADENCE_MS, type SchedulerAudit } from "../src/pm-core-scheduler.ts";
import { createPmCoreJobs, startPmCoreSchedulers } from "../src/pm-core-wiring.ts";

type Timer = ReturnType<typeof setInterval>;

function fakeTimers() {
  const scheduled: Array<{ callback: () => void; delayMs: number; timer: Timer }> = [];
  const cleared: Timer[] = [];
  return {
    scheduled,
    cleared,
    setInterval: (callback: () => void, delayMs: number) => {
      const timer = (scheduled.length + 1) as unknown as Timer;
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

    expect(timers.scheduled.map((entry) => entry.delayMs)).toEqual([QUOTA_CADENCE_MS, FLEET_FEED_CADENCE_MS]);
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

    await flush();
    expect(gatherCalls).toBe(1);
    audits.length = 0;
    timers.scheduled[0].callback();
    await flush();
    expect(gatherCalls).toBe(2);
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

    await flush();
    audits.length = 0;
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

  test("the fleet-feed job refreshes credentials before collecting", async () => {
    const calls: string[] = [];
    const jobs = createPmCoreJobs({
      feedFile: "/state/fleet-feed.json",
      refreshFleetFeedCredentials: async () => {
        calls.push("refresh");
      },
      loadFleetFeedMain: async () => async () => {
        calls.push("collect");
      },
    });

    await jobs.runFleetFeed();

    expect(calls).toEqual(["refresh", "collect"]);
  });

  test("the fleet-feed pause file skips collection without loading the collector", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-core-feed-pause-"));
    const pauseFile = join(dir, "fleet-feed.pause");
    await writeFile(pauseFile, "drill\n");
    let loaderCalls = 0;
    const jobs = createPmCoreJobs({
      feedFile: join(dir, "fleet-feed.json"),
      loadFleetFeedMain: async () => {
        loaderCalls += 1;
        return async () => {};
      },
    });

    const result = await jobs.runFleetFeed();
    expect(loaderCalls).toBe(0);
    expect(result).toEqual({ skipped: `fleet-feed paused by ${pauseFile}` });
  });

  test("without a pause file the fleet-feed job runs and reports no skip", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-core-feed-nopause-"));
    let loaderCalls = 0;
    const jobs = createPmCoreJobs({
      feedFile: join(dir, "fleet-feed.json"),
      env: {},
      loadFleetFeedMain: async () => {
        loaderCalls += 1;
        return async () => {};
      },
    });

    const result = await jobs.runFleetFeed();
    expect(loaderCalls).toBe(1);
    expect(result).toBeUndefined();
  });

  test("a paused fleet-feed tick is audited as skipped with the pause reason, not ok", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-core-feed-audit-"));
    const pauseFile = join(dir, "fleet-feed.pause");
    await writeFile(pauseFile, "drill\n");
    const audits: SchedulerAudit[] = [];
    const timers = fakeTimers();
    const runtime = startPmCoreSchedulers({
      feedFile: join(dir, "fleet-feed.json"),
      jobDeps: {
        env: {},
        feedPauseFile: pauseFile,
        loadQuotaMain: async () => async () => 0,
        loadFleetFeedMain: async () => async () => {
          throw new Error("collector must not run while paused");
        },
      },
      writeAudit: (record) => {
        audits.push(record);
      },
      setInterval: timers.setInterval,
      clearInterval: timers.clearInterval,
      signalTarget: new EventEmitter(),
      resignal: () => {},
    });
    await flush();
    runtime.shutdown();

    const feedAudits = audits.filter((record) => record.job === "fleet-feed");
    expect(feedAudits).toHaveLength(1);
    expect(feedAudits[0]?.status).toBe("skipped");
    expect(feedAudits[0]?.reason).toBe(`fleet-feed paused by ${pauseFile}`);
  });

  test("startup refreshes both jobs once instead of waiting for the first cadence", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-core-startup-"));
    const audits: SchedulerAudit[] = [];
    const calls: string[] = [];
    const timers = fakeTimers();
    const runtime = startPmCoreSchedulers({
      feedFile: join(dir, "fleet-feed.json"),
      jobDeps: {
        env: {},
        feedPauseFile: join(dir, "no-such.pause"),
        loadQuotaMain: async () => async () => {
          calls.push("quota");
          return 0;
        },
        loadFleetFeedMain: async () => async () => {
          calls.push("fleet-feed");
        },
      },
      writeAudit: (record) => {
        audits.push(record);
      },
      setInterval: timers.setInterval,
      clearInterval: timers.clearInterval,
      signalTarget: new EventEmitter(),
      resignal: () => {},
    });
    await flush();
    runtime.shutdown();

    expect(calls.sort()).toEqual(["fleet-feed", "quota"]);
    expect(audits.map((record) => `${record.job}:${record.status}`).sort()).toEqual([
      "fleet-feed:ok",
      "quota:ok",
    ]);
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

    await flush();
    timers.scheduled[0].callback();
    await flush();
    const lines = (await readFile(join(dir, "pm-core-scheduler-audit.jsonl"), "utf8")).trim().split("\n");
    const records = lines.map((line) => JSON.parse(line) as SchedulerAudit);
    expect(records.length).toBeGreaterThanOrEqual(2);
    expect(records.every((record) => record.status === "ok")).toBe(true);
    expect(records.filter((record) => record.job === "quota")).toHaveLength(2);
    runtime.shutdown();
  });
});
