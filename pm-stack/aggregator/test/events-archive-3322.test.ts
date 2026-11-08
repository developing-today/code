// #3322 retention contract: audit records that leave the hot stream
// are archived before they are pruned, the archive verifies
// independently, a backup generation (events.json + archive
// partitions + retry outbox + activity-ingestion.json, each in the
// manifest) restores the retained plus archived history, the pending
// retry state, and the ingestion checkpoint/seen-id recovery — and
// replaying restored deliveries does not duplicate them. History
// outside the retained+archived set is reported as a visible gap.
import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { IngestionStore, type ActivityDelivery } from "../src/activity.ts";
import {
  EVENT_CHAIN_GENESIS,
  EVENT_RETENTION_MAX,
  EventsStore,
  eventHash,
  partitionRetention,
  verifyArchivedRecords,
  type AuditEvent,
  type StreamEvent,
} from "../src/events.ts";

const base = { actor: "drew", kind: "action-applied", ref: "gh:developing-today/code#1", summary: "did a thing" };
const DAY_MS = 24 * 60 * 60 * 1000;
const iso = (msAgo: number) => new Date(Date.now() - msAgo).toISOString();

function tmpDir(prefix: string): string {
  return mkdtempSync(join(tmpdir(), prefix));
}

/** Stamp records into a valid stored chain, the way persist()'s
 *  rechain does, so a fixture file can carry records (including
 *  past-retention ones) that load() will verify. */
function stampChain(rows: Array<Partial<AuditEvent> & { at: string; summary: string }>, anchor = EVENT_CHAIN_GENESIS): AuditEvent[] {
  let prev = anchor;
  return rows.map((row, i) => {
    const stamped: AuditEvent = {
      actor: "drew",
      kind: "action-applied",
      ref: "gh:developing-today/code#1",
      version: 2,
      id: `evt_fixture_${i}`,
      ingestedAt: row.at,
      attribution: { confidence: "unknown", asserted: false },
      source: { identity: "test-fixture" },
      ...row,
      prevHash: prev,
      hash: "",
    } as AuditEvent;
    stamped.hash = eventHash(prev, stamped);
    prev = stamped.hash;
    return stamped;
  });
}

async function writeEventsFile(file: string, events: AuditEvent[]): Promise<void> {
  await mkdir(dirname(file), { recursive: true });
  await writeFile(file, JSON.stringify({ version: 2, events }, null, 2));
}

