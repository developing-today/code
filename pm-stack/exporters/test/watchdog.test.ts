// Tests for the quota-watchdog oneshot entry point (#6032).
//
// Oracles, named per the testing doctrine (AGENTS.md):
//   - Acceptance fixtures: the issue's own checklist, driven end to
//     end through runTick against a scratch tree — the broken-source
//     ladder walk with caps honored, the rate-limited cooldown
//     honored from DISK by a fresh tick, the human-only challenge
//     marking the incident human-blocked with its surfacing flag,
//     and quiet-when-healthy.
//   - Cross-invocation persistence (#5928 oracle): every tick is a
//     separate runTick call that re-reads the state file from disk;
//     no state is shared in memory between ticks in these tests, so
//     a pass proves the disk record — not a cache — carries the
//     ladder.
//   - Classification oracle: observation classes match the design's
//     table for the cases the producer cannot see itself (missing
//     document, stale read, dead CDP endpoint).

import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runTick, type WatchdogEffects, type RungOutcome } from "../src/watchdog.ts";
import { loadAdapters, parseAdapter } from "../src/adapters.ts";
import type { EscalationEffectors } from "../src/watchdog-escalation.ts";

const T0 = Date.parse("2026-11-17T00:00:00Z");
const iso = (min: number) => new Date(T0 + min * 60_000).toISOString();

interface Harness {
  dir: string;
  stateFile: string;
  historyFile: string;
  fixtureDir: string;
  logs: string[];
  rung2Calls: number;
  rung3Calls: number;
  escCalls: string[];
  effects: WatchdogEffects;
  setDoc(doc: unknown | null): Promise<void>;
  tick(min: number, liveEscalation?: boolean): ReturnType<typeof runTick>;
}

async function harness(opts: {
  rung2?: () => RungOutcome;
  rung3?: () => RungOutcome;
  cdpOk?: boolean;
} = {}): Promise<Harness> {
  const dir = await mkdtemp(join(tmpdir(), "watchdog-test-"));
  const adaptersDir = join(dir, "adapters");
  const fixtureDir = join(dir, "fixture");
  await mkdir(adaptersDir, { recursive: true });
  await mkdir(fixtureDir, { recursive: true });
  await writeFile(join(adaptersDir, "fixture.json"), JSON.stringify({
    name: "fixture",
    latestPath: join(fixtureDir, "latest.json"),
    cdpPort: 19299,
    origins: ["https://fixture.invalid"],
    cookieNames: ["fixture-session"],
    loginUrl: "https://fixture.invalid/login",
    challengeMarkers: ["verify-it", "2fa"],
    issueRepo: "developing-today/code",
    staleAfterMs: 30 * 60_000,
    cooldownMs: 15 * 60_000,
  }));
  const logs: string[] = [];
  const escCalls: string[] = [];
  const h: Harness = {
    dir,
    stateFile: join(dir, "quota-watchdog-state.json"),
    historyFile: join(dir, "quota-watchdog-history.jsonl"),
    fixtureDir,
    logs,
    rung2Calls: 0,
    rung3Calls: 0,
    escCalls,
    effects: {
      probeCdp: async () => opts.cdpOk === false
        ? { ok: false, detail: "CDP /json/version unreachable: connection refused" }
        : { ok: true, detail: "CDP answered" },
      rung2: async () => {
        h.rung2Calls++;
        return opts.rung2?.() ?? { outcome: "failed", detail: "re-validation failed" };
      },
      rung3: async () => {
        h.rung3Calls++;
        return opts.rung3?.() ?? { outcome: "failed", detail: "re-login failed" };
      },
      escalation: {
        enqueueTask: async (packet) => { escCalls.push(`enqueue:${packet.dedupeKey}`); escCalls.push(`enqueue-issue:${packet.issueNumber}`); return "4242"; },
        openIssue: async (a) => { escCalls.push(`open:${a.repo}`); return 777; },
        commentIssue: async (a) => { escCalls.push(`comment:${a.issueNumber}`); },
        closeIssue: async (a) => { escCalls.push(`close:${a.issueNumber}`); },
        completeTask: async (a) => { escCalls.push(`complete:${a.queueTaskId}`); },
      } satisfies EscalationEffectors,
      log: (line) => logs.push(line),
    },
    async setDoc(doc) {
      const p = join(fixtureDir, "latest.json");
      if (doc === null) await rm(p, { force: true });
      else await writeFile(p, JSON.stringify(doc));
    },
    tick(min, liveEscalation = true) {
      return runTick({
        adaptersDir,
        stateFile: h.stateFile,
        historyFile: h.historyFile,
        effects: h.effects,
        liveEscalation,
        now: new Date(T0 + min * 60_000),
      });
    },
  };
  return h;
}

