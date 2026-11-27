/**
 * Durable recommendation accumulator conformance tests (C9139-02).
 *
 * Oracle: a finite reference model from C9139-01 (`createSnapshot` plus
 * `makePage`) and durable-storage fault injection. Every emitted EnvelopeId
 * is independently recomputed from its complete signed envelope; the fake
 * signer is deterministic test machinery and does not claim cryptographic
 * verification or production signing state.
 *
 * The #15003 split-key layout has its own oracle: the recorded put batches.
 * A page advance must commit exactly the new page key and the cursor key —
 * never the parent or order keys — and each batch must stay smaller than
 * the total order the v1 layout rewrote on every advance.
 */
import { describe, expect, it } from "vitest";
import {
  createSnapshot,
  hexToBytes,
  pageEnvelopeId,
  type PageEnvelope,
  type ParentEnvelope,
} from "../src/recommendation_state";
import { RecommendationAccumulator } from "../src/recommendation_do";

const encoder = new TextEncoder();
const hex = (bytes: Uint8Array) => [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");

const LEGACY_KEY = "recommendation-accumulator-v1";
const PARENT_KEY = "recommendation-accumulator-v2:parent";
const ORDER_KEY = "recommendation-accumulator-v2:order";
const CURSOR_KEY = "recommendation-accumulator-v2:cursor";
const pageKey = (position: number) => `recommendation-accumulator-v2:page:${position}`;

class MemoryStorage {
  private values = new Map<string, unknown>();
  failBeforePut = false;
  failAfterPut = false;
  /** Every committed put batch, keyed exactly as written. */
  readonly putBatches: Array<Record<string, unknown>> = [];

  async get<T>(key: string): Promise<T | undefined> {
    const value = this.values.get(key);
    return value === undefined ? undefined : (structuredClone(value) as T);
  }

  async put<T>(key: string | Record<string, unknown>, value?: T): Promise<void> {
    const entries: Record<string, unknown> = typeof key === "string" ? { [key]: value } : key;
    if (this.failBeforePut) {
      this.failBeforePut = false;
      throw new Error("injected crash before durable commit");
    }
    for (const [entryKey, entryValue] of Object.entries(entries)) {
      this.values.set(entryKey, structuredClone(entryValue));
    }
    this.putBatches.push(structuredClone(entries));
    if (this.failAfterPut) {
      this.failAfterPut = false;
      throw new Error("injected lost acknowledgement after durable commit");
    }
  }

  async delete(keys: string | string[]): Promise<void> {
    for (const key of Array.isArray(keys) ? keys : [keys]) {
      this.values.delete(key);
    }
  }

  keys(): string[] {
    return [...this.values.keys()];
  }
}

function signer(counter: { calls: number }) {
  return {
    async fetch(_input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
      counter.calls++;
      const request = JSON.parse(String(init?.body)) as { message_hex: string };
      const bytes = Uint8Array.from(request.message_hex.match(/.{2}/g) ?? [], (part) => Number.parseInt(part, 16));
      const digest = await crypto.subtle.digest("SHA-256", bytes);
      return Response.json({ signature_hex: hex(new Uint8Array(digest)) });
    },
  };
}

function parent(tenant = "tenant-a", subject = "subject-1"): ParentEnvelope {
  return {
    body: {
      type: "recommendation-parent-v1",
      tenant,
      subject,
      algorithm: "score-desc-id-asc-v1",
      cutoff_sequence: 12,
      page_size: 10,
      input_digest: `sha256:${tenant}:${subject}:input-12`,
      candidate_set: Array.from({ length: 12 }, (_, index) => ({
        id: `rec-${String(index).padStart(2, "0")}`,
        score_micros: 1000 - Math.floor(index / 2),
        arrival_sequence: index + 1,
      })),
    } as ParentEnvelope["body"],
    signature: encoder.encode(`parent-signature:${tenant}:${subject}`),
  };
}

function bigParent(count: number): ParentEnvelope {
  return {
    body: {
      type: "recommendation-parent-v1",
      tenant: "tenant-a",
      subject: "subject-1",
      algorithm: "score-desc-id-asc-v1",
      cutoff_sequence: count,
      page_size: 10,
      input_digest: `sha256:tenant-a:subject-1:input-${count}`,
      candidate_set: Array.from({ length: count }, (_, index) => ({
        id: `rec-${String(index).padStart(3, "0")}`,
        score_micros: 1_000_000 - index,
        arrival_sequence: index + 1,
      })),
    } as ParentEnvelope["body"],
    signature: encoder.encode("parent-signature:tenant-a:subject-1"),
  };
}

function instance(storage: MemoryStorage, counter = { calls: 0 }) {
  const state = { storage } as unknown as DurableObjectState;
  return { accumulator: new RecommendationAccumulator(state, { PAGE_SIGNER: signer(counter) }), counter };
}

async function start(accumulator: RecommendationAccumulator, value: ParentEnvelope) {
  const response = await accumulator.fetch(
    new Request("https://do/start", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ parent: value }),
    }),
  );
  if (!response.ok) throw new Error(JSON.stringify(await response.json()));
  return response.json() as Promise<{ parent_snapshot_id: string; cursor: { position: number } }>;
}