describe("audit archive retention (#3322)", () => {
  test("partitionRetention splits both bounds: pruned are the oldest, in stored order", () => {
    // Count bound (all rows inside the time bound).
    const rows: StreamEvent[] = Array.from({ length: EVENT_RETENTION_MAX + 7 }, (_, i) => ({
      ...base,
      at: iso((EVENT_RETENTION_MAX + 7 - i) * 1000),
    }));
    const byCount = partitionRetention(rows);
    expect(byCount.retained).toHaveLength(EVENT_RETENTION_MAX);
    expect(byCount.pruned.map((e) => e.at)).toEqual(rows.slice(0, 7).map((e) => e.at));
    // Time bound.
    const mixed: StreamEvent[] = [
      { ...base, at: iso(40 * DAY_MS) },
      { ...base, at: iso(31 * DAY_MS) },
      { ...base, at: iso(60 * 60 * 1000) },
    ];
    const byAge = partitionRetention(mixed);
    expect(byAge.pruned.map((e) => e.at)).toEqual([mixed[0]!.at, mixed[1]!.at]);
    expect(byAge.retained).toHaveLength(1);
  });

  test("load archives pruned records before dropping them, byte-identical, and the archive verifies after restart", async () => {
    const dir = tmpDir("agg-events-archive-load-");
    const file = join(dir, "events.json");
    const stamped = stampChain([
      { at: iso(40 * DAY_MS), summary: "old one" },
      { at: iso(35 * DAY_MS), summary: "old two" },
      { at: iso(60 * 60 * 1000), summary: "recent" },
    ]);
    await writeEventsFile(file, stamped);

    const store = new EventsStore(file);
    expect((await store.list()).map((e) => e.summary)).toEqual(["recent"]);
    // The pruned records are preserved exactly as they were stored
    // (hashes included) — the archive is evidence, not a rewrite.
    const archived = await store.listArchived();
    expect(archived).toEqual(stamped.slice(0, 2));
    const verdict = verifyArchivedRecords(archived);
    expect(verdict.ok).toBe(true);
    // The archived slice is the original chain's prefix: its links
    // verify contiguously from genesis.
    expect(verdict.linksVerified).toBe(1);
    expect(archived[0]!.prevHash).toBe(EVENT_CHAIN_GENESIS);

    // Restart: same picture, no duplicate archiving, health agrees.
    const reloaded = new EventsStore(file);
    const health = await reloaded.health();
    expect(health.archivedRecords).toBe(2);
    expect(health.archive.ok).toBe(true);
    expect(health.historyGap.present).toBe(false);
    expect(health.integrity.ok).toBe(true);
    expect(await reloaded.listArchived()).toEqual(stamped.slice(0, 2));
    // The archive lives where the backup set looks for it.
    expect(existsSync(join(dir, "events.json.archive"))).toBe(true);
    const partitions = readdirSync(join(dir, "events.json.archive"));
    expect(partitions.some((n) => n.startsWith("events-archive-") && n.endsWith(".json"))).toBe(true);
  });

  test("an append past the retention bound is archived, not dropped", async () => {
    const dir = tmpDir("agg-events-archive-append-");
    const store = new EventsStore(join(dir, "events.json"));
    await store.record({ ...base, at: iso(40 * DAY_MS), summary: "backdated" });
    expect(await store.list()).toHaveLength(0);
    const archived = await store.listArchived();
    expect(archived.map((e) => e.summary)).toEqual(["backdated"]);
    expect(verifyArchivedRecords(archived).ok).toBe(true);
    await store.record({ ...base, at: iso(60 * 1000), summary: "fresh" });
    expect((await store.list()).map((e) => e.summary)).toEqual(["fresh"]);
    expect((await new EventsStore(join(dir, "events.json")).health()).archivedRecords).toBe(1);
  });

  test("a backfilled append inside the retained range keeps the chain valid and prunes the oldest", async () => {
    // #3322 Codex review, PR #7665 (P1): retention must keep the
    // STORED-chain order of the retained records. A backfilled event
    // whose `at` falls inside the existing range is stamped at the
    // chain tail; re-sorting it into the middle by timestamp broke
    // the links and set the sticky integrityViolation, which then
    // refused every later append.
    const dir = tmpDir("agg-events-archive-backfill-");
    const file = join(dir, "events.json");
    // A full hot stream (the count bound) written as one fixture file.
    const rows = Array.from({ length: EVENT_RETENTION_MAX }, (_, i) => ({
      at: iso((EVENT_RETENTION_MAX - i) * 1000),
      summary: `e${i}`,
    }));
    await writeEventsFile(file, stampChain(rows));
    const store = new EventsStore(file);
    expect((await store.health()).events).toBe(EVENT_RETENTION_MAX);
    // Backfilled INSIDE the retained range: stamped at the chain tail,
    // but older than the newest records by `at`.
    await store.record({ ...base, at: iso(Math.floor(EVENT_RETENTION_MAX / 2) * 1000), summary: "backfilled-inside" });
    const health = await store.health();
    expect(health.integrity.ok).toBe(true);
    expect(health.events).toBe(EVENT_RETENTION_MAX);
    // The oldest record left the hot stream and is archived, not lost.
    expect(health.archivedRecords).toBe(1);
    expect((await store.listArchived()).map((e) => e.summary)).toEqual(["e0"]);
    // The appended record is still in the stream and later appends work.
    expect((await store.list()).map((e) => e.summary)).toContain("backfilled-inside");
    await store.record({ ...base, at: iso(0), summary: "after" });
    expect((await store.health()).integrity.ok).toBe(true);
  }, 30_000);

  test("pruning a backfilled record out of the middle archives it and rechains the retained stream", async () => {
    // A time bound can remove a record from the middle of stored
    // order. The old chain verified first; the removed record is
    // archived with its original hash; the retained stream is
    // rechained so its next persist verifies (#3322 Codex review,
    // PR #7665).
    const dir = tmpDir("agg-events-archive-middle-");
    const file = join(dir, "events.json");
    const stamped = stampChain([
      { at: iso(60 * 1000), summary: "head-recent" },
      { at: iso(40 * DAY_MS), summary: "middle-old" },
      { at: iso(30 * 1000), summary: "tail-recent" },
    ]);
    await writeEventsFile(file, stamped);
    const store = new EventsStore(file);
    const health = await store.health();
    expect(health.integrity.ok).toBe(true);
    expect(health.events).toBe(2);
    expect(health.archivedRecords).toBe(1);
    const archived = await store.listArchived();
    expect(archived).toEqual([stamped[1]!]);
    expect(verifyArchivedRecords(archived).ok).toBe(true);
    await store.record({ ...base, at: iso(0), summary: "post" });
    expect((await store.health()).integrity.ok).toBe(true);
    expect((await store.list()).map((e) => e.summary)).toEqual(["post", "tail-recent", "head-recent"]);
  });

  test("a tampered archive partition fails the archive health verdict", async () => {
    const dir = tmpDir("agg-events-archive-tamper-");
    const file = join(dir, "events.json");
    const stamped = stampChain([
      { at: iso(40 * DAY_MS), summary: "old one" },
      { at: iso(60 * 60 * 1000), summary: "recent" },
    ]);
    await writeEventsFile(file, stamped);
    const store = new EventsStore(file);
    await store.list();
    const partition = readdirSync(join(dir, "events.json.archive"))[0]!;
    const path = join(dir, "events.json.archive", partition);
    const parsed = JSON.parse(readFileSync(path, "utf8")) as { records: AuditEvent[] };
    parsed.records[0]!.summary = "rewritten history";
    writeFileSync(path, JSON.stringify(parsed, null, 2));
    const health = await new EventsStore(file).health();
    expect(health.archive.ok).toBe(false);
    expect(health.archivedRecords).toBe(1);
  });

  test("history pruned before the archive contract is a visible gap, not a silent hole", async () => {
    const dir = tmpDir("agg-events-archive-gap-");
    const file = join(dir, "events.json");
    // A chain whose first retained record anchors to a predecessor
    // that was pruned with no archive (the pre-contract state).
    const stamped = stampChain([{ at: iso(60 * 60 * 1000), summary: "retained" }], "f".repeat(64));
    await writeEventsFile(file, stamped);
    const health = await new EventsStore(file).health();
    expect(health.historyGap.present).toBe(true);
    expect(health.historyGap.detail).toContain("retention anchor");
    expect(health.archivedRecords).toBe(0);
  });
});

