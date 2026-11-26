/**
 * Admin before/after comparison over the snapshot-audit set (#9140 slice
 * 5): a report for one object across two pinned versions.
 */
import { env } from "cloudflare:test";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import worker, { type Env } from "../src/index";
import { AuditError, auditObjectId, compareAudited, compareEnvelopes, pinForAudit } from "../src/audit_report";
import {
  DEFAULT_CLASSES,
  RetainedObjectError,
  RetainedObjectStore,
  type Envelope,
  type EnvelopeDraft,
} from "../src/retained_object";
import schemaSql from "../migrations/0003_retained_objects.sql?raw";
import retirementSql from "../migrations/0004_retained_objects_retirement.sql?raw";

const db = (env as unknown as { HOT_CACHE: D1Database }).HOT_CACHE;
const store = new RetainedObjectStore(db);
const OBJECT = "recommendation:tenant-a/subject-1";

async function apply(sql: string): Promise<void> {
  const statements = sql
    .split("\n")
    .filter((line) => !line.trim().startsWith("--"))
    .join("\n")
    .split(";")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  for (const stmt of statements) await db.prepare(stmt).run();
}

beforeAll(async () => {
  await apply(schemaSql);
  await apply(retirementSql);
});

beforeEach(async () => {
  await db.prepare("DELETE FROM atlas_retained_objects").run();
  await db.prepare("DELETE FROM atlas_retention_classes").run();
  await store.seedDefaults();
  await store.setClass({ ...DEFAULT_CLASSES.recommendation, enabled: true });
  await store.setClass({ ...DEFAULT_CLASSES["search-history"], allowsQueryText: true, enabled: true });
});

async function enableAudit(): Promise<void> {
  await store.setClass({ ...DEFAULT_CLASSES["snapshot-audit"], enabled: true });
}

function recommendation(rule: string, items: Array<[string, number]>, round: number): EnvelopeDraft {
  return {
    objectId: OBJECT,
    family: "recommendation",
    class: "recommendation",
    capturedAt: `2026-11-2${round}T00:00:00.000Z`,
    inputsRef: `parent-snapshot:p${round}`,
    payload: {
      rule_id: rule,
      parent_snapshot_id: `p${round}`,
      input_digest: `sha256:${round}`,
      items: items.map(([id, score_micros]) => ({ id, score_micros })),
    },
  };
}

describe("audit pins", () => {
  it("refuses while the audit class is disabled", async () => {
    await store.append(recommendation("r1", [["a", 9]], 1));
    await expect(pinForAudit(db, OBJECT, 1)).rejects.toMatchObject({ code: "retention_class_disabled" });
    await expect(pinForAudit(db, OBJECT, 1)).rejects.toBeInstanceOf(RetainedObjectError);
  });

  it("copies a version once and names a missing one", async () => {
    await enableAudit();
    await store.append(recommendation("r1", [["a", 9]], 1));
    expect(await pinForAudit(db, OBJECT, 1)).toEqual({ object_id: OBJECT, version: 1, audit_version: 1, pinned: true });
    expect(await pinForAudit(db, OBJECT, 1)).toEqual({
      object_id: OBJECT,
      version: 1,
      audit_version: 1,
      pinned: false,
    });
    const copy = await store.current(auditObjectId(OBJECT));
    expect(copy).toMatchObject({ class: "snapshot-audit", family: "recommendation", inputsRef: "version:1" });
    await expect(pinForAudit(db, OBJECT, 7)).rejects.toMatchObject({ code: "not_found" });
  });
});