async function next(accumulator: RecommendationAccumulator, tenant: string, subject: string, cursor?: unknown) {
  const response = await accumulator.fetch(
    new Request("https://do/next", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ tenant, subject, ...(cursor === undefined ? {} : { cursor }) }),
    }),
  );
  return {
    response,
    body: (await response.json()) as {
      envelope?: { body: PageEnvelope["body"]; signature_hex: string };
      envelope_id?: string;
      next_cursor?: { parent_snapshot_id: string; position: number };
      error?: { code: string };
    },
  };
}

function envelopeFromWire(page: Awaited<ReturnType<typeof next>>["body"]) {
  const envelope: PageEnvelope = {
    body: page.envelope!.body,
    signature: hexToBytes(page.envelope!.signature_hex),
  };
  return envelope;
}

describe("durable recommendation accumulator (oracle: C9139-01 finite model + write fault injection)", () => {
  it("pins the model order, persists parent/cursor, and replays independently identified pages after restart", async () => {
    const storage = new MemoryStorage();
    const first = instance(storage);
    const input = parent();
    const model = await createSnapshot(input);
    const started = await start(first.accumulator, input);
    expect(started.parent_snapshot_id).toBe(model.parent_snapshot_id);

    const page0 = await next(first.accumulator, "tenant-a", "subject-1");
    expect(page0.response.status).toBe(200);
    expect(page0.body.envelope?.body.entries.map((entry) => entry.id)).toEqual(
      model.ordered.slice(0, 10).map((entry) => entry.id),
    );
    expect(page0.body.envelope_id).toBe(await pageEnvelopeId(envelopeFromWire(page0.body)));
    expect(page0.body.envelope?.body.done).toBe(false);
    expect(page0.body.next_cursor).toEqual({ parent_snapshot_id: model.parent_snapshot_id, position: 10 });

    // A new instance models process eviction/restart over the same durable state.
    const restarted = instance(storage);
    const replay = await next(restarted.accumulator, "tenant-a", "subject-1");
    expect(replay.body).toEqual(page0.body);
    expect(restarted.counter.calls).toBe(0);

    const page1 = await next(restarted.accumulator, "tenant-a", "subject-1", page0.body.next_cursor);
    expect(page1.response.status).toBe(200);
    expect(page1.body.envelope?.body.entries.map((entry) => entry.id)).toEqual(
      model.ordered.slice(10).map((entry) => entry.id),
    );
    expect(page1.body.envelope?.body.done).toBe(true);
    expect(page1.body.envelope?.body.parent_snapshot_id).toBe(model.parent_snapshot_id);
    expect(page1.body.envelope_id).toBe(await pageEnvelopeId(envelopeFromWire(page1.body)));
    expect(page1.body.envelope_id).not.toBe(page0.body.envelope_id);
  });

  it("retries identically after a crash before commit or a lost acknowledgement after commit", async () => {
    const storage = new MemoryStorage();
    const counter = { calls: 0 };
    const { accumulator } = instance(storage, counter);
    const input = parent();

    storage.failBeforePut = true;
    await expect(start(accumulator, input)).rejects.toThrow("before durable commit");
    const started = await start(accumulator, input);
    const model = await createSnapshot(input);
    expect(started.parent_snapshot_id).toBe(model.parent_snapshot_id);

    storage.failBeforePut = true;
    const failedPage = await next(accumulator, "tenant-a", "subject-1");
    expect(failedPage.response.status).toBe(500);
    storage.failAfterPut = true;
    const lostAck = await next(accumulator, "tenant-a", "subject-1");
    expect(lostAck.response.status).toBe(500);

    const restarted = instance(storage);
    const replay = await next(restarted.accumulator, "tenant-a", "subject-1");
    expect(replay.response.status).toBe(200);
    expect(replay.body.envelope_id).toBe(await pageEnvelopeId(envelopeFromWire(replay.body)));
    expect(replay.body.envelope_id).toBe((await next(restarted.accumulator, "tenant-a", "subject-1")).body.envelope_id);
    expect(counter.calls).toBe(2); // before-commit retry signs again; committed replay does not.
  });

  it("rejects cross-tenant/subject access and wrong-parent or expired cursors", async () => {
    const storage = new MemoryStorage();
    const { accumulator } = instance(storage);
    const input = parent();
    const model = await createSnapshot(input);
    await start(accumulator, input);

    const crossTenant = await next(accumulator, "tenant-b", "subject-1");
    expect(crossTenant.response.status).toBe(403);
    const crossSubject = await next(accumulator, "tenant-a", "subject-2");
    expect(crossSubject.response.status).toBe(403);
    const wrongParent = await next(accumulator, "tenant-a", "subject-1", {
      parent_snapshot_id: "sha256:wrong-parent",
      position: 0,
    });
    expect(wrongParent.response.status).toBe(400);
    expect(wrongParent.body.error?.code).toBe("WrongParentSnapshotId");
    const expired = await next(accumulator, "tenant-a", "subject-1", {
      parent_snapshot_id: model.parent_snapshot_id,
      position: 0,
      expires_at: 1,
    });
    expect(expired.response.status).toBe(410);
    expect(expired.body.error?.code).toBe("CursorExpired");
  });

  it("keeps separate tenant/subject partitions isolated", async () => {
    const aStorage = new MemoryStorage();
    const bStorage = new MemoryStorage();
    const a = instance(aStorage);
    const b = instance(bStorage);
    const aParent = parent("tenant-a", "subject-1");
    const bParent = parent("tenant-b", "subject-1");
    const [aStarted, bStarted] = await Promise.all([start(a.accumulator, aParent), start(b.accumulator, bParent)]);
    expect(aStarted.parent_snapshot_id).not.toBe(bStarted.parent_snapshot_id);
    const [aPage, bPage] = await Promise.all([
      next(a.accumulator, "tenant-a", "subject-1"),
      next(b.accumulator, "tenant-b", "subject-1"),
    ]);
    expect(aPage.body.envelope?.body.parent_snapshot_id).toBe(aStarted.parent_snapshot_id);
    expect(bPage.body.envelope?.body.parent_snapshot_id).toBe(bStarted.parent_snapshot_id);
    expect(aPage.body.envelope_id).not.toBe(bPage.body.envelope_id);
  });

  it("advances a page by writing only the new page and the cursor, never the parent or order (#15003)", async () => {
    const storage = new MemoryStorage();
    const { accumulator } = instance(storage);
    const input = bigParent(200);
    const model = await createSnapshot(input);
    await start(accumulator, input);

    // start() wrote the immutable parts once, under their own keys, and
    // never touched the legacy single key.
    const startBatches = storage.putBatches.length;
    expect(startBatches).toBeGreaterThan(0);
    const startKeys = storage.putBatches.flatMap((batch) => Object.keys(batch));
    expect(startKeys).toContain(PARENT_KEY);
    expect(startKeys).toContain(ORDER_KEY);
    expect(storage.keys()).not.toContain(LEGACY_KEY);

    let cursor: unknown;
    for (let pageIndex = 0; pageIndex < 20; pageIndex++) {
      const result = await next(accumulator, "tenant-a", "subject-1", cursor);
      expect(result.response.status).toBe(200);
      cursor = result.body.next_cursor;
    }

    const orderBytes = JSON.stringify(model.ordered).length;
    const advanceBatches = storage.putBatches.slice(startBatches);
    expect(advanceBatches).toHaveLength(20);
    for (const batch of advanceBatches) {
      const keys = Object.keys(batch).sort();
      expect(keys).toHaveLength(2);
      expect(keys).toContain(CURSOR_KEY);
      expect(keys).toContain(
        pageKey(
          Number(
            keys
              .find((key) => key !== CURSOR_KEY)!
              .split(":")
              .at(-1),
          ),
        ),
      );
      // Each advance batch is smaller than the total order alone, which
      // the v1 layout re-serialized and rewrote on every advance.
      expect(JSON.stringify(batch).length).toBeLessThan(orderBytes);
    }
  });

  it("prunes replay pages beyond the retained horizon and deletes their keys", async () => {
    const storage = new MemoryStorage();
    const { accumulator } = instance(storage);
    const started = await start(accumulator, bigParent(200));
    let cursor: unknown;
    for (let pageIndex = 0; pageIndex < 20; pageIndex++) {
      const result = await next(accumulator, "tenant-a", "subject-1", cursor);
      expect(result.response.status).toBe(200);
      cursor = result.body.next_cursor;
    }
    expect(storage.keys()).not.toContain(pageKey(0));
    expect(storage.keys()).toContain(pageKey(190));

    const expired = await next(accumulator, "tenant-a", "subject-1", {
      parent_snapshot_id: started.parent_snapshot_id,
      position: 0,
    });
    expect(expired.response.status).toBe(410);
    expect(expired.body.error?.code).toBe("RecommendationReplayExpired");

    const replay = await next(accumulator, "tenant-a", "subject-1", {
      parent_snapshot_id: started.parent_snapshot_id,
      position: 190,
    });
    expect(replay.response.status).toBe(200);
    expect(replay.body.envelope?.body.done).toBe(true);
  });

  it("migrates a v1 single-key accumulator on first read and never writes it again", async () => {
    const storage = new MemoryStorage();
    const input = parent();
    const model = await createSnapshot(input);
    const page0Envelope: PageEnvelope = {
      body: {
        parent_snapshot_id: model.parent_snapshot_id,
        position: 0,
        entries: model.ordered.slice(0, 10),
        done: false,
      },
      signature: encoder.encode("page-signature-0"),
    };
    const page0 = {
      envelope: page0Envelope,
      envelope_id: await pageEnvelopeId(page0Envelope),
      next_cursor: { parent_snapshot_id: model.parent_snapshot_id, position: 10 },
    };
    await storage.put(LEGACY_KEY, {
      schema_version: 1,
      tenant: "tenant-a",
      subject: "subject-1",
      parent: input,
      parent_snapshot_id: model.parent_snapshot_id,
      ordered: model.ordered,
      cursor_position: 10,
      pages: { "0": page0 },
    });

    const { accumulator, counter } = instance(storage);
    const replay = await next(accumulator, "tenant-a", "subject-1");
    expect(replay.response.status).toBe(200);
    expect(replay.body.envelope_id).toBe(page0.envelope_id);
    expect(counter.calls).toBe(0); // migrated replay is served, not re-signed
    expect(storage.keys()).not.toContain(LEGACY_KEY);
    expect(storage.keys()).toContain(pageKey(0));
    expect(storage.keys()).toContain(PARENT_KEY);

    const page1 = await next(accumulator, "tenant-a", "subject-1", replay.body.next_cursor);
    expect(page1.response.status).toBe(200);
    expect(page1.body.envelope?.body.done).toBe(true);
    expect(counter.calls).toBe(1); // only the genuinely new page is signed
  });
});
