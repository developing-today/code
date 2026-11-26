// Write-amplification regression (#14865): the per-set Durable Object
// used to rewrite its entire seen-event-id array (up to 1,000 ids) on
// every ingest batch carrying even one new event. Oracle: after the
// seen set is at the cap, ingesting one new event must write only
// O(1) small values — never the whole seen set — while dedup and the
// 1,000-id retention cap keep their exact previous behaviour.
import { describe, expect, test } from "bun:test";
import { DecisionSetEvents } from "../src/index.ts";

interface PutRecord {
  keys: string[];
  bytes: number;
}

class FakeStorage {
  data = new Map<string, unknown>();
  puts: PutRecord[] = [];

  async get<T>(key: string | string[]): Promise<T | Map<string, T> | undefined> {
    if (Array.isArray(key)) {
      const out = new Map<string, T>();
      for (const k of key) if (this.data.has(k)) out.set(k, this.data.get(k) as T);
      return out;
    }
    return this.data.get(key) as T | undefined;
  }

  async put<T>(key: string | Record<string, T>, value?: T): Promise<void> {
    const entries: [string, unknown][] =
      typeof key === "string" ? [[key, value]] : Object.entries(key as Record<string, unknown>);
    let bytes = 0;
    for (const [k, v] of entries) {
      this.data.set(k, v);
      bytes += k.length + JSON.stringify(v).length;
    }
    this.puts.push({ keys: entries.map(([k]) => k), bytes });
  }

  async delete(key: string | string[]): Promise<number> {
    const keys = Array.isArray(key) ? key : [key];
    let n = 0;
    for (const k of keys) if (this.data.delete(k)) n += 1;
    return n;
  }

  async list<T>(options?: { prefix?: string }): Promise<Map<string, T>> {
    const out = new Map<string, T>();
    for (const k of [...this.data.keys()].sort()) {
      if (!options?.prefix || k.startsWith(options.prefix)) out.set(k, this.data.get(k) as T);
    }
    return out;
  }

  seenCount(): number {
    return [...this.data.keys()].filter((k) => k.startsWith("seen/")).length;
  }
}

function makeObject(storage: FakeStorage): DecisionSetEvents {
  const state = {
    storage,
    acceptWebSocket: () => {},
    getWebSockets: () => [],
  };
  return new DecisionSetEvents(state as unknown as DurableObjectState);
}

function ev(eventId: string): Record<string, unknown> {
  return {
    eventId,
    setId: "set-1",
    questionRef: "ledger-1#1",
    type: "question-answered",
    payload: { optionId: "a" },
    ts: "2026-11-25T00:00:00.000Z",
  };
}

async function ingest(obj: DecisionSetEvents, events: Record<string, unknown>[]): Promise<Record<string, unknown>> {
  const res = await obj.fetch(
    new Request("https://do/ingest", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ setId: "set-1", events }),
    }),
  );
  return (await res.json()) as Record<string, unknown>;
}

describe("seen-event store", () => {
  test("deduplicates redelivered events across batches", async () => {
    const storage = new FakeStorage();
    const obj = makeObject(storage);
    const first = await ingest(obj, [ev("e1"), ev("e2")]);
    expect(first.fannedOut).toBe(2);
    expect(first.deduplicated).toBe(0);
    const second = await ingest(obj, [ev("e2"), ev("e3")]);
    expect(second.fannedOut).toBe(3);
    expect(second.deduplicated).toBe(1);
    expect(second.lastEventId).toBe("e3");
  });

  test("one new event at the cap writes O(1) bytes, not the whole seen set", async () => {
    const storage = new FakeStorage();
    const obj = makeObject(storage);
    // Fill the seen set to the 1,000-id cap in batches of 100.
    for (let base = 0; base < 1000; base += 100) {
      await ingest(
        obj,
        Array.from({ length: 100 }, (_, i) => ev(`seed-${base + i}`)),
      );
    }
    expect(storage.seenCount()).toBe(1000);
    storage.puts = [];
    const res = await ingest(obj, [ev("one-new-event")]);
    expect(res.fannedOut).toBe(1001);
    // The legacy whole-array key is never written.
    expect(storage.puts.every((p) => !p.keys.includes("seenEventIds"))).toBe(true);
    // Total bytes written for one new event stay small: the old code
    // rewrote ~1,000 ids (~15 KB) here.
    const totalBytes = storage.puts.reduce((sum, p) => sum + p.bytes, 0);
    expect(totalBytes).toBeLessThan(1000);
    // Cap still enforced: exactly 1,000 seen keys, the new id retained.
    expect(storage.seenCount()).toBe(1000);
    expect(storage.data.has("seen/one-new-event")).toBe(true);
    expect(storage.data.has("seen/seed-0")).toBe(false);
  });

  test("migrates a legacy seenEventIds array once and keeps deduplicating", async () => {
    const storage = new FakeStorage();
    storage.data.set("seenEventIds", ["old-1", "old-2"]);
    const obj = makeObject(storage);
    const res = await ingest(obj, [ev("old-1"), ev("fresh")]);
    expect(res.deduplicated).toBe(1);
    expect(res.fannedOut).toBe(1);
    expect(storage.data.has("seenEventIds")).toBe(false);
    expect(storage.data.has("seen/old-1")).toBe(true);
    expect(storage.data.has("seen/old-2")).toBe(true);
    // And the migrated ids still dedup on the next batch.
    const again = await ingest(obj, [ev("old-2")]);
    expect(again.deduplicated).toBe(1);
  });
});
