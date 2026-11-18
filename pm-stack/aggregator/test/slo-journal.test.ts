import { describe, expect, test } from "bun:test";
import { appendFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { groupSamplesByFacet, SloJournal, type SloJournalLine } from "../src/slo-journal.ts";

const MINUTE = 60_000;

describe("SLO journal", () => {
  test("round-trips appended lines in order and never rewrites earlier ones", async () => {
    const dir = await mkdtemp(join(tmpdir(), "slo-journal-"));
    try {
      const file = join(dir, "nested", "slo-journal.jsonl");
      const journal = new SloJournal(file);
      const first: SloJournalLine[] = [
        { facet: "feed-age", atMs: MINUTE, value: 120_000, bad: false },
        { facet: "intake-up", atMs: MINUTE, value: null, bad: null },
      ];
      const second: SloJournalLine[] = [{ facet: "feed-age", atMs: 2 * MINUTE, value: 900_000, bad: true }];
      await journal.append(first);
      await journal.append(second);

      expect(await journal.readAll()).toEqual([...first, ...second]);
      const raw = await readFile(file, "utf8");
      expect(raw.trimEnd().split("\n")).toHaveLength(3);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("reads an absent journal as empty", async () => {
    const dir = await mkdtemp(join(tmpdir(), "slo-journal-"));
    try {
      expect(await new SloJournal(join(dir, "missing.jsonl")).readAll()).toEqual([]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("skips a torn line and keeps appending after a failed write", async () => {
    const dir = await mkdtemp(join(tmpdir(), "slo-journal-"));
    try {
      const file = join(dir, "slo-journal.jsonl");
      const good: SloJournalLine = { facet: "feed-age", atMs: MINUTE, value: 1, bad: false };
      await writeFile(file, `${JSON.stringify(good)}\n{"facet":"feed-a`, "utf8");
      await appendFile(file, "\n", "utf8");
      const journal = new SloJournal(file);
      expect(await journal.readAll()).toEqual([good]);

      // A journal whose parent is a file cannot be written: the append
      // fails, and a later append to a writable journal is unaffected.
      const blocker = join(dir, "blocker");
      await writeFile(blocker, "", "utf8");
      const broken = new SloJournal(join(blocker, "slo-journal.jsonl"));
      await expect(broken.append([good])).rejects.toThrow();

      const later: SloJournalLine = { facet: "feed-age", atMs: 2 * MINUTE, value: 2, bad: false };
      await mkdir(join(dir, "ok"), { recursive: true });
      const shared = new SloJournal(join(dir, "ok", "j.jsonl"));
      // Poison the chain directly, then append: the append still lands.
      (shared as unknown as { writes: Promise<void> }).writes = Promise.reject(new Error("boom"));
      await shared.append([later]);
      expect(await shared.readAll()).toEqual([later]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("groups samples by facet without the facet key", () => {
    const grouped = groupSamplesByFacet([
      { facet: "a", atMs: 1, value: null, bad: false },
      { facet: "b", atMs: 1, value: 5, bad: true },
      { facet: "a", atMs: 2, value: null, bad: true },
    ]);
    expect(grouped.get("a")).toEqual([
      { atMs: 1, value: null, bad: false },
      { atMs: 2, value: null, bad: true },
    ]);
    expect(grouped.get("b")).toEqual([{ atMs: 1, value: 5, bad: true }]);
  });
});
