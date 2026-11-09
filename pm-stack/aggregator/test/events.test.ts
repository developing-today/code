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
  pageEvents,
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

  test("a failed append cannot duplicate on retry: a stable id replays to the single record (PR #5141 codex; mechanism superseded by #3322 v2)", async () => {
    // The v1 mechanism (rolling the in-memory append back) was replaced
    // when main landed the #3322 v2 record() (PR #5137): on a persist
    // failure the record is queued in the durable retry outbox and kept
    // in memory, and a caller-supplied stable id makes a retry return
    // the existing record instead of appending a twin. The #3321
    // ingestion writer passes id = activity:<source>:<sourceId>.
    const dir = mkdtempSync(join(tmpdir(), "agg-events-blocked-"));
    const blocker = join(dir, "blocker");
    writeFileSync(blocker, "not a directory");
    const store = new EventsStore(join(blocker, "events.json"));
    await expect(store.record({ ...base, id: "activity:test:1" })).rejects.toThrow();
    // The ingestion retry re-presents the same stable id: it resolves
    // to the single retained record, never a second copy.
    const replay = await store.record({ ...base, id: "activity:test:1" });
    expect(replay.id).toBe("activity:test:1");
    const blockedList = await store.list();
    expect(blockedList).toHaveLength(1);
    expect(blockedList[0]?.id).toBe("activity:test:1");
    // Same contract on a healthy store: one stable id lands once.
    const { store: healthy } = tmpStore();
    const first = await healthy.record({ ...base, id: "activity:test:2" });
    const second = await healthy.record({ ...base, id: "activity:test:2" });
    expect(second.id).toBe(first.id);
    expect(await healthy.list()).toHaveLength(1);
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

  test("project, epic, ref and summary search filters share one matcher", () => {
    const events: StreamEvent[] = [
      { ...base, at: "2026-11-01T10:00:00Z", ref: "gh:developing-today/code#7", summary: "Moved to review" },
      { ...base, at: "2026-11-01T09:00:00Z", ref: "developing-today/code#8", summary: "Closed" },
    ];
    const itemContext = new Map([
      [
        "developing-today/code#7",
        { project: "https://github.com/orgs/developing-today/projects/1", epic: "developing-today/code#1" },
      ],
      ["developing-today/code#8", { project: "https://github.com/orgs/developing-today/projects/2", epic: null }],
    ]);
    expect(
      filterEvents(events, {
        refs: ["developing-today/code#7"],
        project: "https://github.com/orgs/developing-today/projects/1",
        epic: "developing-today/code#1",
        search: "REVIEW",
        itemContext,
      }).map((event) => event.ref),
    ).toEqual(["gh:developing-today/code#7"]);
  });

  test("cursor pages use stable id ordering and exclude events ingested after the snapshot", async () => {
    const { store } = tmpStore();
    const now = Date.now();
    await store.record({ ...base, id: "event-c", at: new Date(now - 3 * 60_000).toISOString(), summary: "oldest" });
    await store.record({ ...base, id: "event-b", at: new Date(now - 2 * 60_000).toISOString(), summary: "middle" });
    await store.record({ ...base, id: "event-a", at: new Date(now - 1 * 60_000).toISOString(), summary: "newest" });
    const first = pageEvents(await store.list(), { limit: 2 });
    expect(first.events.map((event) => event.id)).toEqual(["event-a", "event-b"]);
    expect(first.nextCursor).not.toBeNull();

    // A late-arriving old event sorts behind the cursor by event time,
    // but its ingestion time is beyond the first page's fixed snapshot.
    await store.record({
      ...base,
      id: "late-event",
      at: new Date(now - 5 * 60_000).toISOString(),
      summary: "arrived late",
    });
    const second = pageEvents(await store.list(), { limit: 2, cursor: first.nextCursor });
    expect(second.events.map((event) => event.id)).toEqual(["event-c"]);
    expect(second.events.map((event) => event.id)).not.toContain("late-event");
  });

  test("ingestedAt stamps are strictly monotonic across rapid appends (#3322, PR #7665)", async () => {
    // pageEvents snapshot exclusion is only exact if a later append
    // can never share the snapshot's newest ingestedAt; wall-clock
    // stamps tie at millisecond resolution, so record() must stamp
    // strictly past the previous record even within the same ms.
    const { store } = tmpStore();
    for (let i = 0; i < 5; i++) {
      await store.record({ ...base, id: `mono-${i}`, summary: `rapid ${i}` });
    }
    const stamps = (await store.list()).map((e) => Date.parse(e.ingestedAt)).sort((a, b) => a - b);
    for (let i = 1; i < stamps.length; i++) {
      expect(stamps[i]).toBeGreaterThan(stamps[i - 1]!);
    }
  });

  test("pageEvents compares ingestion timestamps as instants, not strings", () => {
    // Migrated v1 rows keep offset timestamps as ingestedAt: the first
    // row below is 07:00Z even though its string sorts after the native
    // 08:00Z row. Lexicographic comparison would pick the wrong
    // snapshot and wrongly exclude the migrated row from later pages.
    const mk = (id: string, at: string, ingestedAt: string) => ({
      ...base,
      version: 2 as const,
      id,
      at,
      ingestedAt,
      attribution: { confidence: "unknown" as const, asserted: false },
      source: { identity: "test" },
      prevHash: "",
      hash: "",
    });
    const migrated = mk("migrated", "2026-11-04T06:00:00.000Z", "2026-11-04T15:00:00+08:00");
    const native = mk("native", "2026-11-04T07:30:00.000Z", "2026-11-04T08:00:00.000Z");
    const first = pageEvents([native, migrated], { limit: 1 });
    expect(first.snapshotAt).toBe("2026-11-04T08:00:00.000Z");
    const second = pageEvents([native, migrated], { limit: 1, cursor: first.nextCursor });
    expect(second.events.map((event) => event.id)).toEqual(["migrated"]);
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

  test("validateStreamEvent refuses impossible calendar dates (#5154)", () => {
    // Date.parse rolls impossible dates over and accepts locale-dependent
    // non-ISO forms; the ingest boundary must refuse both with its ISO error.
    for (const at of [
      "2026-02-30",
      "2026-02-30T10:00:00Z",
      "2026-04-31T10:00:00Z",
      "March 2, 2026",
      "03/02/2026",
      "2026-13-01T00:00:00Z",
      "2026-02-29T10:00:00Z",
    ]) {
      expect(validateStreamEvent({ ...base, at })).toEqual({
        ok: false,
        error: "event.at must be an ISO timestamp string",
      });
    }
    // Controls: real timestamps still validate — leap date, offset,
    // fractional seconds, date-only.
    expect(validateStreamEvent({ ...base, at: "2024-02-29T10:00:00Z" }).ok).toBe(true);
    expect(validateStreamEvent({ ...base, at: "2026-11-01T10:00:00+02:00" }).ok).toBe(true);
    expect(validateStreamEvent({ ...base, at: "2026-11-01T10:00:00.500Z" }).ok).toBe(true);
    expect(validateStreamEvent({ ...base, at: "2026-02-28" }).ok).toBe(true);
  });

  test("legacy event files with impossible or non-ISO timestamps are quarantined", async () => {
    for (const at of ["2026-02-30T10:00:00Z", "2026-04-31T10:00:00Z", "March 2, 2026", "03/02/2026"]) {
      const dir = mkdtempSync(join(tmpdir(), "agg-events-invalid-at-"));
      const file = join(dir, "events.json");
      const original = JSON.stringify({ version: 1, events: [{ ...base, at }] }, null, 2);
      writeFileSync(file, original);

      expect(await new EventsStore(file).list()).toEqual([]);
      const quarantined = readdirSync(dir).find((name) => name.startsWith("events.json.corrupt-"));
      expect(quarantined).toBeDefined();
      expect(readFileSync(join(dir, quarantined!), "utf8")).toBe(original);
    }
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

// #3322 slice A: audit v2 — versioned records with stable ids,
// attribution confidence, separate event/ingest timestamps, source
// identity, hash-linked integrity, append-only corrections, v1
// dual-read, and the durable retry outbox.
import { readFileSync, writeFileSync, mkdirSync, rmSync, existsSync, readdirSync } from "node:fs";
import { verifyEventChain } from "../src/events.ts";

describe("events audit v2 (#3322 slice A)", () => {
  test("record stamps v2 audit fields: stable id, ingest time, attribution, source, hash link", async () => {
    const { store } = tmpStore();
    const at = new Date(Date.now() - 60_000).toISOString();
    const e = await store.record({ ...base, at });
    expect(e.version).toBe(2);
    expect(typeof e.id).toBe("string");
    expect(e.id.length).toBeGreaterThan(0);
    // Event time is the supplied `at`; ingest time is stamped by the
    // store and is not the event time.
    expect(e.at).toBe(at);
    expect(Number.isFinite(Date.parse(e.ingestedAt))).toBe(true);
    expect(e.ingestedAt).not.toBe(at);
    // Unknown actors are marked unknown and never asserted.
    expect(e.attribution).toEqual({ confidence: "unknown", asserted: false });
    expect(e.source.identity.length).toBeGreaterThan(0);
    expect(e.hash.length).toBe(64);
    const second = await store.record({ ...base, summary: "second" });
    expect(second.prevHash).toBe(e.hash);
    expect(verifyEventChain(await store.list().then((l) => [...l].reverse())).ok).toBe(true);
    const health = await store.health();
    expect(health.integrity.ok).toBe(true);
    expect(health.pendingRetries).toBe(0);
    expect(health.events).toBe(2);
  });

  test("authenticated attribution must be asserted; unasserted claims are rejected", async () => {
    const { store } = tmpStore();
    const ok = await store.record({
      ...base,
      attribution: { confidence: "authenticated", asserted: true },
      source: { identity: "dashboard", link: "https://example.test/x" },
    });
    expect(ok.attribution.confidence).toBe("authenticated");
    expect(ok.source.link).toBe("https://example.test/x");
    await expect(
      store.record({ ...base, attribution: { confidence: "authenticated", asserted: false } }),
    ).rejects.toThrow("asserted");
  });

  test("v1 store files remain readable and migrate with deterministic ids", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-events-v1-"));
    const file = join(dir, "events.json");
    const at = new Date(Date.now() - 60_000).toISOString();
    writeFileSync(file, JSON.stringify({ version: 1, events: [{ ...base, at }] }, null, 2));
    const first = await new EventsStore(file).list();
    expect(first.length).toBe(1);
    expect(first[0]?.version).toBe(2);
    expect(first[0]?.migratedFromVersion).toBe(1);
    expect(first[0]?.attribution.confidence).toBe("unknown");
    const second = await new EventsStore(file).list();
    expect(second[0]?.id).toBe(first[0]?.id); // deterministic migration id
    // The next append persists the file as v2.
    const store = new EventsStore(file);
    await store.record({ ...base, summary: "after migration" });
    const onDisk = JSON.parse(readFileSync(file, "utf8")) as { version: number };
    expect(onDisk.version).toBe(2);
  });

  test("a silently edited stored record fails the integrity check", async () => {
    const { file, store } = tmpStore();
    await store.record({ ...base });
    await store.record({ ...base, summary: "second" });
    const onDisk = JSON.parse(readFileSync(file, "utf8")) as { events: { summary: string }[] };
    onDisk.events[0]!.summary = "tampered";
    writeFileSync(file, JSON.stringify(onDisk, null, 2));
    const reloaded = new EventsStore(file);
    const health = await reloaded.health();
    expect(health.integrity.ok).toBe(false);
    // The stream itself stays readable; the violation is surfaced,
    // not hidden.
    expect((await reloaded.list()).length).toBe(2);
  });

  test("corrections are separate append-only records naming their target", async () => {
    const { store } = tmpStore();
    const original = await store.record({ ...base, summary: "original claim" });
    const correction = await store.recordCorrection(original.id, {
      actor: "keeper",
      summary: "correction: the claim above was about code#2",
    });
    expect(correction.kind).toBe("correction");
    expect(correction.corrects).toBe(original.id);
    const list = await store.list();
    expect(list.length).toBe(2);
    expect(list.find((e) => e.id === original.id)?.summary).toBe("original claim"); // untouched
    await expect(store.recordCorrection("no-such-id", { actor: "keeper", summary: "x" })).rejects.toThrow(
      "no such record",
    );
  });

  test("a failed append is persisted to the retry outbox and merged on the next append", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-events-retry-"));
    const file = join(dir, "events.json");
    const store = new EventsStore(file);
    await store.record({ ...base, summary: "landed" });
    // Make persist fail: the events path becomes a non-empty
    // directory, so the atomic rename over it fails — while the
    // sibling outbox file stays writable.
    rmSync(file);
    mkdirSync(file);
    writeFileSync(join(file, "occupant"), "x");
    await expect(store.record({ ...base, summary: "queued for retry" })).rejects.toThrow();
    expect(existsSync(store.outboxFile)).toBe(true);
    expect(await store.health().then((h) => h.pendingRetries)).toBe(1);
    // Recover and append again: the outbox flushes first.
    rmSync(file, { recursive: true });
    await store.record({ ...base, summary: "after recovery" });
    const summaries = (await new EventsStore(file).list()).map((e) => e.summary);
    expect(summaries).toContain("queued for retry");
    expect(summaries).toContain("after recovery");
    expect(summaries).toContain("landed");
    expect(existsSync(store.outboxFile)).toBe(false);
  });

  test("outbox records already in memory are not cleared until a persist succeeds", async () => {
    // Codex re-review on PR #5137 (P1): record() keeps a failed event
    // in memory, so the next append's flush saw it as "known", skipped
    // the persist, and cleared the outbox — a second failure then
    // re-queued only the newer event and a restart lost the first.
    const dir = mkdtempSync(join(tmpdir(), "agg-events-outbox-persist-"));
    const file = join(dir, "events.json");
    const store = new EventsStore(file);
    await store.record({ ...base, summary: "landed" });
    rmSync(file);
    mkdirSync(file);
    writeFileSync(join(file, "occupant"), "x");
    await expect(store.record({ ...base, summary: "first failure" })).rejects.toThrow();
    await expect(store.record({ ...base, summary: "second failure" })).rejects.toThrow();
    // Both failures must still be queued: the first was never made
    // durable, so its outbox entry may not be cleared.
    expect(await store.health().then((h) => h.pendingRetries)).toBe(2);
    rmSync(file, { recursive: true });
    await store.record({ ...base, summary: "after recovery" });
    const summaries = (await new EventsStore(file).list()).map((e) => e.summary);
    expect(summaries).toContain("first failure");
    expect(summaries).toContain("second failure");
    expect(summaries).toContain("after recovery");
    expect(summaries).toContain("landed");
    expect(existsSync(store.outboxFile)).toBe(false);
  });

  test("a chain that failed verification is not re-stamped by a later append", async () => {
    // Codex re-review on PR #5137 (P2): persist() re-stamped every
    // hash, so one ordinary append after a tamper laundered the
    // altered history into a valid chain and the violation vanished
    // on restart. Persists must refuse until a human investigates.
    const { file, store } = tmpStore();
    await store.record({ ...base });
    await store.record({ ...base, summary: "second" });
    const onDisk = JSON.parse(readFileSync(file, "utf8")) as { events: { summary: string }[] };
    onDisk.events[0]!.summary = "tampered";
    writeFileSync(file, JSON.stringify(onDisk, null, 2));
    const reloaded = new EventsStore(file);
    expect((await reloaded.health()).integrity.ok).toBe(false);
    await expect(reloaded.record({ ...base, summary: "routine append" })).rejects.toThrow("integrity violation");
    // The on-disk evidence is untouched and a fresh process still
    // sees the violation; the refused append waits in the outbox.
    expect(readFileSync(file, "utf8")).toContain("tampered");
    const fresh = new EventsStore(file);
    expect((await fresh.health()).integrity.ok).toBe(false);
    expect(existsSync(reloaded.outboxFile)).toBe(true);
  });

  test("a damaged record in a v2 file is quarantined, not migrated as v1", async () => {
    // Codex re-review on PR #5137 (P2): deleting a v2 record's hash
    // made it fail the v2 shape but pass the v1 shape, so it was
    // silently "migrated" with an empty hash and excluded from
    // verification. Migration is gated on the file version now.
    const dir = mkdtempSync(join(tmpdir(), "agg-events-damaged-v2-"));
    const file = join(dir, "events.json");
    const store = new EventsStore(file);
    await store.record({ ...base });
    const onDisk = JSON.parse(readFileSync(file, "utf8")) as { version: number; events: Record<string, unknown>[] };
    expect(onDisk.version).toBe(2);
    delete onDisk.events[0]!.hash;
    writeFileSync(file, JSON.stringify(onDisk, null, 2));
    const reloaded = new EventsStore(file);
    expect(await reloaded.list()).toEqual([]);
    expect(readdirSync(dir).some((name) => name.includes(".corrupt-"))).toBe(true);
  });

  test("a stored record backdated past retention is still integrity-checked (PR #5137 codex)", async () => {
    // Pruning ran before verification, so altering a record AND
    // backdating it past the 30-day cutoff removed it before its
    // hash was ever checked and health reported a healthy stream.
    const { file, store } = tmpStore();
    await store.record({ ...base });
    const onDisk = JSON.parse(readFileSync(file, "utf8")) as { events: { at: string }[] };
    onDisk.events[0]!.at = new Date(Date.now() - 40 * 24 * 60 * 60 * 1000).toISOString();
    writeFileSync(file, JSON.stringify(onDisk, null, 2));
    const reloaded = new EventsStore(file);
    expect((await reloaded.health()).integrity.ok).toBe(false);
  });

  test("a v2 record with an empty hash is quarantined as damaged (PR #5137 codex)", async () => {
    // An empty hash passed the shape check and was then excluded
    // from verification by the hash !== "" filters, so a tampered
    // summary with hash:"" reported healthy integrity.
    const dir = mkdtempSync(join(tmpdir(), "agg-events-empty-hash-"));
    const file = join(dir, "events.json");
    const store = new EventsStore(file);
    await store.record({ ...base });
    const onDisk = JSON.parse(readFileSync(file, "utf8")) as { version: number; events: Record<string, unknown>[] };
    expect(onDisk.version).toBe(2);
    onDisk.events[0]!.hash = "";
    onDisk.events[0]!.summary = "tampered";
    writeFileSync(file, JSON.stringify(onDisk, null, 2));
    const reloaded = new EventsStore(file);
    expect(await reloaded.list()).toEqual([]);
    expect(readdirSync(dir).some((name) => name.includes(".corrupt-"))).toBe(true);
  });

  test("a caller-supplied id replays idempotently instead of appending a twin (PR #5137 codex)", async () => {
    const { store } = tmpStore();
    const first = await store.record({ ...base, id: "evt-caller-1", summary: "first version" });
    const replay = await store.record({ ...base, id: "evt-caller-1", summary: "second version" });
    expect(replay.id).toBe(first.id);
    expect(replay.summary).toBe("first version");
    const list = await store.list();
    expect(list.filter((e) => e.id === "evt-caller-1")).toHaveLength(1);
    // The id resolves unambiguously for corrections.
    const correction = await store.recordCorrection("evt-caller-1", { actor: "keeper", summary: "correction" });
    expect(correction.corrects).toBe("evt-caller-1");
  });

  test("concurrent failed appends both reach the retry outbox; neither flush erases the other (PR #5137 codex)", async () => {
    // flushOutbox's unlink raced queueForRetry: a flush could delete
    // a record queued after its snapshot was persisted. The record
    // lifecycle is serialized now, so concurrent failures queue
    // independently and both land on recovery.
    const dir = mkdtempSync(join(tmpdir(), "agg-events-concurrent-retry-"));
    const file = join(dir, "events.json");
    const store = new EventsStore(file);
    await store.record({ ...base, summary: "landed" });
    rmSync(file);
    mkdirSync(file);
    writeFileSync(join(file, "occupant"), "x");
    const results = await Promise.allSettled([
      store.record({ ...base, summary: "concurrent failure A" }),
      store.record({ ...base, summary: "concurrent failure B" }),
    ]);
    expect(results.map((r) => r.status)).toEqual(["rejected", "rejected"]);
    expect(await store.health().then((h) => h.pendingRetries)).toBe(2);
    rmSync(file, { recursive: true });
    await store.record({ ...base, summary: "after recovery" });
    const summaries = (await new EventsStore(file).list()).map((e) => e.summary);
    expect(summaries).toContain("concurrent failure A");
    expect(summaries).toContain("concurrent failure B");
    expect(summaries).toContain("after recovery");
    expect(summaries).toContain("landed");
    expect(existsSync(store.outboxFile)).toBe(false);
  });
});
