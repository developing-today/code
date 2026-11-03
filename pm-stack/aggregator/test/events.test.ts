// #1739: the cross-stack event log — one append-only chronological
// stream over the aggregator's own facts. Covers the store
// (append/read round-trip, newest-first order), the kind/ref/actor
// filters, the retention bounds (30 days / 5000 records, whichever
// first), and the fire-and-forget contract: a failing event write
// never blocks or fails the underlying action.
import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  EVENT_RETENTION_MAX,
  EventsStore,
  emitEvent,
  filterEvents,
  pruneEvents,
  validateStreamEvent,
  type StreamEvent,
} from "../src/events.ts";

function tmpStore(): { file: string; store: EventsStore } {
  const dir = mkdtempSync(join(tmpdir(), "agg-events-"));
  const file = join(dir, "events.json");
  return { file, store: new EventsStore(file) };
}

const base = { actor: "drew", kind: "action-applied", ref: "gh:developing-today/code#1", summary: "did a thing" };

describe("events store (#1739)", () => {
  test("append -> reload round-trip persists events, newest first", async () => {
    const { file, store } = tmpStore();
    // Relative to now (#4489 Codex re-review): record() prunes against
    // the real clock, so fixed calendar dates would age out of the
    // 30-day retention window and this test would start failing
    // without any code change.
    const firstAt = new Date(Date.now() - 2 * 60 * 60 * 1000).toISOString();
    const secondAt = new Date(Date.now() - 60 * 60 * 1000).toISOString();
    await store.record({ ...base, at: firstAt });
    await store.record({ ...base, at: secondAt, summary: "later thing" });
    const reloaded = new EventsStore(file);
    const list = await reloaded.list();
    expect(list.map((e) => e.summary)).toEqual(["later thing", "did a thing"]);
    expect(list[0]?.at).toBe(secondAt);
  });

  test("record stamps `at` when the writer omits it", async () => {
    const { store } = tmpStore();
    const e = await store.record({ ...base });
    expect(Number.isFinite(Date.parse(e.at))).toBe(true);
  });

  test("kind / ref / actor filters each narrow the stream", async () => {
    const { store } = tmpStore();
    // Relative to now, like the round-trip test above: record() prunes
    // against the real clock (30-day retention).
    const t0 = Date.now();
    await store.record({ ...base, at: new Date(t0 - 3000).toISOString() });
    await store.record({
      actor: "orchestrator",
      kind: "tier-change",
      ref: "gh:developing-today/code#2",
      summary: "moved tier",
      at: new Date(t0 - 2000).toISOString(),
    });
    await store.record({
      actor: "keeper",
      kind: "session-outcome",
      ref: "session:opencode-laptop:abc",
      summary: "session merged",
      at: new Date(t0 - 1000).toISOString(),
    });
    expect((await store.list({ kind: "tier-change" })).map((e) => e.ref)).toEqual(["gh:developing-today/code#2"]);
    expect((await store.list({ ref: "gh:developing-today/code#1" })).length).toBe(1);
    expect((await store.list({ actor: "KEEPER" })).map((e) => e.kind)).toEqual(["session-outcome"]);
    expect((await store.list({ kind: "tier-change", actor: "drew" })).length).toBe(0);
    expect((await store.list()).length).toBe(3);
  });

  test("filterEvents is pure and newest-first", () => {
    const events: StreamEvent[] = [
      { ...base, at: "2026-11-01T10:00:00.000Z" },
      { ...base, at: "2026-11-01T12:00:00.000Z", summary: "newest" },
    ];
    expect(filterEvents(events)[0]?.summary).toBe("newest");
    expect(events[0]?.summary).toBe("did a thing"); // input untouched
  });

  test("retention: records older than 30 days are dropped on append", async () => {
    const { store } = tmpStore();
    const old = new Date(Date.now() - 31 * 24 * 60 * 60 * 1000).toISOString();
    await store.record({ ...base, at: old, summary: "ancient" });
    await store.record({ ...base, summary: "fresh" });
    const list = await store.list();
    expect(list.map((e) => e.summary)).toEqual(["fresh"]);
  });

  test("retention: the stream is capped at the newest 5000 records", () => {
    const now = new Date("2026-11-01T00:00:00.000Z").getTime();
    const many: StreamEvent[] = Array.from({ length: EVENT_RETENTION_MAX + 10 }, (_, i) => ({
      ...base,
      at: new Date(now + i * 1000).toISOString(),
      summary: `e${i}`,
    }));
    const pruned = pruneEvents(many, new Date(now + (EVENT_RETENTION_MAX + 10) * 1000));
    expect(pruned.length).toBe(EVENT_RETENTION_MAX);
    expect(pruned[0]?.summary).toBe("e10");
    expect(pruned[pruned.length - 1]?.summary).toBe(`e${EVENT_RETENTION_MAX + 9}`);
  });

  test("retention chooses newest timestamps, not latest arrivals", () => {
    const now = Date.parse("2026-11-01T00:00:00.000Z");
    const newest = Array.from({ length: EVENT_RETENTION_MAX }, (_, i) => ({
      ...base,
      at: new Date(now + i * 1000).toISOString(),
      summary: `new-${i}`,
    }));
    const oldArrivals = Array.from({ length: 10 }, (_, i) => ({
      ...base,
      at: new Date(now - (10 - i) * 1000).toISOString(),
      summary: `old-${i}`,
    }));
    const pruned = pruneEvents([...newest, ...oldArrivals], new Date(now + 10_000));
    expect(pruned).toHaveLength(EVENT_RETENTION_MAX);
    expect(pruned[0]?.summary).toBe("new-0");
    expect(pruned.at(-1)?.summary).toBe(`new-${EVENT_RETENTION_MAX - 1}`);
  });

  test("publication identity lookup survives a fresh store instance", async () => {
    const { file, store } = tmpStore();
    await store.record({
      actor: "orchestrator",
      kind: "orchestration-published",
      ref: "orchestration:run-1",
      summary: "published run-1",
    });
    expect(await new EventsStore(file).has("orchestration-published", "orchestration:run-1")).toBe(true);
    expect(await new EventsStore(file).has("orchestration-published", "orchestration:run-2")).toBe(false);
  });

  test("filterEvents orders by parsed time, not timestamp string", () => {
    // "2026-11-01T10:00:00Z" is lexicographically AFTER the later
    // "2026-11-01T10:00:00.500Z"; parsed-time order must win.
    const events: StreamEvent[] = [
      { ...base, at: "2026-11-01T10:00:00.500Z", summary: "later instant" },
      { ...base, at: "2026-11-01T10:00:00Z", summary: "earlier instant" },
    ];
    expect(filterEvents(events).map((e) => e.summary)).toEqual(["later instant", "earlier instant"]);
  });

  test("identities the action-result contract accepts are valid events", async () => {
    // byRun may carry punctuation (stored as appliedByRun); objectRef
    // may run to 300 chars (isObjectRef). Neither may be lost here.
    const at = new Date().toISOString();
    expect(validateStreamEvent({ ...base, at, actor: "orchestrator (retry 2)" }).ok).toBe(true);
    expect(validateStreamEvent({ ...base, at, ref: `gh:${"a".repeat(295)}` }).ok).toBe(true);
    expect(validateStreamEvent({ ...base, at, ref: `gh:${"a".repeat(299)}` }).ok).toBe(false);
    expect(validateStreamEvent({ ...base, at, ref: "has whitespace" }).ok).toBe(false);
    const { store } = tmpStore();
    const recorded = await store.record({ ...base, actor: "orchestrator (retry 2)" });
    expect(recorded.actor).toBe("orchestrator (retry 2)");
  });

  test("validateStreamEvent rejects malformed records", () => {
    expect(validateStreamEvent({ ...base, at: "2026-11-01T10:00:00.000Z" }).ok).toBe(true);
    expect(validateStreamEvent({ ...base, at: "not a date" }).ok).toBe(false);
    expect(validateStreamEvent({ ...base, actor: "" }).ok).toBe(false);
    expect(validateStreamEvent({ ...base, summary: "" }).ok).toBe(false);
    expect(validateStreamEvent(null).ok).toBe(false);
  });

  test("writer failure does not block the underlying action (fire-and-forget)", async () => {
    const failingStore = {
      record: async (): Promise<StreamEvent> => {
        throw new Error("disk full");
      },
    };
    let actionRan = false;
    const ok = await emitEvent(failingStore, { ...base });
    actionRan = true; // the underlying action proceeds after the failed write
    expect(ok).toBe(false);
    expect(actionRan).toBe(true);
  });

  test("emitEvent reports success when the append lands", async () => {
    const { store } = tmpStore();
    expect(await emitEvent(store, { ...base })).toBe(true);
    expect((await store.list()).length).toBe(1);
  });
});
