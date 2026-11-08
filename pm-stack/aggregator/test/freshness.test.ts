import { describe, expect, test } from "bun:test";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { FreshnessStore, type FreshnessInput, type FreshnessNotice } from "../src/freshness.ts";

function input(eventId: string, scope = "issue:123"): FreshnessInput {
  return {
    eventId,
    source: "github",
    scope: [scope],
    occurredAt: "2026-11-04T12:00:00.000Z",
    reason: "source-event",
  };
}

function storedNotice(sequence: number, eventId: string, observedAt: string): FreshnessNotice {
  return {
    ...input(eventId),
    schemaVersion: 1,
    sequence,
    observedAt,
  };
}

async function withTempDir(run: (dir: string) => Promise<void>): Promise<void> {
  const dir = await mkdtemp(join(tmpdir(), "freshness-store-"));
  try {
    await run(dir);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

describe("FreshnessStore", () => {
  test("rejects extra payload, secret, and body fields without writing them", async () => {
    await withTempDir(async (dir) => {
      const file = join(dir, "freshness.json");
      const store = new FreshnessStore(file);
      const base = input("strict-1");
      const invalid = [
        { ...base, payload: { token: "must-not-persist" } },
        { ...base, secret: "must-not-persist" },
        { ...base, body: "must-not-persist" },
        { ...base, extra: true },
        { ...base, scope: [] },
        { ...base, occurredAt: "not-a-date" },
        { ...base, eventId: "x".repeat(513) },
      ];

      for (const candidate of invalid) {
        await expect(store.publish(candidate as FreshnessInput)).rejects.toThrow(/invalid freshness/);
      }
      await expect(readFile(file, "utf8")).rejects.toMatchObject({ code: "ENOENT" });
      await expect(readFile(`${file}.outbox.jsonl`, "utf8")).rejects.toMatchObject({ code: "ENOENT" });
    });
  });

  test("assigns increasing sequences, deduplicates stable event ids, and serializes concurrent publishes", async () => {
    await withTempDir(async (dir) => {
      const store = new FreshnessStore(join(dir, "freshness.json"));
      const first = await store.publish(input("event-1"));
      const duplicate = await store.publish(input("event-1", "different-scope"));
      expect(first).toMatchObject({ duplicate: false, pending: false, notice: { sequence: 1 } });
      expect(duplicate).toMatchObject({ duplicate: true, pending: false, notice: { sequence: 1 } });

      const concurrent = await Promise.all(Array.from({ length: 20 }, (_, index) => store.publish(input(`parallel-${index}`))));
      expect(concurrent.map((result) => result.notice.sequence)).toEqual(Array.from({ length: 20 }, (_, index) => index + 2));
      expect((await store.listAfter(0)).cursor).toBe(21);
    });
  });

  test("returns reset when retention evicts a cursor range, and bounds notices to 10,000", async () => {
    await withTempDir(async (dir) => {
      const file = join(dir, "freshness.json");
      const now = new Date().toISOString();
      const events = Array.from({ length: 10_001 }, (_, index) => storedNotice(index + 1, `bound-${index + 1}`, now));
      await writeFile(file, JSON.stringify({ version: 1, cursor: 10_001, events }));

      const page = await new FreshnessStore(file).listAfter(0);
      expect(page.cursor).toBe(10_001);
      expect(page.resetRequired).toBe(true);
      expect(page.events).toHaveLength(10_000);
      expect(page.events[0]?.sequence).toBe(2);
      expect(page.events.at(-1)?.sequence).toBe(10_001);
      expect((await new FreshnessStore(file).listAfter(10_001)).resetRequired).toBe(false);
    });
  });

  test("expires notices older than 24 hours and filters by scope", async () => {
    await withTempDir(async (dir) => {
      const file = join(dir, "freshness.json");
      const now = Date.now();
      const old = new Date(now - 24 * 60 * 60 * 1000 - 1).toISOString();
      const recent = new Date(now - 60_000).toISOString();
      const events = [
        storedNotice(1, "expired", old),
        { ...storedNotice(2, "kept", recent), scope: ["project:alpha"] },
      ];
      await writeFile(file, JSON.stringify({ version: 1, cursor: 2, events }));

      const store = new FreshnessStore(file);
      const scoped = await store.listAfter(0, "project:alpha");
      expect(scoped).toMatchObject({ cursor: 2, resetRequired: true });
      expect(scoped.events.map((event) => event.eventId)).toEqual(["kept"]);
      expect((await store.listAfter(0, "project:missing")).events).toEqual([]);
    });
  });

  test("recovers the journal and cursor after restart", async () => {
    await withTempDir(async (dir) => {
      const file = join(dir, "freshness.json");
      const written = await new FreshnessStore(file).publish(input("restart-1"));
      const page = await new FreshnessStore(file).listAfter(0);
      expect(page).toMatchObject({ cursor: 1, resetRequired: false, events: [written.notice] });
    });
  });

  test("queues a failed journal write durably, then flushes the outbox on recovery", async () => {
    await withTempDir(async (dir) => {
      const file = join(dir, "freshness.json");
      const store = new FreshnessStore(file);
      await store.listAfter(0); // Load the empty store before making its target unwritable.
      await mkdir(file); // The atomic rename now fails, while the sibling outbox remains writable.

      const queued = await store.publish(input("outbox-1"));
      expect(queued).toMatchObject({ duplicate: false, pending: true, notice: { sequence: 0 } });
      const pendingDuplicate = await store.publish(input("outbox-1"));
      expect(pendingDuplicate).toMatchObject({ duplicate: true, pending: true, notice: { sequence: 0 } });
      expect(await readFile(`${file}.outbox.jsonl`, "utf8")).toContain('"eventId":"outbox-1"');

      await rm(file, { recursive: true });
      const recovered = await new FreshnessStore(file).listAfter(0);
      expect(recovered).toMatchObject({ cursor: 1, resetRequired: false, events: [{ eventId: "outbox-1", sequence: 1 }] });
      await expect(readFile(`${file}.outbox.jsonl`, "utf8")).rejects.toMatchObject({ code: "ENOENT" });
    });
  });

  test("persists an old outbox event before removing it during late recovery", async () => {
    await withTempDir(async (dir) => {
      const file = join(dir, "freshness.json");
      const observedAt = new Date(Date.now() - 25 * 60 * 60 * 1000).toISOString();
      const entry = { input: input("late-recovery"), observedAt };
      await writeFile(`${file}.outbox.jsonl`, `${JSON.stringify(entry)}\n`);

      await new FreshnessStore(file).listAfter(0);

      const journal = JSON.parse(await readFile(file, "utf8")) as {
        cursor: number;
        events: Array<{ eventId: string; sequence: number }>;
      };
      expect(journal).toMatchObject({ cursor: 1, events: [{ eventId: "late-recovery", sequence: 1 }] });
      await expect(readFile(`${file}.outbox.jsonl`, "utf8")).rejects.toMatchObject({ code: "ENOENT" });
    });
  });

  test("fails closed when the durable outbox exceeds its entry bound", async () => {
    await withTempDir(async (dir) => {
      const file = join(dir, "freshness.json");
      const observedAt = new Date().toISOString();
      const lines = Array.from({ length: 10_001 }, (_, index) =>
        JSON.stringify({ input: input(`over-cap-${index}`), observedAt }),
      );
      await writeFile(`${file}.outbox.jsonl`, `${lines.join("\n")}\n`);

      await expect(new FreshnessStore(file).listAfter(0)).rejects.toThrow("outbox exceeds its configured limit");
      expect((await readFile(`${file}.outbox.jsonl`, "utf8")).split("\n")).toHaveLength(10_002);
      await expect(readFile(file, "utf8")).rejects.toMatchObject({ code: "ENOENT" });
    });
  });

  test("fails closed without changing a corrupt journal", async () => {
    await withTempDir(async (dir) => {
      const file = join(dir, "freshness.json");
      const corrupt = '{"version":1,"cursor":0,"events":[],"secret":"leave-me-alone"}';
      await writeFile(file, corrupt);
      const store = new FreshnessStore(file);

      await expect(store.publish(input("must-not-write"))).rejects.toThrow("invalid freshness journal");
      expect(await readFile(file, "utf8")).toBe(corrupt);
    });
  });
});
