// #14958: the segmented append-only layout for the audit-event
// store (follow-on of the #14865 write-amplification sweep). The
// v2 layout rewrote and re-chained the whole hot-stream file on
// every append; the v3 layout keeps the records in append-only
// segment files under <file>.segments/ with a small checkpoint
// manifest at <file>. Oracles, per test: segment bytes are
// append-only (a persist never changes a byte already written);
// per-append growth is bounded by the new records' lines plus the
// manifest, independent of stream length; a legacy v1/v2 file
// converts on first persist with its chain byte-identical; the
// manifest checkpoint makes a truncated tail an integrity
// violation (full-chain verification alone cannot see a missing
// tail); a torn trailing line is a never-completed append and is
// dropped, while damage anywhere else quarantines the store set;
// a lost manifest is rebuilt by adopting the segments.
import { describe, expect, test } from "bun:test";
import { appendFileSync, existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  EVENT_CHAIN_GENESIS,
  EVENT_RETENTION_MAX,
  EVENT_SEGMENT_MAX_RECORDS,
  EventsStore,
  eventHash,
  type AuditEvent,
  type EventsManifest,
} from "../src/events.ts";

const base = { actor: "drew", kind: "action-applied", ref: "gh:developing-today/code#1", summary: "did a thing" };
const iso = (msAgo: number) => new Date(Date.now() - msAgo).toISOString();

function tmpStore(): { dir: string; file: string; store: EventsStore } {
  const dir = mkdtempSync(join(tmpdir(), "agg-events-seg-"));
  const file = join(dir, "events.json");
  return { dir, file, store: new EventsStore(file) };
}

function segmentNames(store: EventsStore): string[] {
  return readdirSync(store.segmentsDir).sort();
}

function segmentBytes(store: EventsStore): Map<string, string> {
  const out = new Map<string, string>();
  for (const name of segmentNames(store)) out.set(name, readFileSync(join(store.segmentsDir, name), "utf8"));
  return out;
}

function readManifest(file: string): EventsManifest {
  return JSON.parse(readFileSync(file, "utf8")) as EventsManifest;
}

/** Stamp records into a valid stored chain, the way the archive
 *  suite's fixtures do, so a hand-written legacy file verifies. */