describe("before/after report", () => {
  it("lists added, removed, moved and rescored items within one rule", async () => {
    await enableAudit();
    await store.append(
      recommendation(
        "r1",
        [
          ["a", 900],
          ["b", 800],
          ["c", 700],
        ],
        1,
      ),
    );
    await store.append(
      recommendation(
        "r1",
        [
          ["b", 950],
          ["a", 900],
          ["d", 600],
        ],
        2,
      ),
    );
    await pinForAudit(db, OBJECT, 1);
    await pinForAudit(db, OBJECT, 2);
    const report = await compareAudited(db, OBJECT, 1, 2);
    expect(report).toEqual({
      object_id: OBJECT,
      family: "recommendation",
      from: { version: 1, captured_at: "2026-11-21T00:00:00.000Z" },
      to: { version: 2, captured_at: "2026-11-22T00:00:00.000Z" },
      fields_changed: ["input_digest", "items", "parent_snapshot_id"],
      items: {
        comparable: true,
        added: ["d"],
        removed: ["c"],
        moved: [
          { id: "b", from_rank: 2, to_rank: 1 },
          { id: "a", from_rank: 1, to_rank: 2 },
        ],
        rescored: [{ id: "b", from_micros: 800, to_micros: 950 }],
      },
    });
  });

  it("does not compare scores across rules", async () => {
    await enableAudit();
    await store.append(recommendation("r1", [["a", 900]], 1));
    await store.append(recommendation("r2", [["a", 100]], 2));
    await pinForAudit(db, OBJECT, 1);
    await pinForAudit(db, OBJECT, 2);
    const report = await compareAudited(db, OBJECT, 1, 2);
    expect(report.fields_changed).toContain("rule_id");
    expect(report.items).toEqual({ comparable: false, added: [], removed: [], moved: [], rescored: [] });
  });

  it("reads only the audit set, so it outlives the source versions", async () => {
    await enableAudit();
    await store.append(recommendation("r1", [["a", 900]], 1));
    await store.append(recommendation("r1", [["b", 900]], 2));
    await pinForAudit(db, OBJECT, 1);
    await pinForAudit(db, OBJECT, 2);
    await db.prepare("DELETE FROM atlas_retained_objects WHERE object_id = ?").bind(OBJECT).run();
    const report = await compareAudited(db, OBJECT, 1, 2);
    expect(report.items).toMatchObject({ added: ["b"], removed: ["a"] });
  });

  it("diffs search result refs and keeps query text out of the audit set", async () => {
    await enableAudit();
    const session = "search-session:s1";
    for (const [round, refs] of [
      [1, ["chunk:1", "chunk:2"]],
      [2, ["chunk:2", "chunk:3"]],
    ] as const) {
      await store.append({
        objectId: session,
        family: "search_history",
        class: "search-history",
        capturedAt: `2026-11-2${round}T00:00:00.000Z`,
        inputsRef: `query-key:k${round}`,
        payload: { session: "s1", query: "secret words", filters: { k: 5 }, cache: "miss", result_refs: [...refs] },
      });
    }
    expect(((await store.version(session, 1))!.payload as { query?: string }).query).toBe("secret words");
    await pinForAudit(db, session, 1);
    await pinForAudit(db, session, 2);
    const copy = (await store.current(auditObjectId(session))) as Envelope;
    expect(copy.payload).not.toHaveProperty("query");
    const report = await compareAudited(db, session, 1, 2);
    expect(report.result_refs).toEqual({ added: ["chunk:3"], removed: ["chunk:1"] });
    expect(report.items).toBeUndefined();
  });

  it("names a version that is not pinned, and refuses mixed families", async () => {
    await enableAudit();
    await store.append(recommendation("r1", [["a", 900]], 1));
    await pinForAudit(db, OBJECT, 1);
    await expect(compareAudited(db, OBJECT, 1, 2)).rejects.toMatchObject({ code: "not_audited" });
    const a = (await store.current(auditObjectId(OBJECT)))!;
    expect(() => compareEnvelopes(OBJECT, 1, a, 2, { ...a, family: "hot_snapshot" })).toThrow(AuditError);
  });
});

describe("admin routes", () => {
  const auth = { Authorization: "Bearer test-gate" };
  const base = "https://atlas.test/v1/admin/retained";
  const call = (req: Request, retained: D1Database | undefined = db) =>
    worker.fetch(req, { ...(env as unknown as Env), RETAINED: retained });
  const pin = (body: unknown, headers: Record<string, string> = auth) =>
    new Request(`${base}/audit`, { method: "POST", headers, body: JSON.stringify(body) });

  it("is gated and off without a store", async () => {
    expect((await call(pin({ object: OBJECT, version: 1 }), {})).status).toBe(401);
    const off = await call(pin({ object: OBJECT, version: 1 }), undefined);
    expect(off.status).toBe(503);
  });

  it("pins and compares over HTTP", async () => {
    await store.append(recommendation("r1", [["a", 900]], 1));
    await store.append(recommendation("r1", [["b", 900]], 2));
    const disabled = await call(pin({ object: OBJECT, version: 1 }));
    expect(disabled.status).toBe(409);
    await enableAudit();
    expect((await call(pin({ object: OBJECT, version: 0 }))).status).toBe(400);
    expect((await call(pin({ object: OBJECT, version: 9 }))).status).toBe(404);
    expect(await (await call(pin({ object: OBJECT, version: 1 }))).json()).toMatchObject({ audit_version: 1 });
    const notPinned = await call(
      new Request(`${base}/compare?object=${encodeURIComponent(OBJECT)}&from=1&to=2`, { headers: auth }),
    );
    expect(notPinned.status).toBe(404);
    await call(pin({ object: OBJECT, version: 2 }));
    const report = await call(
      new Request(`${base}/compare?object=${encodeURIComponent(OBJECT)}&from=1&to=2`, { headers: auth }),
    );
    expect(report.status).toBe(200);
    expect(await report.json()).toMatchObject({ items: { added: ["b"], removed: ["a"] } });
    expect((await call(new Request(`${base}/compare?object=x&from=one&to=2`, { headers: auth }))).status).toBe(400);
  });
});
