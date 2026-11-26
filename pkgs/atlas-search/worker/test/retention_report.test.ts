/**
 * Retention dry-run reports and promotion (#9140 slice 6): a report lists
 * candidate versions per class with no deletes, and a class retires
 * anything only after two unchanged scheduled reports.
 */
import { env } from "cloudflare:test";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import worker, { type Env } from "../src/index";
import { DEFAULT_CLASSES, RetainedObjectStore } from "../src/retained_object";
import { promoteClass, runRetentionReport, type ClassSection, type RetentionReport } from "../src/retention_report";
import baseSql from "../migrations/0003_retained_objects.sql?raw";
import retirementSql from "../migrations/0004_retained_objects_retirement.sql?raw";
import modesSql from "../migrations/0005_retention_modes_and_reports.sql?raw";

const db = (env as unknown as { HOT_CACHE: D1Database }).HOT_CACHE;
const HOUR = 3_600_000;
const T0 = Date.parse("2026-11-26T00:00:00Z");
const OBJECT = "hot:tenant-a/subject-1";
let clock = T0;
const store = new RetainedObjectStore(db, () => clock);

beforeAll(async () => {
  const statements = [baseSql, retirementSql, modesSql]
    .join(";\n")
    .split("\n")
    .filter((line) => !line.trim().startsWith("--"))
    .join("\n")
    .split(";")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  for (const stmt of statements) await db.prepare(stmt).run();
});

beforeEach(async () => {
  await db.prepare("DELETE FROM atlas_retained_objects").run();
  await db.prepare("DELETE FROM atlas_retention_classes").run();
  await db.prepare("DELETE FROM atlas_retention_reports").run();
  clock = T0;
  await store.seedDefaults();
  await store.setClass({ ...DEFAULT_CLASSES["hot-short"], enabled: true });
});

async function hot(at: number): Promise<number> {
  clock = at;
  const stored = await store.append({
    objectId: OBJECT,
    family: "hot_snapshot",
    class: "hot-short",
    capturedAt: new Date(at).toISOString(),
    inputsRef: `parent-snapshot:p${at}`,
    payload: { parent_snapshot_id: `p${at}`, window: 10, items: [{ id: "rec-private-item", score_micros: 1 }] },
  });
  return stored.version;
}

function section(report: RetentionReport, name: string): ClassSection {
  return report.classes.find((entry) => entry.rule.class === name)!;
}

const report = (runId: string, kind: "scheduled" | "on-demand", at: number) =>
  runRetentionReport(db, runId, kind, () => at);

describe("retention dry-run report", () => {
  it("lists candidates past the window and outside the floor, and changes nothing", async () => {
    for (const at of [T0, T0 + HOUR, T0 + 2 * HOUR]) await hot(at);
    const result = await report("run-1", "on-demand", T0 + 30 * HOUR);
    const hotShort = section(result, "hot-short");
    expect(hotShort.rule).toMatchObject({ class: "hot-short", mode: "dry-run", keep_newest: 2, promoted_from: [] });
    expect(hotShort.triggers.age.count).toBe(3);
    expect(hotShort.protected.floor).toBe(2);
    expect(hotShort.candidates).toMatchObject({ count: 1, would_cold_tier: 1, would_delete: 0 });
    expect(hotShort.versions).toEqual([
      expect.objectContaining({ object_id: OBJECT, version: 1, triggers: ["age"], action: "report-only" }),
    ]);
    expect(section(result, "recommendation").candidates.count).toBe(0);
    expect(result.rule_set.map((rule) => rule.class)).toEqual([
      "hot-short",
      "recommendation",
      "search-history",
      "snapshot-audit",
    ]);
    // No content, and nothing retired.
    expect(JSON.stringify(result)).not.toContain("rec-private-item");
    expect(await store.retired(OBJECT)).toEqual([]);
    expect((await store.live(OBJECT)).length).toBe(3);
    expect(await store.retireExpired(OBJECT)).toEqual([]);
  });

  it("reports deltas against the previous report of the same kind", async () => {
    for (const at of [T0, T0 + HOUR, T0 + 2 * HOUR]) await hot(at);
    await report("run-1", "scheduled", T0 + 30 * HOUR);
    await hot(T0 + 30 * HOUR);
    await report("ad-hoc", "on-demand", T0 + 31 * HOUR);
    const second = await report("run-2", "scheduled", T0 + 60 * HOUR);
    expect(second.deltas).toEqual({
      previous_run_id: "run-1",
      newly_eligible: [`${OBJECT}@2`],
      no_longer_eligible: [],
      mode_changes: [],
    });
    await expect(report("run-2", "scheduled", T0 + 61 * HOUR)).rejects.toMatchObject({ code: "duplicate_run" });
  });
});

