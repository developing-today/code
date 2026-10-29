// Snapshot retention (#1117, hub design §6): raw monthly files are
// kept 13 months, then compacted into monthly rollups (min/max/last
// per service+metric, resetsAt preserved). The existing reading path
// (SnapshotStore.readAll / byService) must read a compacted store
// with no changes to callers, and compaction must be idempotent.
import { describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { SnapshotRecord } from "../src/model.ts";
import {
  SnapshotStore,
  compactRecordsToRollups,
  expandRollup,
  rollupFileFor,
  shiftMonth,
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