const okDoc = (min: number) => ({ service: "fixture", status: "ok", measuredAt: iso(min) });
const failDoc = (min: number, status: string, detail = "producer failed") => ({
  service: "fixture", status, measuredAt: iso(min), detail,
});

describe("watchdog entry point — acceptance fixtures", () => {
  test("quiet when healthy: state + history written, no repairs, no output", async () => {
    const h = await harness();
    await h.setDoc(okDoc(0));
    const { state, history } = await h.tick(0);
    expect(state.sources.fixture.consecutiveFailures).toBe(0);
    expect(state.sources.fixture.incident).toBeNull();
    expect(history).toHaveLength(1);
    expect(history[0].decision).toBe("quiet");
    expect(h.rung2Calls + h.rung3Calls).toBe(0);
    expect(h.escCalls).toHaveLength(0);
    expect(h.logs).toHaveLength(0);
    // The state file exists on disk and round-trips.
    const onDisk = JSON.parse(await readFile(h.stateFile, "utf8"));
    expect(onDisk.sources.fixture.lastSuccessAt).toBe(iso(0));
    const histLines = (await readFile(h.historyFile, "utf8")).trim().split("\n");
    expect(histLines).toHaveLength(1);
    await rm(h.dir, { recursive: true, force: true });
  });

  test("broken source walks the ladder rung by rung, caps honored, then escalates exactly once", async () => {
    const h = await harness();
    // class `unknown` enters at rung 1: retry, retry, CDP re-validate x2,
    // re-login x2, then every rung is spent -> escalate.
    let last;
    for (let min = 0; min <= 6; min++) {
      await h.setDoc(failDoc(min, "mystery-failure"));
      last = await h.tick(min);
    }
    const health = last!.state.sources.fixture;
    expect(health.rungAttempts).toEqual({ 0: 0, 1: 2, 2: 2, 3: 2 });
    expect(h.rung2Calls).toBe(2);
    expect(h.rung3Calls).toBe(2);
    expect(health.incident?.surfacing).toBe("escalate");
    expect(health.incident?.escalation?.queueTaskId).toBe("4242");
    expect(health.incident?.escalation?.issueNumber).toBe(777);
    expect(h.escCalls.filter((c) => c.startsWith("enqueue:"))).toHaveLength(1);
    expect(h.escCalls.filter((c) => c.startsWith("open"))).toHaveLength(1);
    // The issue is created first and its number reaches the enqueue:
    // the queue task names the incident issue (task 20118).
    expect(h.escCalls).toContain("enqueue-issue:777");
    expect(h.escCalls.indexOf("open:developing-today/code")).toBeLessThan(h.escCalls.indexOf("enqueue-issue:777"));
    // Repair log records every rung attempt and stays within its bound.
    const rungs = health.repairLog.filter((e) => e.outcome === "attempted").map((e) => e.rung);
    expect(rungs).toEqual([1, 1, 2, 2, 3, 3]);
    // A further failing tick dedupes: no second task, no second issue.
    await h.setDoc(failDoc(7, "mystery-failure"));
    await h.tick(7);
    expect(h.escCalls.filter((c) => c.startsWith("enqueue:"))).toHaveLength(1);
    expect(h.escCalls.filter((c) => c.startsWith("open"))).toHaveLength(1);
    await rm(h.dir, { recursive: true, force: true });
  });

  test("rate-limited fixture: cooldown is set on disk and the next fresh tick honors it with no repair action", async () => {
    const h = await harness();
    await h.setDoc(failDoc(0, "rate-limited", "429 from provider"));
    const first = await h.tick(0);
    expect(first.state.sources.fixture.cooldownUntil).toBe(iso(15));
    expect(first.history[0].decision).toBe("wait-cooldown");
    // Next tick is a fresh invocation re-reading disk; still cooling down.
    await h.setDoc(failDoc(5, "rate-limited", "429 from provider"));
    const second = await h.tick(5);
    expect(second.history[0].decision).toBe("wait-cooldown");
    expect(second.state.sources.fixture.rungAttempts[0]).toBe(1);
    expect(h.rung2Calls + h.rung3Calls).toBe(0);
    await rm(h.dir, { recursive: true, force: true });
  });

  test("human-only challenge at re-login marks the incident human-blocked and surfaces it the same tick", async () => {
    const h = await harness({
      rung3: () => ({ outcome: "failed", detail: "human-only challenge at login (2fa) — stopping" }),
    });
    await h.setDoc(failDoc(0, "session-expired", "landed on login page"));
    const { state } = await h.tick(0);
    const health = state.sources.fixture;
    expect(health.incident?.humanBlocked).toBe(true);
    expect(health.incident?.surfacing).toBe("human-blocked");
    expect(h.logs.some((l) => l.startsWith("NEEDS DREW: fixture"))).toBe(true);
    // The ladder stops dead: a later tick attempts no further repairs.
    await h.setDoc(failDoc(5, "session-expired", "landed on login page"));
    const later = await h.tick(5);
    expect(later.history[0].decision).toBe("human-blocked");
    expect(h.rung3Calls).toBe(1);
    await rm(h.dir, { recursive: true, force: true });
  });

  test("recovery: a healthy tick after escalation comments, closes, and completes the hand-off", async () => {
    const h = await harness();
    for (let min = 0; min <= 6; min++) {
      await h.setDoc(failDoc(min, "mystery-failure"));
      await h.tick(min);
    }
    expect(h.escCalls.filter((c) => c.startsWith("enqueue:"))).toHaveLength(1);
    await h.setDoc(okDoc(10));
    const { state } = await h.tick(10);
    expect(state.sources.fixture.incident).toBeNull();
    expect(h.escCalls).toContain("comment:777");
    expect(h.escCalls).toContain("close:777");
    expect(h.escCalls).toContain("complete:4242");
    await rm(h.dir, { recursive: true, force: true });
  });
});

