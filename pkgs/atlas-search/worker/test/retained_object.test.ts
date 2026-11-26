import { env } from "cloudflare:test";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import {
  CLASS_NAMES,
  DEFAULT_CLASSES,
  FAMILIES,
  RetainedObjectError,
  RetainedObjectStore,
  checkClass,
  checkDraft,
  storedPayload,
  type EnvelopeDraft,
  type Family,
  type RetentionClassName,
} from "../src/retained_object";
import baseSql from "../migrations/0003_retained_objects.sql?raw";
import retirementSql from "../migrations/0004_retained_objects_retirement.sql?raw";
import modesSql from "../migrations/0005_retention_modes_and_reports.sql?raw";

const schemaSql = [baseSql, retirementSql, modesSql].join(";\n");

const db = (env as unknown as { HOT_CACHE: D1Database }).HOT_CACHE;
let clock = 1_700_000_000_000;
const store = new RetainedObjectStore(db, () => clock);

function statements(sql: string): string[] {
  return sql
    .split("\n")
    .filter((line) => !line.trim().startsWith("--"))
    .join("\n")
    .split(";")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

beforeAll(async () => {
  for (const stmt of statements(schemaSql)) {
    await db.prepare(stmt).run();
  }
});

beforeEach(async () => {
  await db.prepare("DELETE FROM atlas_retained_objects").run();
  await db.prepare("DELETE FROM atlas_retention_classes").run();
  await store.seedDefaults();
});

function code(fn: () => unknown): string | undefined {
  try {
    fn();
  } catch (error) {
    return error instanceof RetainedObjectError ? error.code : `untyped: ${String(error)}`;
  }
  return undefined;
}

async function codeOf(promise: Promise<unknown>): Promise<string | undefined> {
  try {
    await promise;
  } catch (error) {
    return error instanceof RetainedObjectError ? error.code : `untyped: ${String(error)}`;
  }
  return undefined;
}

const HOME: Record<Family, RetentionClassName> = {
  recommendation: "recommendation",
  search_history: "search-history",
  hot_snapshot: "hot-short",
};

function draft(objectId: string, family: Family = "recommendation", payload: unknown = { items: [] }): EnvelopeDraft {
  return {
    objectId,
    family,
    class: HOME[family],
    capturedAt: "2026-11-26T03:30:00Z",
    inputsRef: "stats:v7",
    payload,
  };
}

async function enable(name: RetentionClassName, allowsQueryText = false): Promise<void> {
  await store.setClass({ ...DEFAULT_CLASSES[name], enabled: true, allowsQueryText });
}

describe("retention classes", () => {
  it("accepts every default and anything longer or higher", () => {
    for (const name of CLASS_NAMES) {
      const base = DEFAULT_CLASSES[name];
      expect(code(() => checkClass(base))).toBeUndefined();
      for (const more of [0, 1, 1000]) {
        expect(code(() => checkClass({ ...base, windowMs: base.windowMs + more }))).toBeUndefined();
        expect(code(() => checkClass({ ...base, keepNewest: base.keepNewest + more }))).toBeUndefined();
      }
    }
  });

  it("refuses a window or floor below the default, one unit at a time", () => {
    for (const name of CLASS_NAMES) {
      const base = DEFAULT_CLASSES[name];
      expect(code(() => checkClass({ ...base, windowMs: base.windowMs - 1 }))).toBe("retention_below_default");
      expect(code(() => checkClass({ ...base, keepNewest: base.keepNewest - 1 }))).toBe("retention_below_default");
      expect(code(() => checkClass({ ...base, windowMs: 0, keepNewest: 0 }))).toBe("retention_below_default");
    }
    expect(code(() => checkClass({ ...DEFAULT_CLASSES["hot-short"], name: "forever" as RetentionClassName }))).toBe(
      "unknown_retention_class",
    );
  });

  it("matches the design defaults", () => {
    const day = 24 * 60 * 60 * 1000;
    expect(DEFAULT_CLASSES["hot-short"]).toMatchObject({ windowMs: day, keepNewest: 2 });
    expect(DEFAULT_CLASSES.recommendation).toMatchObject({ windowMs: 30 * day, keepNewest: 10 });
    expect(DEFAULT_CLASSES["search-history"]).toMatchObject({ windowMs: 90 * day, keepNewest: 20 });
    expect(DEFAULT_CLASSES["snapshot-audit"]).toMatchObject({ windowMs: 365 * day, keepNewest: 50 });
    for (const name of CLASS_NAMES) {
      expect(DEFAULT_CLASSES[name].enabled).toBe(false);
      expect(DEFAULT_CLASSES[name].allowsQueryText).toBe(false);
    }
  });

  it("seeds the defaults once and keeps a longer class", async () => {
    await store.setClass({ ...DEFAULT_CLASSES.recommendation, keepNewest: 25 });
    await store.seedDefaults();
    expect(await store.getClass("recommendation")).toMatchObject({ keepNewest: 25 });
    expect(await store.getClass("hot-short")).toEqual(DEFAULT_CLASSES["hot-short"]);
  });

  it("refuses a shortened class at write time, in the store and in the table", async () => {
    const base = DEFAULT_CLASSES["snapshot-audit"];
    expect(await codeOf(store.setClass({ ...base, windowMs: base.windowMs - 1 }))).toBe("retention_below_default");
    expect(await store.getClass("snapshot-audit")).toEqual(base);
    // The table's own CHECK refuses what a writer that skips the store sends.
    for (const name of CLASS_NAMES) {
      const rule = DEFAULT_CLASSES[name];
      await expect(
        db
          .prepare("UPDATE atlas_retention_classes SET window_ms = ? WHERE class = ?")
          .bind(rule.windowMs - 1, name)
          .run(),
      ).rejects.toThrow(/CHECK/);
      await expect(
        db
          .prepare("UPDATE atlas_retention_classes SET keep_newest = ? WHERE class = ?")
          .bind(rule.keepNewest - 1, name)
          .run(),
      ).rejects.toThrow(/CHECK/);
    }
  });
});

describe("envelope", () => {
  it("pairs each family with its own class or snapshot-audit", () => {
    for (const family of FAMILIES) {
      for (const name of CLASS_NAMES) {
        const outcome = code(() => checkDraft({ ...draft("o", family), class: name }));
        const fits = name === "snapshot-audit" || name === HOME[family];
        expect(outcome, `${family} in ${name}`).toBe(fits ? undefined : "family_class_mismatch");
      }
    }
  });

  it("refuses malformed drafts", () => {
    expect(code(() => checkDraft({ ...draft("o"), objectId: "" }))).toBe("malformed_envelope");
    expect(code(() => checkDraft({ ...draft("o"), inputsRef: "" }))).toBe("malformed_envelope");
    expect(code(() => checkDraft({ ...draft("o"), capturedAt: "yesterday" }))).toBe("malformed_envelope");
    expect(code(() => checkDraft({ ...draft("o"), payload: undefined }))).toBe("malformed_envelope");
    expect(code(() => checkDraft({ ...draft("o"), family: "log" as Family }))).toBe("unknown_family");
  });

  it("keeps query text only under a class that allows it", () => {
    const payload = { query: "who owns atlas", filters: ["repo"], results: ["r1"], session: "s1" };
    const closed = { ...DEFAULT_CLASSES["search-history"] };
    const open = { ...closed, allowsQueryText: true };
    expect(storedPayload("search_history", closed, payload)).toEqual({
      filters: ["repo"],
      results: ["r1"],
      session: "s1",
    });
    expect(storedPayload("search_history", open, payload)).toEqual(payload);
    expect(storedPayload("recommendation", closed, payload)).toEqual(payload);
  });
});

describe("store", () => {
  it("refuses writes until the class is enabled", async () => {
    expect(await codeOf(store.append(draft("rec-1")))).toBe("retention_class_disabled");
    expect(await store.current("rec-1")).toBeUndefined();
  });

  it("appends versions from 1 and never overwrites", async () => {
    await enable("recommendation");
    for (let n = 1; n <= 5; n++) {
      clock += 1000;
      const stored = await store.append(draft("rec-1", "recommendation", { items: [`item-${n}`] }));
      expect(stored.version).toBe(n);
    }
    const history = await store.history("rec-1");
    expect(history.map((v) => v.version)).toEqual([1, 2, 3, 4, 5]);
    expect(history.map((v) => (v.payload as { items: string[] }).items[0])).toEqual([
      "item-1",
      "item-2",
      "item-3",
      "item-4",
      "item-5",
    ]);
    expect((await store.current("rec-1"))?.version).toBe(5);
    expect(await store.version("rec-1", 2)).toEqual({
      ...draft("rec-1", "recommendation", { items: ["item-2"] }),
      version: 2,
    });
    // Another object's versions are its own.
    expect((await store.append(draft("rec-2"))).version).toBe(1);
    // The primary key refuses an overwrite that skips the store.
    await expect(
      db
        .prepare(
          "INSERT INTO atlas_retained_objects (object_id, version, family, class, captured_at, inputs_ref, payload, written_at_ms) VALUES ('rec-1', 3, 'recommendation', 'recommendation', 'x', 'y', '{}', 0)",
        )
        .run(),
    ).rejects.toThrow(/UNIQUE|PRIMARY/);
  });

  it("lands an expected version once and refuses a stale one", async () => {
    await enable("recommendation");
    expect((await store.append(draft("rec-1"), 0)).version).toBe(1);
    expect(await codeOf(store.append(draft("rec-1"), 0))).toBe("version_conflict");
    expect((await store.append(draft("rec-1"), 1)).version).toBe(2);
    expect((await store.history("rec-1")).length).toBe(2);
  });

  it("keeps the family and class pairing in the table", async () => {
    await expect(
      db
        .prepare(
          "INSERT INTO atlas_retained_objects (object_id, version, family, class, captured_at, inputs_ref, payload, written_at_ms) VALUES ('h', 1, 'hot_snapshot', 'recommendation', '2026-11-26T03:30:00Z', 'y', '{}', 0)",
        )
        .run(),
    ).rejects.toThrow(/CHECK/);
  });

  it("drops search-history query text unless the class allows it", async () => {
    await enable("search-history");
    const payload = { query: "who owns atlas", results: ["r1"], session: "s1" };
    const stored = await store.append(draft("session-1", "search_history", payload));
    expect(stored.payload).toEqual({ results: ["r1"], session: "s1" });
    expect((await store.current("session-1"))?.payload).toEqual({ results: ["r1"], session: "s1" });
    await enable("search-history", true);
    const kept = await store.append(draft("session-1", "search_history", payload));
    expect(kept.payload).toEqual(payload);
    expect((await store.version("session-1", 2))?.payload).toEqual(payload);
  });

  it("files any family under snapshot-audit", async () => {
    await enable("snapshot-audit");
    for (const family of FAMILIES) {
      const stored = await store.append({ ...draft(`audit-${family}`, family), class: "snapshot-audit" });
      expect(stored.version).toBe(1);
    }
  });
});