function stampChain(rows: Array<Partial<AuditEvent> & { at: string; summary: string }>): AuditEvent[] {
  let prev = EVENT_CHAIN_GENESIS;
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

describe("events segmented layout (#14958)", () => {
  test("segment bytes are append-only across appends and growth is bounded by the new records", async () => {
    const { file, store } = tmpStore();
    for (let i = 0; i < 60; i++) {
      await store.record({ ...base, id: `evt_seg_${i}`, summary: `event ${i}` });
    }
    const before = segmentBytes(store);
    const manifestBefore = readManifest(file);
    expect(manifestBefore.version).toBe(3);
    expect(manifestBefore.checkpoint.records).toBe(60);
    const sizeBefore = [...before.values()].reduce((n, s) => n + s.length, 0) + readFileSync(file, "utf8").length;

    await store.record({ ...base, id: "evt_seg_60", summary: "event 60" });

    // Oracle 1 (append-only): the same segment files exist and every
    // byte written before is still there, unchanged, as a prefix.
    const after = segmentBytes(store);
    expect([...after.keys()]).toEqual([...before.keys()]);
    for (const [name, bytes] of before) {
      expect(after.get(name)!.startsWith(bytes)).toBe(true);
    }
    // Oracle 2 (bounded growth): one append grew the whole store by
    // one record line plus at most a manifest's worth of bytes —
    // not by anything proportional to the 60 records already stored.
    const sizeAfter = [...after.values()].reduce((n, s) => n + s.length, 0) + readFileSync(file, "utf8").length;
    expect(sizeAfter - sizeBefore).toBeLessThan(2000);
    expect(sizeBefore).toBeGreaterThan(10 * (sizeAfter - sizeBefore));
    const reloaded = new EventsStore(file);
    expect((await reloaded.list()).length).toBe(61);
    expect((await reloaded.health()).integrity.ok).toBe(true);
    expect(readManifest(file).checkpoint.records).toBe(61);
  });

  test("segments roll at the segment bound and the manifest indexes both", async () => {
    const { file, store } = tmpStore();
    const total = EVENT_SEGMENT_MAX_RECORDS + 5;
    for (let i = 0; i < total; i++) {
      await store.record({ ...base, id: `evt_roll_${i}`, summary: `roll ${i}` });
    }
    expect(segmentNames(store)).toHaveLength(2);
    const manifest = readManifest(file);
    expect(manifest.segments.map((s) => s.records)).toEqual([EVENT_SEGMENT_MAX_RECORDS, 5]);
    expect(manifest.checkpoint.records).toBe(total);
    const reloaded = new EventsStore(file);
    expect((await reloaded.list()).length).toBe(total);
    expect((await reloaded.health()).integrity.ok).toBe(true);
  }, 30_000);

  test("a legacy v2 file loads unchanged and converts on first persist, chain byte-identical", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-events-convert-"));
    const file = join(dir, "events.json");
    const stamped = stampChain([
      { at: iso(3000), summary: "one" },
      { at: iso(2000), summary: "two" },
      { at: iso(1000), summary: "three" },
    ]);
    writeFileSync(file, JSON.stringify({ version: 2, events: stamped }, null, 2));
    const store = new EventsStore(file);
    expect((await store.list()).map((e) => e.summary)).toEqual(["three", "two", "one"]);
    // Still the legacy file until a persist happens.
    expect((JSON.parse(readFileSync(file, "utf8")) as { version: number }).version).toBe(2);

    await store.record({ ...base, summary: "four" });
    const manifest = readManifest(file);
    expect(manifest.version).toBe(3);
    expect(manifest.checkpoint.records).toBe(4);
    expect(existsSync(join(dir, "events.json.segments"))).toBe(true);
    const reloaded = new EventsStore(file);
    const list = await reloaded.list();
    expect(list.map((e) => e.summary)).toEqual(["four", "three", "two", "one"]);
    // The converted records kept their original hashes: conversion
    // is a layout change, not a re-stamping of history.
    const byId = new Map(list.map((e) => [e.id, e.hash]));
    for (const original of stamped) expect(byId.get(original.id)).toBe(original.hash);
    expect((await reloaded.health()).integrity.ok).toBe(true);
  });

  test("a legacy v1 file converts on first persist with its deterministic migration ids", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-events-convert-v1-"));
    const file = join(dir, "events.json");
    const rows = [
      { ...base, at: iso(2000), summary: "legacy one" },
      { ...base, at: iso(1000), summary: "legacy two" },
    ];
    writeFileSync(file, JSON.stringify({ version: 1, events: rows }, null, 2));
    const store = new EventsStore(file);
    const migratedIds = (await store.list()).map((e) => e.id).sort();
    await store.record({ ...base, summary: "after migration" });
    expect(readManifest(file).version).toBe(3);
    const reloaded = new EventsStore(file);
    const list = await reloaded.list();
    expect(list.length).toBe(3);
    expect(
      list
        .filter((e) => e.migratedFromVersion === 1)
        .map((e) => e.id)
        .sort(),
    ).toEqual(migratedIds);
    expect((await reloaded.health()).integrity.ok).toBe(true);
  });

  test("a tail truncated below the manifest checkpoint is an integrity violation, not a shorter history", async () => {
    const { file, store } = tmpStore();
    for (let i = 0; i < 4; i++) await store.record({ ...base, summary: `event ${i}` });
    expect(readManifest(file).checkpoint.records).toBe(4);
    // Delete the last record's line cleanly: the remaining chain
    // still verifies link-by-link, so only the checkpoint can see
    // the loss.
    const segName = segmentNames(store)[0]!;
    const segPath = join(store.segmentsDir, segName);
    const lines = readFileSync(segPath, "utf8")
      .split("\n")
      .filter((l) => l !== "");
    writeFileSync(segPath, `${lines.slice(0, 3).join("\n")}\n`);
    const reloaded = new EventsStore(file);
    const health = await reloaded.health();
    expect(health.integrity.ok).toBe(false);
    expect(health.integrity.detail).toContain("checkpoint");
    expect((await reloaded.list()).length).toBe(3); // readable, not hidden
    await expect(reloaded.record({ ...base, summary: "must not land" })).rejects.toThrow("integrity violation");
  });

  test("a torn trailing line is dropped as a never-completed append; the store keeps working", async () => {
    const { file, store } = tmpStore();
    for (let i = 0; i < 3; i++) await store.record({ ...base, summary: `event ${i}` });
    const segPath = join(store.segmentsDir, segmentNames(store)[0]!);
    appendFileSync(segPath, '{"id":"evt_torn","summ');
    const reloaded = new EventsStore(file);
    expect((await reloaded.list()).length).toBe(3);
    expect((await reloaded.health()).integrity.ok).toBe(true);
    await reloaded.record({ ...base, summary: "after torn" });
    const fresh = new EventsStore(file);
    expect((await fresh.list()).length).toBe(4);
    expect((await fresh.health()).integrity.ok).toBe(true);
  });

  test("damage in the middle of a segment quarantines the whole store set, bytes preserved", async () => {
    const { dir, file, store } = tmpStore();
    for (let i = 0; i < 3; i++) await store.record({ ...base, summary: `event ${i}` });
    const segName = segmentNames(store)[0]!;
    const segPath = join(store.segmentsDir, segName);
    const lines = readFileSync(segPath, "utf8")
      .split("\n")
      .filter((l) => l !== "");
    lines[1] = "this is not json";
    writeFileSync(segPath, `${lines.join("\n")}\n`);
    const reloaded = new EventsStore(file);
    expect(await reloaded.list()).toEqual([]);
    const names = readdirSync(dir);
    expect(names.some((n) => n.startsWith("events.json.corrupt-"))).toBe(true);
    expect(names.some((n) => n.startsWith("events.json.segments.corrupt-"))).toBe(true);
    const quarantinedSeg = names.find((n) => n.startsWith("events.json.segments.corrupt-"))!;
    expect(readFileSync(join(dir, quarantinedSeg, segName), "utf8")).toContain("this is not json");
  });

  test("a lost manifest is rebuilt by adopting the segments", async () => {
    const { file, store } = tmpStore();
    for (let i = 0; i < 3; i++) await store.record({ ...base, summary: `event ${i}` });
    rmSync(file);
    const reloaded = new EventsStore(file);
    expect((await reloaded.list()).length).toBe(3);
    expect((await reloaded.health()).integrity.ok).toBe(true);
    await reloaded.record({ ...base, summary: "event 3" });
    expect(readManifest(file).version).toBe(3);
    expect(readManifest(file).checkpoint.records).toBe(4);
    expect((await new EventsStore(file).list()).length).toBe(4);
  });

  test("prefix pruning at the retention cap appends without rewriting segment bytes", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-events-seg-cap-"));
    const file = join(dir, "events.json");
    const total = EVENT_RETENTION_MAX + 5;
    const rows = Array.from({ length: total }, (_, i) => ({
      at: iso((total - i) * 1000),
      summary: `e${i}`,
    }));
    writeFileSync(file, JSON.stringify({ version: 2, events: stampChain(rows) }, null, 2));
    const store = new EventsStore(file);
    // Load archives the 5 oldest and converts the retained 5000.
    expect((await store.health()).events).toBe(EVENT_RETENTION_MAX);
    expect((await store.health()).archivedRecords).toBe(5);
    const before = segmentBytes(store);
    for (let i = 0; i < 3; i++) {
      await store.record({ ...base, at: iso(0), summary: `fresh ${i}` });
    }
    const health = await store.health();
    expect(health.events).toBe(EVENT_RETENTION_MAX);
    expect(health.archivedRecords).toBe(8);
    expect(health.integrity.ok).toBe(true);
    // Oracle: pruning the oldest records never rewrote a segment —
    // every file that existed before is still there with every
    // earlier byte intact as a prefix (appends only extend the
    // tail segment, rolling to a fresh one when it fills).
    const after = segmentBytes(store);
    for (const [name, bytes] of before) {
      expect(after.get(name)).toBeDefined();
      expect(after.get(name)!.startsWith(bytes)).toBe(true);
    }
    const reloaded = new EventsStore(file);
    expect((await reloaded.health()).integrity.ok).toBe(true);
    expect((await reloaded.health()).archivedRecords).toBe(8);
  }, 30_000);
});