describe("watchdog entry point — classification of what the producer cannot see", () => {
  test("stale read: a fresh-looking ok document older than the limit is a failure", async () => {
    const h = await harness();
    await h.setDoc({ service: "fixture", status: "ok", measuredAt: iso(-120) });
    const { state, history } = await h.tick(0);
    expect(history[0].observation).toBe("failed");
    expect(history[0].class).toBe("unknown");
    expect(state.sources.fixture.lastDetail).toContain("stale read");
    expect(state.sources.fixture.consecutiveFailures).toBe(1);
    await rm(h.dir, { recursive: true, force: true });
  });

  test("missing document with a dead CDP endpoint classifies as browser-down", async () => {
    const h = await harness({ cdpOk: false });
    await h.setDoc(null);
    const { history } = await h.tick(0);
    expect(history[0].observation).toBe("failed");
    expect(history[0].class).toBe("browser-down");
    await rm(h.dir, { recursive: true, force: true });
  });

  test("producer pre-classification passes through (cdp-unavailable -> cdp-unreachable)", async () => {
    const h = await harness();
    await h.setDoc(failDoc(0, "cdp-unavailable", "browser produced no response"));
    const { history } = await h.tick(0);
    expect(history[0].class).toBe("cdp-unreachable");
    await rm(h.dir, { recursive: true, force: true });
  });
});

describe("adapter loader", () => {
  test("parseAdapter rejects adapters with nothing to observe or no issue repo", () => {
    expect(parseAdapter({ name: "x", issueRepo: "a/b" }).adapter).toBeNull();
    expect(parseAdapter({ name: "x", latestPath: "/tmp/l.json" }).adapter).toBeNull();
    const ok = parseAdapter({ name: "x", latestPath: "/tmp/l.json", issueRepo: "a/b" });
    expect(ok.adapter?.name).toBe("x");
    expect(ok.adapter?.staleAfterMs).toBeGreaterThan(0);
  });

  test("loadAdapters skips a broken file and reports it, keeping the good ones", async () => {
    const dir = await mkdtemp(join(tmpdir(), "watchdog-adapters-"));
    await writeFile(join(dir, "good.json"), JSON.stringify({ name: "good", latestPath: "/tmp/l.json", issueRepo: "a/b" }));
    await writeFile(join(dir, "bad.json"), "{not json");
    const { adapters, errors } = await loadAdapters(dir);
    expect(adapters.map((a) => a.name)).toEqual(["good"]);
    expect(errors.some((e) => e.includes("bad.json"))).toBe(true);
    await rm(dir, { recursive: true, force: true });
  });
});