describe("backup / restore round trip (#3322)", () => {
  const deliveries: ActivityDelivery[] = [
    { source: "watchdog", sourceId: "wd-1", at: iso(2 * 60 * 60 * 1000), kind: "watchdog-event", ref: "gh:developing-today/code#1", summary: "watchdog one" },
    { source: "watchdog", sourceId: "wd-2", at: iso(60 * 60 * 1000), kind: "watchdog-event", ref: "gh:developing-today/code#1", summary: "watchdog two" },
  ];

  function walkFiles(root: string): string[] {
    const out: string[] = [];
    const walk = (d: string) => {
      for (const name of readdirSync(d, { withFileTypes: true })) {
        const p = join(d, name.name);
        if (name.isDirectory()) walk(p);
        else out.push(relative(root, p));
      }
    };
    walk(root);
    return out.sort();
  }

  async function copyTree(from: string, to: string): Promise<void> {
    for (const rel of walkFiles(from)) {
      await mkdir(dirname(join(to, rel)), { recursive: true });
      await writeFile(join(to, rel), await readFile(join(from, rel)));
    }
  }

  test("a manifest-verified generation restores events, archive, pending retries, and ingestion checkpoints; replay does not duplicate", async () => {
    // --- source state -------------------------------------------------
    const src = tmpDir("agg-backup-src-");
    const srcEventsFile = join(src, "events.json");
    const srcEvents = new EventsStore(srcEventsFile);
    await srcEvents.record({ ...base, at: iso(40 * DAY_MS), summary: "archived history" });
    await srcEvents.record({ ...base, at: iso(2 * 60 * 60 * 1000), summary: "retained one" });
    const srcIngestion = new IngestionStore(join(src, "activity-ingestion.json"));
    const firstPass = await srcIngestion.ingest(srcEvents, deliveries);
    expect(firstPass).toMatchObject({ ingested: 2, duplicates: 0, failed: 0 });
    // A failed append waiting in the durable retry outbox.
    const pending: AuditEvent = {
      ...base,
      at: iso(30 * 60 * 1000),
      summary: "pending retry",
      version: 2,
      id: "evt_pending_restore",
      ingestedAt: iso(30 * 60 * 1000),
      attribution: { confidence: "unknown", asserted: false },
      source: { identity: "test-fixture" },
      prevHash: EVENT_CHAIN_GENESIS,
      hash: "",
    };
    await writeFile(srcEvents.outboxFile, `${JSON.stringify(pending)}\n`);

    // --- backup generation: copy the set + manifest -------------------
    const generation = tmpDir("agg-backup-gen-");
    await copyTree(src, generation);
    const manifest: Record<string, { sha256: string; size: number }> = {};
    for (const rel of walkFiles(generation)) {
      const bytes = readFileSync(join(generation, rel));
      manifest[rel] = { sha256: createHash("sha256").update(bytes).digest("hex"), size: bytes.length };
    }
    // Every member the #3322 acceptance names is in the manifest.
    for (const member of [
      "events.json",
      "events.json.outbox.jsonl",
      "activity-ingestion.json",
    ]) {
      expect(manifest[member]).toBeDefined();
    }
    expect(Object.keys(manifest).some((k) => k.startsWith("events.json.archive/"))).toBe(true);
    // Verify the generation against the manifest before restoring.
    for (const [rel, entry] of Object.entries(manifest)) {
      const bytes = readFileSync(join(generation, rel));
      expect(createHash("sha256").update(bytes).digest("hex")).toBe(entry.sha256);
      expect(bytes.length).toBe(entry.size);
    }

    // --- restore into an isolated state dir ---------------------------
    const dst = tmpDir("agg-backup-dst-");
    await copyTree(generation, dst);
    const dstEvents = new EventsStore(join(dst, "events.json"));
    const restored = await dstEvents.list();
    const summaries = restored.map((e) => e.summary);
    expect(summaries).toContain("retained one");
    expect(summaries).toContain("pending retry"); // outbox flushed on load
    expect(summaries).toContain("watchdog one");
    const health = await dstEvents.health();
    expect(health.pendingRetries).toBe(0);
    // The store's own verdict verifies the restored chain in stored
    // order (list() is a newest-first read surface, not chain order).
    expect(health.integrity.ok).toBe(true);
    // Retained plus archived history: the archived record is back,
    // intact, and not duplicated into the hot stream.
    const archived = await dstEvents.listArchived();
    expect(archived.map((e) => e.summary)).toEqual(["archived history"]);
    expect(verifyArchivedRecords(archived).ok).toBe(true);
    expect(health.archivedRecords).toBe(1);
    expect(summaries).not.toContain("archived history");

    // Ingestion checkpoint/seen-ID recovery: replaying the same
    // deliveries is all duplicates and emits nothing new.
    const dstIngestion = new IngestionStore(join(dst, "activity-ingestion.json"));
    expect(await dstIngestion.seenIds("watchdog")).toEqual(["wd-1", "wd-2"]);
    expect(await dstIngestion.checkpoint("watchdog")).toMatchObject({ deliveries: 2 });
    const before = (await dstEvents.list()).length;
    const replay = await dstIngestion.ingest(dstEvents, deliveries);
    expect(replay).toMatchObject({ ingested: 0, duplicates: 2, failed: 0 });
    expect((await dstEvents.list()).length).toBe(before);
  });
});
