/**
 * The recommendation family of retained objects (#9140 slice 2): one
 * version per recompute, reproducible from its inputs.
 */
import { env } from "cloudflare:test";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { RecommendationAccumulator } from "../src/recommendation_do";
import {
  recommendationDraft,
  recommendationObjectId,
  recordRecommendation,
  reproduces,
  type RecommendationPayload,
} from "../src/recommendation_history";
import { createSnapshot, type ParentEnvelope } from "../src/recommendation_state";
import { DEFAULT_CLASSES, RetainedObjectStore } from "../src/retained_object";
import schemaSql from "../migrations/0003_retained_objects.sql?raw";

const db = (env as unknown as { HOT_CACHE: D1Database }).HOT_CACHE;
const store = new RetainedObjectStore(db);
const encoder = new TextEncoder();

beforeAll(async () => {
  const statements = schemaSql
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
  await store.seedDefaults();
});

function parent(round: number, tenant = "tenant-a", subject = "subject-1"): ParentEnvelope {
  return {
    body: {
      type: "recommendation-parent-v1",
      tenant,
      subject,
      algorithm: "score-desc-id-asc-v1",
      cutoff_sequence: 12,
      page_size: 10,
      input_digest: `sha256:${tenant}:${subject}:input-${round}`,
      candidate_set: Array.from({ length: 14 }, (_, index) => ({
        id: `rec-${String(index).padStart(2, "0")}`,
        score_micros: 1000 - ((index * (round + 3)) % 7),
        arrival_sequence: index + 1,
      })),
    } as ParentEnvelope["body"],
    signature: encoder.encode(`parent-signature:${tenant}:${subject}:${round}`),
  };
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

const unusedSigner = {
  async fetch(): Promise<Response> {
    throw new Error("start never signs");
  },
};

function accumulator(retained: D1Database | undefined) {
  const state = { storage: new MemoryStorage() } as unknown as DurableObjectState;
  return new RecommendationAccumulator(state, { PAGE_SIGNER: unusedSigner, RETAINED: retained });
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
  return response.json() as Promise<{ started: boolean; parent_snapshot_id: string; retained_version?: number }>;
}

async function enable(): Promise<void> {
  await store.setClass({ ...DEFAULT_CLASSES.recommendation, enabled: true });
}

describe("recommendation history", () => {
  it("builds a draft whose items are the snapshot's order", async () => {
    const snapshot = await createSnapshot(parent(1));
    const draft = recommendationDraft(snapshot, "tenant-a", "subject-1", new Date("2026-11-26T03:30:00Z"));
    expect(draft).toMatchObject({
      objectId: "recommendation:tenant-a/subject-1",
      family: "recommendation",
      class: "recommendation",
      capturedAt: "2026-11-26T03:30:00.000Z",
      inputsRef: `parent-snapshot:${snapshot.parent_snapshot_id}`,
    });
    expect(draft.payload.items).toEqual(snapshot.ordered.map(({ id, score_micros }) => ({ id, score_micros })));
    expect(draft.payload.items.length).toBe(12); // the cutoff drops two arrivals
    expect(draft.payload.rule_id).toBe("score-desc-id-asc-v1");
    expect(recommendationObjectId("a/b", "c d")).toBe("recommendation:a%2Fb/c%20d");
  });

  it("records nothing without a binding or while the class is disabled", async () => {
    const snapshot = await createSnapshot(parent(1));
    expect(await recordRecommendation(undefined, snapshot, "tenant-a", "subject-1")).toEqual({
      recorded: false,
      reason: "off",
    });
    expect(await recordRecommendation(db, snapshot, "tenant-a", "subject-1")).toEqual({
      recorded: false,
      reason: "off",
    });
    const started = await start(accumulator(db), parent(1));
    expect(started.started).toBe(true);
    expect(started.retained_version).toBeUndefined();
    expect(await store.history(recommendationObjectId("tenant-a", "subject-1"))).toEqual([]);
  });

  it("keeps one version per recompute, and each reproduces from its inputs", async () => {
    await enable();
    const target = accumulator(db);
    const parents = [parent(1), parent(2), parent(3)];
    for (const [index, value] of parents.entries()) {
      const started = await start(target, value);
      expect(started.retained_version).toBe(index + 1);
      // Starting the same parent again is not a recompute.
      const again = await start(target, value);
      expect(again.started).toBe(false);
      expect(again.retained_version).toBeUndefined();
    }
    const history = await store.history(recommendationObjectId("tenant-a", "subject-1"));
    expect(history.map((version) => version.version)).toEqual([1, 2, 3]);
    for (const [index, value] of parents.entries()) {
      const snapshot = await createSnapshot(value);
      expect(reproduces(history[index]!, snapshot)).toBe(true);
      expect(history[index]!.inputsRef).toBe(`parent-snapshot:${snapshot.parent_snapshot_id}`);
      // A different recompute does not reproduce this version.
      const other = await createSnapshot(parents[(index + 1) % parents.length]!);
      expect(reproduces(history[index]!, other)).toBe(false);
    }
    const payload = history[2]!.payload as RecommendationPayload;
    expect(payload.input_digest).toBe("sha256:tenant-a:subject-1:input-3");
  });

  it("never fails a start when the history store is unusable", async () => {
    await enable();
    const broken = {
      prepare() {
        throw new Error("d1 unavailable");
      },
    } as unknown as D1Database;
    const started = await start(accumulator(broken), parent(1));
    expect(started.started).toBe(true);
    expect(started.retained_version).toBeUndefined();
  });
});
