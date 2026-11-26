/**
 * The hot-snapshot family under the short class (#9140 slice 4): the live
 * proof from design §7 — three hot snapshots, the short class retires the
 * oldest and keeps the floor.
 */
import { env } from "cloudflare:test";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { HOT_WINDOW, hotSnapshotDraft, hotSnapshotObjectId, recordHotSnapshot } from "../src/hot_snapshot";
import { RecommendationAccumulator } from "../src/recommendation_do";
import { createSnapshot, type ParentEnvelope } from "../src/recommendation_state";
import { DEFAULT_CLASSES, RetainedObjectStore } from "../src/retained_object";
import baseSql from "../migrations/0003_retained_objects.sql?raw";
import retirementSql from "../migrations/0004_retained_objects_retirement.sql?raw";
import modesSql from "../migrations/0005_retention_modes_and_reports.sql?raw";

const schemaSql = [baseSql, retirementSql, modesSql].join(";\n");

const db = (env as unknown as { HOT_CACHE: D1Database }).HOT_CACHE;
const store = new RetainedObjectStore(db);
const encoder = new TextEncoder();
const HOUR = 3_600_000;
const T0 = Date.parse("2026-11-26T00:00:00Z");
const OBJECT = hotSnapshotObjectId("tenant-a", "subject-1");

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
});

beforeEach(async () => {
  await db.prepare("DELETE FROM atlas_retained_objects").run();
  await db.prepare("DELETE FROM atlas_retention_classes").run();
  await store.seedDefaults();
});

function parent(round: number): ParentEnvelope {
  return {
    body: {
      type: "recommendation-parent-v1",
      tenant: "tenant-a",
      subject: "subject-1",
      algorithm: "score-desc-id-asc-v1",
      cutoff_sequence: 12,
      page_size: 10,
      input_digest: `sha256:hot:input-${round}`,
      candidate_set: Array.from({ length: 14 }, (_, index) => ({
        id: `rec-${String(index).padStart(2, "0")}`,
        score_micros: 1000 - ((index * (round + 3)) % 7),
        arrival_sequence: index + 1,
      })),
    } as ParentEnvelope["body"],
    signature: encoder.encode(`parent-signature:hot:${round}`),
  };
}

async function enable(windowMs = DEFAULT_CLASSES["hot-short"].windowMs, mode = "live-cold"): Promise<void> {
  await store.setClass({ ...DEFAULT_CLASSES["hot-short"], windowMs, enabled: true });
  // Promotion itself is covered in retention_report.test.ts.
  await db.prepare("UPDATE atlas_retention_classes SET mode = ? WHERE class = 'hot-short'").bind(mode).run();
}

async function capture(round: number, at: number) {
  return recordHotSnapshot(db, await createSnapshot(parent(round)), "tenant-a", "subject-1", () => at);
}

class MemoryStorage {
  private values = new Map<string, unknown>();
  async get<T>(key: string): Promise<T | undefined> {
    const value = this.values.get(key);
    return value === undefined ? undefined : (structuredClone(value) as T);
  }
  async put<T>(key: string | Record<string, unknown>, value?: T): Promise<void> {
    const entries: Record<string, unknown> = typeof key === "string" ? { [key]: value } : key;
    for (const [k, v] of Object.entries(entries)) this.values.set(k, structuredClone(v));
  }
  async delete(keys: string | string[]): Promise<void> {
    for (const key of Array.isArray(keys) ? keys : [keys]) this.values.delete(key);
  }
}

function accumulator(retained: D1Database | undefined) {
  const state = {
    storage: new MemoryStorage(),
  } as unknown as DurableObjectState;
  const signer = {
    async fetch(): Promise<Response> {
      throw new Error("start never signs");
    },
  };
  return new RecommendationAccumulator(state, {
    PAGE_SIGNER: signer,
    RETAINED: retained,
  });
}

async function start(target: RecommendationAccumulator, value: ParentEnvelope) {
  const response = await target.fetch(
    new Request("https://do/start", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ parent: value }),
    }),
  );
  expect(response.ok).toBe(true);
  return response.json() as Promise<{ started: boolean; hot_version?: number }>;
}