describe("promotion to live-cold", () => {
  it("needs two unchanged scheduled reports; on-demand ones do not count", async () => {
    for (const at of [T0, T0 + HOUR, T0 + 2 * HOUR]) await hot(at);
    await expect(promoteClass(db, "hot-short")).rejects.toMatchObject({ code: "not_earned" });
    await report("demand-1", "on-demand", T0 + 30 * HOUR);
    await report("demand-2", "on-demand", T0 + 31 * HOUR);
    await report("cycle-1", "scheduled", T0 + 32 * HOUR);
    await expect(promoteClass(db, "hot-short")).rejects.toMatchObject({ code: "not_earned" });
    await report("cycle-2", "scheduled", T0 + 33 * HOUR);
    expect(await promoteClass(db, "hot-short")).toEqual(["cycle-1", "cycle-2"]);
    await expect(promoteClass(db, "nope")).rejects.toMatchObject({ code: "unknown_retention_class" });

    clock = T0 + 34 * HOUR;
    expect(await store.retireExpired(OBJECT)).toEqual([1]);
    const third = await report("cycle-3", "scheduled", T0 + 35 * HOUR);
    expect(third.deltas.mode_changes).toEqual([{ class: "hot-short", from: "dry-run", to: "live-cold" }]);
    expect(third.deltas.no_longer_eligible).toEqual([`${OBJECT}@1`]);
    expect(section(third, "hot-short").rule.promoted_from).toEqual(["cycle-1", "cycle-2"]);
  });

  it("refuses a class whose rule changed between the reports, and a change returns it to dry-run", async () => {
    await report("cycle-1", "scheduled", T0);
    await store.setClass({ ...DEFAULT_CLASSES["hot-short"], windowMs: 48 * HOUR, enabled: true });
    await report("cycle-2", "scheduled", T0 + HOUR);
    await expect(promoteClass(db, "hot-short")).rejects.toMatchObject({ code: "not_earned" });
    await report("cycle-3", "scheduled", T0 + 2 * HOUR);
    expect(await promoteClass(db, "hot-short")).toEqual(["cycle-2", "cycle-3"]);
    // Enabling or disabling keeps the mode; a new floor does not.
    await store.setClass({ ...DEFAULT_CLASSES["hot-short"], windowMs: 48 * HOUR, enabled: false });
    expect(section(await report("cycle-4", "scheduled", T0 + 3 * HOUR), "hot-short").rule.mode).toBe("live-cold");
    await store.setClass({ ...DEFAULT_CLASSES["hot-short"], windowMs: 48 * HOUR, keepNewest: 3, enabled: false });
    const after = section(await report("cycle-5", "scheduled", T0 + 4 * HOUR), "hot-short").rule;
    expect(after).toMatchObject({ mode: "dry-run", promoted_from: [], keep_newest: 3 });
  });
});

describe("report route", () => {
  const auth = { Authorization: "Bearer test-gate" };
  const call = (req: Request) => worker.fetch(req, { ...(env as unknown as Env), RETAINED: db });

  it("runs an on-demand report behind the gate", async () => {
    const url = "https://atlas.test/v1/admin/retained/report";
    expect((await call(new Request(url, { method: "POST" }))).status).toBe(401);
    const response = await call(new Request(url, { method: "POST", headers: auth }));
    expect(response.status).toBe(200);
    const body = (await response.json()) as RetentionReport;
    expect(body.kind).toBe("on-demand");
    expect(body.run_id).toMatch(/^on-demand-/);
    expect((await call(new Request(url, { headers: auth }))).status).toBe(405);
    expect((await call(new Request("https://atlas.test/v1/admin/retained/nope", { headers: auth }))).status).toBe(404);
  });
});
