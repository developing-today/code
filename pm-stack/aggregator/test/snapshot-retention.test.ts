// Snapshot retention (#1117, hub design §6): raw monthly files are
// kept 13 months, then compacted into monthly rollups (min/max/last
// per service+metric, resetsAt preserved). The existing reading path
// (SnapshotStore.readAll / byService) must read a compacted store
// with no changes to callers, and compaction must be idempotent.
import { describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, readdir, readFile, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { SnapshotRecord } from "../src/model.ts";
import {
  SnapshotStore,
  compactRecordsToRollups,
  expandRollup,
  rollupFileFor,
  shiftMonth,
  withSnapshotStoreLock,
} from "../src/snapshots.ts";

const NOW = new Date("2026-10-29T12:00:00Z"); // cutoff month: 2025-09

function rec(ts: string, serviceId: string, metricId: string, value: number, extra: Partial<SnapshotRecord> = {}): SnapshotRecord {
  return {
    ts,
    serviceId,
    metricId,
    value,
    unit: "pct",
    source: "api",
    observedBy: "test",
    ...extra,
  };
}

async function seedStore(dir: string): Promise<void> {
  await mkdir(dir, { recursive: true });
  // Eligible month (2025-06): two services, one with a resetsAt window.
  const june: SnapshotRecord[] = [
    rec("2025-06-03T12:00:00Z", "svc-a", "weekly", 40, { limit: 100, resetsAt: "2025-06-08T00:00:00Z" }),
    rec("2025-06-10T12:00:00Z", "svc-a", "weekly", 85, { limit: 100, resetsAt: "2025-06-15T00:00:00Z" }),
    rec("2025-06-20T12:00:00Z", "svc-a", "weekly", 10, { limit: 100, resetsAt: "2025-06-22T00:00:00Z" }),
    rec("2025-06-05T12:00:00Z", "svc-b", "tokens", 7, { unit: "count" }),
    rec("2025-06-25T12:00:00Z", "svc-b", "tokens", 9, { unit: "count" }),
  ];
  await writeFile(join(dir, "usage-2025-06.jsonl"), june.map((r) => JSON.stringify(r)).join("\n") + "\n");
  // Boundary month (2025-09, exactly 13 months back): eligible.
  await writeFile(
    join(dir, "usage-2025-09.jsonl"),
    JSON.stringify(rec("2025-09-15T12:00:00Z", "svc-a", "weekly", 55)) + "\n",
  );
  // Recent month (2025-10, 12 months back): stays raw.
  await writeFile(
    join(dir, "usage-2025-10.jsonl"),
    JSON.stringify(rec("2025-10-15T12:00:00Z", "svc-a", "weekly", 66)) + "\n",
  );
}

describe("shiftMonth", () => {
  test("shifts across year boundaries", () => {
    expect(shiftMonth("2026-10", -13)).toBe("2025-09");
    expect(shiftMonth("2026-01", -1)).toBe("2025-12");
    expect(shiftMonth("2025-12", 2)).toBe("2026-02");
  });
});

describe("compactRecordsToRollups / expandRollup", () => {
  test("min/max/last are per service+metric and resetsAt rides the last record", () => {
    const records = [
      rec("2025-06-03T12:00:00Z", "svc-a", "weekly", 40, { resetsAt: "2025-06-08T00:00:00Z" }),
      rec("2025-06-10T12:00:00Z", "svc-a", "weekly", 85, { resetsAt: "2025-06-15T00:00:00Z" }),
      rec("2025-06-20T12:00:00Z", "svc-a", "weekly", 10, { resetsAt: "2025-06-22T00:00:00Z" }),
    ];
    const rollups = compactRecordsToRollups("2025-06", records);
    expect(rollups).toHaveLength(1);
    const r = rollups[0];
    expect(r.count).toBe(3);
    expect(r.valueMin).toBe(10);
    expect(r.minTs).toBe("2025-06-20T12:00:00Z");
    expect(r.valueMax).toBe(85);
    expect(r.valueLast).toBe(10);
    expect(r.resetsAt).toBe("2025-06-22T00:00:00Z");
    const expanded = expandRollup(r);
    const last = expanded[expanded.length - 1];
    expect(last.value).toBe(10);
    expect(last.resetsAt).toBe("2025-06-22T00:00:00Z");
    expect(Math.min(...expanded.map((e) => e.value as number))).toBe(10);
    expect(Math.max(...expanded.map((e) => e.value as number))).toBe(85);
  });

  test("estimate pairs compact by low/high and expand back to pairs", () => {
    const records: SnapshotRecord[] = [
      { ts: "2025-06-02T12:00:00Z", serviceId: "svc-c", metricId: "payg-estimate", value: { low: 1, high: 5 }, unit: "usd-est", source: "estimate", observedBy: "test" },
      { ts: "2025-06-12T12:00:00Z", serviceId: "svc-c", metricId: "payg-estimate", value: { low: 3, high: 4 }, unit: "usd-est", source: "estimate", observedBy: "test" },
    ];
    const [r] = compactRecordsToRollups("2025-06", records);
    expect(r.valueMin).toEqual({ low: 1, high: 5 });
    expect(r.valueLast).toEqual({ low: 3, high: 4 });
    expect(expandRollup(r).length).toBeGreaterThan(0);
  });
});

describe("SnapshotStore.compact", () => {
  test("compacts eligible months, keeps recent raw, stays readable, idempotent", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-retention-"));
    try {
      await seedStore(dir);
      const store = new SnapshotStore(dir);

      // Dry run first: report only, disk untouched.
      const dry = await store.compact({ now: NOW, dryRun: true });
      expect(dry.cutoffMonth).toBe("2025-09");
      expect(dry.months.map((m) => m.month)).toEqual(["2025-06", "2025-09"]);
      const june = dry.months[0];
      expect(june.recordsBefore).toBe(5);
      expect(june.recordsAfter).toBe(2);
      expect(june.perService["svc-a"]).toEqual({ before: 3, after: 1 });
      expect(june.perService["svc-b"]).toEqual({ before: 2, after: 1 });
      expect((await readdir(dir)).sort()).toEqual([
        "usage-2025-06.jsonl",
        "usage-2025-09.jsonl",
        "usage-2025-10.jsonl",
      ]);

      // Real run.
      const report = await store.compact({ now: NOW });
      expect(report.months).toHaveLength(2);
      const names = (await readdir(dir)).sort();
      expect(names).toEqual([
        rollupFileFor("2025-06"),
        rollupFileFor("2025-09"),
        "usage-2025-10.jsonl",
      ]);

      // The existing reading path reads the compacted store: svc-a's
      // last June value and its resetsAt survive, the max is visible,
      // and the October raw record is untouched.
      const all = await store.readAll();
      const svcA = all.filter((r) => r.serviceId === "svc-a");
      expect(svcA.map((r) => r.value)).toContain(85); // June max preserved
      expect(svcA.map((r) => r.value)).toContain(66); // October raw
      const juneLast = svcA.filter((r) => r.ts.startsWith("2025-06")).at(-1);
      expect(juneLast?.value).toBe(10);
      expect(juneLast?.resetsAt).toBe("2025-06-22T00:00:00Z");
      const grouped = await store.byService();
      expect(grouped["svc-b"].map((r) => r.value)).toEqual([7, 9]);

      // Idempotent: a second run reports the months unchanged in
      // effect (no raw records left to fold) and the files are
      // byte-identical.
      const rollupBefore = await readFile(join(dir, rollupFileFor("2025-06")), "utf8");
      const second = await store.compact({ now: NOW });
      expect(second.months.every((m) => m.recordsBefore === m.recordsAfter || m.unchanged)).toBe(true);
      expect(await readFile(join(dir, rollupFileFor("2025-06")), "utf8")).toBe(rollupBefore);
      expect((await store.readAll()).length).toBe(all.length);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("a store with only recent data is a no-op", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-retention-"));
    try {
      await writeFile(
        join(dir, "usage-2026-10.jsonl"),
        JSON.stringify(rec("2026-10-05T12:00:00Z", "svc-a", "weekly", 12)) + "\n",
      );
      const report = await new SnapshotStore(dir).compact({ now: NOW });
      expect(report.months).toEqual([]);
      expect(await readdir(dir)).toEqual(["usage-2026-10.jsonl"]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

describe("Codex review regressions (PR #2379)", () => {
  test("append stays deduped after compaction: replaying a batch appends nothing", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-retention-"));
    try {
      const store = new SnapshotStore(dir);
      const batch = [1, 2, 3, 4].map((v, i) =>
        rec(`2025-06-0${i + 2}T12:00:00Z`, "svc-a", "weekly", v),
      );
      expect(await store.append(batch)).toBe(4);
      await store.compact({ now: NOW });
      // The batch's non-extreme observations (2, 3) are no longer in
      // the expanded history — only the rollup keys remember them.
      expect(await store.append(batch)).toBe(0);
      expect((await readdir(dir)).filter((n) => n === "usage-2025-06.jsonl")).toEqual([]);
      expect((await store.rollupKeys()).length).toBe(4);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("late records merge quota metadata on its own clock, matching one-pass compaction", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-retention-"));
    const dir2 = await mkdtemp(join(tmpdir(), "snap-retention-"));
    try {
      const late = rec("2025-06-20T12:00:00Z", "svc-a", "weekly", 10);
      const early = rec("2025-06-02T12:00:00Z", "svc-a", "weekly", 40, {
        limit: 100,
        resetsAt: "2025-06-08T00:00:00Z",
      });
      // Incremental: compact the late record first, then merge the early one.
      const inc = new SnapshotStore(dir);
      await inc.append([late]);
      await inc.compact({ now: NOW });
      await inc.append([early]);
      await inc.compact({ now: NOW });
      // One-pass: both records compacted together.
      const one = new SnapshotStore(dir2);
      await one.append([late, early]);
      await one.compact({ now: NOW });
      const incRollups = await readFile(join(dir, rollupFileFor("2025-06")), "utf8");
      const oneRollups = await readFile(join(dir2, rollupFileFor("2025-06")), "utf8");
      expect(JSON.parse(incRollups)).toEqual(JSON.parse(oneRollups));
      const last = (await inc.readAll()).at(-1);
      expect(last?.resetsAt).toBe("2025-06-08T00:00:00Z");
      expect(last?.limit).toBe(100);
    } finally {
      await rm(dir, { recursive: true, force: true });
      await rm(dir2, { recursive: true, force: true });
    }
  });

  test("a malformed rollup line (missing timestamps) never poisons readAll", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-retention-"));
    try {
      await writeFile(
        join(dir, rollupFileFor("2025-06")),
        JSON.stringify({
          kind: "snapshot-rollup/v1",
          month: "2025-06",
          serviceId: "svc-a",
          metricId: "weekly",
          unit: "pct",
          source: "api",
          count: 3,
          valueMin: 1,
          valueMax: 1,
          valueLast: 1,
        }) + "\n",
      );
      await writeFile(
        join(dir, "usage-2026-10.jsonl"),
        JSON.stringify(rec("2026-10-05T12:00:00Z", "svc-a", "weekly", 12)) + "\n",
      );
      const all = await new SnapshotStore(dir).readAll();
      expect(all).toHaveLength(1);
      expect(all[0].value).toBe(12);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("concurrent append + compact lose no observations", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-retention-"));
    try {
      const a = new SnapshotStore(dir);
      const b = new SnapshotStore(dir);
      const batches = Array.from({ length: 6 }, (_, i) => [
        rec(`2025-06-1${i}T12:00:00Z`, "svc-a", "weekly", i + 1),
      ]);
      await Promise.all([
        (async () => {
          for (const batch of batches) await a.append(batch);
        })(),
        (async () => {
          for (let i = 0; i < 6; i++) await b.compact({ now: NOW });
        })(),
      ]);
      await b.compact({ now: NOW });
      // Every appended observation is durably remembered exactly once.
      expect((await b.rollupKeys()).length).toBe(6);
      const rollupText = await readFile(join(dir, rollupFileFor("2025-06")), "utf8");
      expect(JSON.parse(rollupText).count).toBe(6);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

describe("withSnapshotStoreLock stale reclamation (PR #2379 Codex round 2)", () => {
  test("concurrent reclaimers of an abandoned lock never overlap critical sections", async () => {
    // Regression: reclamation used to rm the lock path unconditionally,
    // so a slower reclaimer could delete the fresh lock a faster one
    // had just acquired and both critical sections ran at once.
    for (let trial = 0; trial < 20; trial++) {
      const dir = await mkdtemp(join(tmpdir(), "snap-lock-"));
      try {
        const lockPath = join(dir, ".snapshots.lock");
        await writeFile(lockPath, "dead-holder");
        const past = new Date(Date.now() - 10 * 60_000);
        await utimes(lockPath, past, past);
        let active = 0;
        let maxActive = 0;
        await Promise.all(
          Array.from({ length: 6 }, () =>
            withSnapshotStoreLock(dir, async () => {
              active++;
              maxActive = Math.max(maxActive, active);
              await new Promise((r) => setTimeout(r, 5));
              active--;
            }),
          ),
        );
        expect(maxActive).toBe(1);
      } finally {
        await rm(dir, { recursive: true, force: true });
      }
    }
  });

  test("both lockfiles abandoned: concurrent reclaimers all complete, never overlap", async () => {
    // Regression (Codex round 3): when BOTH the main lock and the
    // reclaim guard were abandoned, a reclaimer could rename a fresh
    // guard another contender had just acquired and link it back
    // after its owner had already released it — recreating an
    // ownerless guard that stalled everyone else into a lock
    // timeout. Seeding both files stale and running twelve
    // concurrent 2 ms callbacks reproduced the timeout.
    for (let trial = 0; trial < 20; trial++) {
      const dir = await mkdtemp(join(tmpdir(), "snap-lock-"));
      try {
        const past = new Date(Date.now() - 10 * 60_000);
        for (const name of [".snapshots.lock", ".snapshots.reclaim.guard"]) {
          const f = join(dir, name);
          await writeFile(f, "dead-holder");
          await utimes(f, past, past);
        }
        let active = 0;
        let maxActive = 0;
        await Promise.all(
          Array.from({ length: 12 }, () =>
            withSnapshotStoreLock(dir, async () => {
              active++;
              maxActive = Math.max(maxActive, active);
              await new Promise((r) => setTimeout(r, 2));
              active--;
            }),
          ),
        );
        expect(maxActive).toBe(1);
        // No guard, lock, reclaim, or tombstone debris is left behind.
        expect(await readdir(dir)).toEqual([]);
      } finally {
        await rm(dir, { recursive: true, force: true });
      }
    }
  }, 30_000);
});