describe("hot snapshots", () => {
  it("holds a bounded window of the top of the order", async () => {
    const snapshot = await createSnapshot(parent(1));
    const draft = hotSnapshotDraft(snapshot, "a/b", "c", new Date(T0));
    expect(draft).toMatchObject({
      objectId: "hot:a%2Fb/c",
      family: "hot_snapshot",
      class: "hot-short",
      capturedAt: "2026-11-26T00:00:00.000Z",
      inputsRef: `parent-snapshot:${snapshot.parent_snapshot_id}`,
    });
    expect(snapshot.ordered.length).toBeGreaterThan(HOT_WINDOW);
    expect(draft.payload.items).toEqual(
      snapshot.ordered.slice(0, HOT_WINDOW).map(({ id, score_micros }) => ({ id, score_micros })),
    );
  });

  it("records nothing without a binding or while the class is disabled", async () => {
    const snapshot = await createSnapshot(parent(1));
    expect(await recordHotSnapshot(undefined, snapshot, "tenant-a", "subject-1")).toEqual({
      recorded: false,
      reason: "off",
    });
    expect(await capture(1, T0)).toEqual({ recorded: false, reason: "off" });
    expect(await store.history(OBJECT)).toEqual([]);
  });

  it("live proof: three snapshots retire the oldest and keep the floor", async () => {
    await enable();
    expect(await capture(1, T0)).toEqual({
      recorded: true,
      version: 1,
      retired: [],
    });
    // Version 1 is past the 24 h window, but the floor of 2 still keeps it.
    expect(await capture(2, T0 + 25 * HOUR)).toEqual({
      recorded: true,
      version: 2,
      retired: [],
    });
    expect(await capture(3, T0 + 26 * HOUR)).toEqual({
      recorded: true,
      version: 3,
      retired: [1],
    });
    expect((await store.live(OBJECT)).map((v) => v.version)).toEqual([2, 3]);
    expect(await store.retired(OBJECT)).toEqual([1]);
    // Retiring is the cold step: the version is still stored.
    expect((await store.history(OBJECT)).map((v) => v.version)).toEqual([1, 2, 3]);
    // A later pass retires nothing twice.
    expect(await new RetainedObjectStore(db, () => T0 + 27 * HOUR).retireExpired(OBJECT)).toEqual([]);
  });

  it("retires nothing while the class is still in dry-run", async () => {
    await enable(DEFAULT_CLASSES["hot-short"].windowMs, "dry-run");
    await capture(1, T0);
    await capture(2, T0 + 25 * HOUR);
    expect(await capture(3, T0 + 26 * HOUR)).toEqual({ recorded: true, version: 3, retired: [] });
    expect(await store.retired(OBJECT)).toEqual([]);
  });

  it("keeps everything inside the window, however many there are", async () => {
    await enable();
    for (const round of [1, 2, 3, 4]) await capture(round, T0 + round * HOUR);
    expect(await store.retired(OBJECT)).toEqual([]);
    expect((await store.live(OBJECT)).length).toBe(4);
  });

  it("follows a lengthened window", async () => {
    await enable(48 * HOUR);
    await capture(1, T0);
    await capture(2, T0 + 25 * HOUR);
    expect(await capture(3, T0 + 26 * HOUR)).toEqual({
      recorded: true,
      version: 3,
      retired: [],
    });
    expect(await capture(4, T0 + 49 * HOUR)).toEqual({
      recorded: true,
      version: 4,
      retired: [1],
    });
  });

  it("is captured on the accumulator's cadence, once per new snapshot", async () => {
    await enable();
    const target = accumulator(db);
    for (const round of [1, 2, 3]) {
      expect((await start(target, parent(round))).hot_version).toBe(round);
      expect((await start(target, parent(round))).hot_version).toBeUndefined();
    }
    const live = await store.live(OBJECT);
    expect(live.map((v) => v.version)).toEqual([1, 2, 3]);
    expect((live[0]!.payload as { items: unknown[] }).items.length).toBe(HOT_WINDOW);
  });

  it("never fails a start when the store is unusable", async () => {
    await enable();
    const broken = {
      prepare() {
        throw new Error("d1 unavailable");
      },
    } as unknown as D1Database;
    const started = await start(accumulator(broken), parent(1));
    expect(started.started).toBe(true);
    expect(started.hot_version).toBeUndefined();
  });
});
