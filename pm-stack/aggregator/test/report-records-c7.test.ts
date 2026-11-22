import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test } from "bun:test";
import type { AppendOnlyDocumentRemote } from "../src/aa-last-good.ts";
import {
  appendReportRecord,
  appendReportSnapshot,
  appendReportSnapshotCorrection,
  appendReportBlockConflictResolution,
  assertReportUpdateAllowed,
  defaultReportRecordsFile,
  makeReportRecord,
  readCurrentReportRecords,
  readReportRecords,
  readReportSnapshots,
  readReportSnapshotCorrections,
  reportBlockConflict,
  compareReportBlock,
  ReportRecordConflictError,
} from "../src/report-records.ts";

const tempDirs: string[] = [];

async function storePath(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), "report-records-c7-"));
  tempDirs.push(dir);
  return join(dir, "reports.jsonl");
}

afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true })));
});

describe("C7 canonical report records", () => {
  test("persists provenance and cursor and makes a rerun a no-op", async () => {
    const file = await storePath();
    const input = {
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily" as const,
        timezone: "America/Chicago" as const,
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "source-event-42",
      sources: [
        { id: "source-event-42", uri: "gh:developing-today/code/issues/12349", observedAt: "2026-11-22T05:40:00.000Z" },
      ],
      blocks: [
        { id: "change-12349", text: "Added the period boundary foundation.", relatedAt: "2026-11-22T05:40:00.000Z" },
      ],
    };

    const record = makeReportRecord(input, "2026-11-22T05:45:01.000Z");
    const first = await appendReportRecord(file, record);
    const rerun = await appendReportRecord(file, makeReportRecord(input, "2026-11-22T05:46:01.000Z"));
    const persisted = await readReportRecords(file);

    expect(first.inserted).toBe(true);
    expect(rerun.inserted).toBe(false);
    expect(rerun.record).toEqual(record);
    expect(persisted).toEqual([record]);
    expect(persisted[0]?.sourceCursor).toBe("source-event-42");
    expect(persisted[0]?.sources[0]?.uri).toBe("gh:developing-today/code/issues/12349");
    expect((await readFile(file, "utf8")).trim().split("\n")).toHaveLength(1);
  });

  test("rejects a forged record ID at the persistence boundary", async () => {
    const file = await storePath();
    const canonical = makeReportRecord(
      {
        period: {
          id: "daily:2026-11-21:America/Chicago",
          kind: "daily",
          timezone: "America/Chicago",
          periodStart: "2026-11-21T06:00:00.000Z",
          periodEnd: "2026-11-22T06:00:00.000Z",
          softCloseAt: "2026-11-22T14:00:00.000Z",
        },
        asOf: "2026-11-22T05:45:00.000Z",
        templateVersion: "daily-v1",
        sourceCursor: "source-event-42",
        sources: [{ id: "source-event-42", uri: "gh:issue/12349", observedAt: "2026-11-22T05:40:00.000Z" }],
        blocks: [{ id: "change-12349", text: "Canonical prose.", relatedAt: "2026-11-22T05:40:00.000Z" }],
      },
      "2026-11-22T05:45:01.000Z",
    );

    await expect(appendReportRecord(file, { ...canonical, id: "report:forged" })).rejects.toThrow(
      /canonical contents and ID/,
    );
    await expect(
      appendReportSnapshot(file, { ...canonical, id: "report:forged" }, "2026-11-22T14:00:00.000Z"),
    ).rejects.toThrow(/canonical contents and ID/);
    expect(await readReportRecords(file)).toEqual([]);
    expect(await readReportSnapshots(file)).toEqual([]);
  });

  test("rejects changed content for the same cursor and keeps rollover snapshots immutable", async () => {
    const file = await storePath();
    const input = {
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily" as const,
        timezone: "America/Chicago" as const,
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "source-event-42",
      sources: [
        { id: "source-event-42", uri: "gh:developing-today/code/issues/12349", observedAt: "2026-11-22T05:40:00.000Z" },
      ],
      blocks: [{ id: "change-12349", text: "Original prose.", relatedAt: "2026-11-22T05:40:00.000Z" }],
    };
    const record = makeReportRecord(input, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(file, record);

    const conflictingRecord = makeReportRecord(
      {
        ...input,
        blocks: [{ ...input.blocks[0]!, text: "Silently changed prose." }],
      },
      "2026-11-22T05:46:01.000Z",
    );
    await expect(appendReportRecord(file, conflictingRecord)).rejects.toBeInstanceOf(ReportRecordConflictError);
    await expect(appendReportSnapshot(file, conflictingRecord, "2026-11-22T14:00:00.000Z")).rejects.toBeInstanceOf(
      ReportRecordConflictError,
    );
    expect((await readReportRecords(file))[0]?.blocks[0]?.text).toBe("Original prose.");

    const snapshot = await appendReportSnapshot(file, record, "2026-11-22T14:00:00.000Z");
    expect(snapshot.inserted).toBe(true);
    const snapshotRerun = await appendReportSnapshot(
      file,
      makeReportRecord(input, "2026-11-22T05:46:01.000Z"),
      "2026-11-22T14:01:00.000Z",
    );
    expect(snapshotRerun.inserted).toBe(false);
    expect(snapshotRerun.snapshot.snapshotAt).toBe("2026-11-22T14:00:00.000Z");
    const mutatedSnapshotSource = makeReportRecord(
      {
        ...input,
        blocks: [{ ...input.blocks[0]!, text: "Changed after rollover." }],
      },
      "2026-11-22T05:47:01.000Z",
    );
    await expect(appendReportSnapshot(file, mutatedSnapshotSource, "2026-11-22T14:00:00.000Z")).rejects.toBeInstanceOf(
      ReportRecordConflictError,
    );
    expect((await readReportSnapshots(file))[0]?.record.blocks[0]?.text).toBe("Original prose.");
    const conflictingGeneration = makeReportRecord(
      {
        ...input,
        blocks: [{ ...input.blocks[0]!, text: "Conflicting after standalone snapshot." }],
      },
      "2026-11-22T05:48:01.000Z",
    );
    await expect(appendReportRecord(file, conflictingGeneration)).rejects.toBeInstanceOf(ReportRecordConflictError);
  });

  test("records snapshot corrections as new linked immutable records", async () => {
    const file = await storePath();
    const input = {
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily" as const,
        timezone: "America/Chicago" as const,
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "cursor-original",
      sources: [{ id: "event-1", uri: "gh:issue/1", observedAt: "2026-11-22T05:40:00.000Z" }],
      blocks: [{ id: "change-1", text: "Original snapshot.", relatedAt: "2026-11-22T05:40:00.000Z" }],
    };
    const original = makeReportRecord(input, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(file, original);
    await appendReportSnapshot(file, original, "2026-11-22T14:00:00.000Z");
    const correction = makeReportRecord(
      {
        ...input,
        sourceCursor: "cursor-correction-1",
        blocks: [{ ...input.blocks[0]!, text: "Corrected snapshot." }],
      },
      "2026-11-22T15:00:00.000Z",
    );

    const first = await appendReportSnapshotCorrection(
      file,
      "snapshot:daily:2026-11-21:America/Chicago",
      correction,
      "2026-11-22T15:00:00.000Z",
    );
    const rerun = await appendReportSnapshotCorrection(
      file,
      "snapshot:daily:2026-11-21:America/Chicago",
      correction,
      "2026-11-22T15:05:00.000Z",
    );
    expect(first.inserted).toBe(true);
    expect(rerun.inserted).toBe(false);
    expect(first.correction.snapshotId).toBe("snapshot:daily:2026-11-21:America/Chicago");
    expect(first.correction.originalRecordId).toBe(original.id);
    expect(first.correction.record.id).toBe(correction.id);
    expect((await readReportSnapshots(file))[0]?.record.blocks[0]?.text).toBe("Original snapshot.");
    expect((await readReportSnapshotCorrections(file)).map((item) => item.record.id)).toEqual([correction.id]);
    const conflictingOriginal = makeReportRecord(
      {
        ...input,
        blocks: [{ ...input.blocks[0]!, text: "Contradictory original record." }],
      },
      "2026-11-22T15:06:00.000Z",
    );
    await expect(
      appendReportSnapshotCorrection(
        file,
        "snapshot:daily:2026-11-21:America/Chicago",
        conflictingOriginal,
        "2026-11-22T15:06:01.000Z",
      ),
    ).rejects.toBeInstanceOf(ReportRecordConflictError);
    await expect(
      appendReportSnapshotCorrection(file, "snapshot:missing", correction, "2026-11-22T15:00:00.000Z"),
    ).rejects.toThrow(/snapshot does not exist/);
  });

  test("keeps later appends readable after a torn trailing JSONL fragment", async () => {
    const file = await storePath();
    const record = makeReportRecord(
      {
        period: {
          id: "daily:2026-11-21:America/Chicago",
          kind: "daily",
          timezone: "America/Chicago",
          periodStart: "2026-11-21T06:00:00.000Z",
          periodEnd: "2026-11-22T06:00:00.000Z",
          softCloseAt: "2026-11-22T14:00:00.000Z",
        },
        asOf: "2026-11-22T05:45:00.000Z",
        templateVersion: "daily-v1",
        sourceCursor: "source-event-42",
        sources: [{ id: "source-event-42", uri: "gh:issue/12349", observedAt: "2026-11-22T05:40:00.000Z" }],
        blocks: [{ id: "change-12349", text: "Recovered append.", relatedAt: "2026-11-22T05:40:00.000Z" }],
      },
      "2026-11-22T05:45:01.000Z",
    );

    await Bun.write(file, '{"eventId":"interrupted"');
    expect((await appendReportRecord(file, record)).inserted).toBe(true);
    expect(await readReportRecords(file)).toEqual([record]);

    await Bun.write(file, `${await readFile(file, "utf8")}{"eventId":"interrupted-snapshot"`);
    expect((await appendReportSnapshot(file, record, "2026-11-22T14:00:00.000Z")).inserted).toBe(true);
    expect(await readReportSnapshots(file)).toHaveLength(1);
    expect(await readReportRecords(file)).toEqual([record]);
  });

  test("allows ordinary edits only before soft close and the documented append forms after it", () => {
    const period = {
      id: "daily:2026-11-21:America/Chicago",
      kind: "daily" as const,
      timezone: "America/Chicago" as const,
      periodStart: "2026-11-21T06:00:00.000Z",
      periodEnd: "2026-11-22T06:00:00.000Z",
      softCloseAt: "2026-11-22T14:00:00.000Z",
    };
    expect(() => assertReportUpdateAllowed(period, "replace", "2026-11-22T13:59:59.000Z")).not.toThrow();
    expect(() => assertReportUpdateAllowed(period, "replace", "2026-11-22T14:00:00.000Z")).toThrow(
      /not allowed after soft close/,
    );
    expect(() => assertReportUpdateAllowed(period, "append", "2026-11-22T14:00:00.000Z")).not.toThrow();
    expect(() => assertReportUpdateAllowed(period, "event-log", "2026-11-23T00:00:00.000Z")).not.toThrow();
  });

  test("keeps prior cursor versions while projecting the latest and flags substantive block conflicts", async () => {
    const file = await storePath();
    const base = {
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily" as const,
        timezone: "America/Chicago" as const,
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "cursor-1",
      sources: [{ id: "event-1", uri: "gh:issue/1", observedAt: "2026-11-22T05:40:00.000Z" }],
      blocks: [{ id: "change-1", text: "Stored paragraph.", relatedAt: "2026-11-22T05:40:00.000Z" }],
    };
    const first = makeReportRecord(base, "2026-11-22T05:45:01.000Z");
    const second = makeReportRecord(
      { ...base, sourceCursor: "cursor-2", asOf: "2026-11-22T05:50:00.000Z" },
      "2026-11-22T05:50:01.000Z",
    );
    await appendReportRecord(file, first);
    await appendReportRecord(file, second);

    expect((await readReportRecords(file)).map((record) => record.sourceCursor)).toEqual(["cursor-1", "cursor-2"]);
    expect((await readCurrentReportRecords(file)).map((record) => record.sourceCursor)).toEqual(["cursor-2"]);
    expect(reportBlockConflict("Stored  paragraph.\n", "Stored paragraph.")).toBe(false);
    expect(reportBlockConflict("Stored paragraph.", "Human changed paragraph.")).toBe(true);
    const identityFirst = makeReportRecord(
      { ...base, templateVersion: "daily-v1\nsource", sourceCursor: "cursor" },
      "2026-11-22T05:45:01.000Z",
    );
    const identitySecond = makeReportRecord(
      { ...base, templateVersion: "daily-v1", sourceCursor: "source\ncursor" },
      "2026-11-22T05:45:01.000Z",
    );
    expect(identityFirst.id).not.toBe(identitySecond.id);
  });

  test("compares conflicting prose as line-level hunks while preserving both originals", () => {
    const comparison = compareReportBlock("Opening\nold detail\nClosing", "Opening\nnew detail\nClosing");
    expect(comparison.conflict).toBe(true);
    expect(comparison.hunks).toEqual([{ before: "old detail", after: "new detail", beforeLines: ["old detail"], afterLines: ["new detail"] }]);
    expect(comparison.storedText).toBe("Opening\nold detail\nClosing");
    expect(comparison.renderedText).toBe("Opening\nnew detail\nClosing");
    expect(compareReportBlock("a\nold one\nb\nold two\nc", "a\nnew one\nb\nnew two\nc").hunks).toEqual([
      { before: "old one", after: "new one", beforeLines: ["old one"], afterLines: ["new one"] },
      { before: "old two", after: "new two", beforeLines: ["old two"], afterLines: ["new two"] },
    ]);
    expect(compareReportBlock("same  prose", "same\nprose").conflict).toBe(false);
  });

  test("property: line diff hunks preserve both inputs with minimal edits (independent LCS oracle)", () => {
    // Deterministic generated sequences exercise repeats, insertions, deletions,
    // and whitespace-only equivalence without making failures irreproducible.
    let seed = 0x5eed1234;
    const next = (max: number) => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return seed % max;
    };
    const alphabet = ["alpha", "beta", "gamma", "delta", "alpha ", " beta"];
    const lcsLength = (left: string[], right: string[]): number => {
      const table = Array.from({ length: left.length + 1 }, () => new Uint16Array(right.length + 1));
      for (let i = left.length - 1; i >= 0; i--) {
        for (let j = right.length - 1; j >= 0; j--) {
          table[i]![j] = left[i] === right[j] ? table[i + 1]![j + 1]! + 1 : Math.max(table[i + 1]![j]!, table[i]![j + 1]!);
        }
      }
      return table[0]![0]!;
    };

    for (let sample = 0; sample < 2000; sample++) {
      const left = Array.from({ length: next(9) + 1 }, () => alphabet[next(alphabet.length)]!);
      const right = Array.from({ length: next(9) + 1 }, () => alphabet[next(alphabet.length)]!);
      const stored = left.join("\n");
      const rendered = right.join("\n");
      const comparison = compareReportBlock(stored, rendered);
      if (stored.replace(/\s+/g, " ").trim() === rendered.replace(/\s+/g, " ").trim()) continue;
      const changedLines = comparison.hunks.reduce((count, hunk) =>
        count + (hunk.before === "" ? 0 : hunk.before.split("\n").length) + (hunk.after === "" ? 0 : hunk.after.split("\n").length), 0);
      // Oracle: for line sequences, a shortest edit script has n+m-2*LCS edits.
      expect(comparison.conflict).toBe(true);
      expect(comparison.storedText).toBe(stored);
      expect(comparison.renderedText).toBe(rendered);
      expect(changedLines).toBe(left.length + right.length - 2 * lcsLength(left, right));
    }
  });

  test("bounded-work fallback retains both complete inputs in one truthful hunk", () => {
    const stored = Array.from({ length: 450 }, (_, index) => `stored-${index}`).join("\n");
    const rendered = Array.from({ length: 450 }, (_, index) => `rendered-${index}`).join("\n");
    const comparison = compareReportBlock(stored, rendered);
    expect(comparison.hunks).toEqual([{ before: stored, after: rendered, beforeLines: stored.split("\n"), afterLines: rendered.split("\n") }]);
  });

  test("diff hunks distinguish a blank line from an absent side", () => {
    const comparison = compareReportBlock("a\n\nb\nold", "a\nb\nnew");
    expect(comparison.hunks[0]).toEqual({ before: "", after: "", beforeLines: [""], afterLines: [] });
  });

  test("resolves either side by appending a version and leaves the frozen snapshot unchanged", async () => {
    const file = await storePath();
    const record = makeReportRecord({
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily",
        timezone: "America/Chicago",
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "cursor-before-resolution",
      sources: [{ id: "event-1", uri: "gh:issue/12350", observedAt: "2026-11-22T05:40:00.000Z" }],
      blocks: [{ id: "change-1", text: "Human-edited wording.", relatedAt: "2026-11-22T05:40:00.000Z" }],
    }, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(file, record);
    await appendReportSnapshot(file, record, "2026-11-22T14:00:00.000Z");

    const comparison = compareReportBlock("Human-edited wording.", "New generated wording.");
    const resolved = await appendReportBlockConflictResolution(
      file,
      record,
      "change-1",
      comparison,
      "rendered",
      "2026-11-22T13:59:59.000Z",
      undefined,
      "rendered-record",
    );

    expect(resolved.inserted).toBe(true);
    expect(resolved.record.blocks[0]).toMatchObject({
      id: "change-1",
      text: "New generated wording.",
      resolution: {
        sourceRecordId: record.id,
        strategy: "rendered",
        comparedRecordId: "rendered-record",
        resolvedAt: "2026-11-22T13:59:59.000Z",
      },
    });
    expect(await readReportRecords(file)).toEqual([record, resolved.record]);
    expect(await readCurrentReportRecords(file)).toEqual([resolved.record]);
    expect((await readReportSnapshots(file))[0]?.record).toEqual(record);

    const repeated = await appendReportBlockConflictResolution(
      file,
      record,
      "change-1",
      comparison,
      "rendered",
      "2026-11-22T14:00:01.000Z",
      undefined,
      "rendered-record",
    );
    expect(repeated.inserted).toBe(false);
    expect(await readReportRecords(file)).toEqual([record, resolved.record]);
  });

  test("rejects a same-ID predecessor with changed content and preserves retry timestamps", async () => {
    const file = await storePath();
    const canonical = makeReportRecord({
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily",
        timezone: "America/Chicago",
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "cursor-predecessor-content",
      sources: [],
      blocks: [
        { id: "change-1", text: "Target wording.", relatedAt: "2026-11-22T05:40:00.000Z" },
        { id: "change-2", text: "Other canonical wording.", relatedAt: "2026-11-22T05:40:00.000Z" },
      ],
    }, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(file, canonical);
    const altered = makeReportRecord({
      ...canonical,
      blocks: [canonical.blocks[0]!, { ...canonical.blocks[1]!, text: "Tampered unrelated wording." }],
    }, canonical.generatedAt);

    await expect(appendReportBlockConflictResolution(
      file,
      altered,
      "change-1",
      compareReportBlock("Target wording.", "Rendered wording."),
      "rendered",
      "2026-11-22T13:00:00.000Z",
    )).rejects.toThrow(/supplied source differs/);
    expect(await readReportRecords(file)).toEqual([canonical]);
  });

  test("requires resolution event time to follow its predecessor", async () => {
    const file = await storePath();
    const record = makeReportRecord({
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily",
        timezone: "America/Chicago",
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "cursor-resolution-order",
      sources: [],
      blocks: [{ id: "change-1", text: "Stored.", relatedAt: "2026-11-22T05:40:00.000Z" }],
    }, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(file, record);
    const backdatedCopy = { ...record, generatedAt: "2026-11-22T05:44:00.000Z" };
    await expect(appendReportBlockConflictResolution(
      file,
      backdatedCopy,
      "change-1",
      compareReportBlock("Stored.", "Rendered."),
      "rendered",
      "2026-11-22T05:44:30.000Z",
    )).rejects.toThrow(/later than the persisted source record/);
  });

  test("accepts an edited or reviewer-combined block as an append-only resolution", async () => {
    const file = await storePath();
    const record = makeReportRecord({
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily",
        timezone: "America/Chicago",
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "cursor-edit-each-combine",
      sources: [{ id: "event-1", uri: "gh:issue/12350", observedAt: "2026-11-22T05:40:00.000Z" }],
      blocks: [
        { id: "change-1", text: "Stored opening.\nStored close.", relatedAt: "2026-11-22T05:40:00.000Z" },
        { id: "change-2", text: "Second stored block.", relatedAt: "2026-11-22T05:40:00.000Z" },
      ],
    }, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(file, record);

    const edited = await appendReportBlockConflictResolution(
      file,
      record,
      "change-1",
      compareReportBlock("Stored opening.\nStored close.", "Generated opening.\nGenerated close."),
      "edited",
      "2026-11-22T13:00:00.000Z",
      "Edited opening.\nEdited close.",
    );
    expect(edited.record.blocks[0]).toMatchObject({
      text: "Edited opening.\nEdited close.",
      resolution: { sourceRecordId: record.id, strategy: "edited", resolvedAt: "2026-11-22T13:00:00.000Z" },
    });

    const combined = await appendReportBlockConflictResolution(
      file,
      edited.record,
      "change-2",
      compareReportBlock("Second stored block.", "Second generated block."),
      "combined",
      "2026-11-22T13:30:00.000Z",
      "Reviewer-combined block.",
    );
    expect(combined.record.blocks[1]).toMatchObject({
      text: "Reviewer-combined block.",
      resolution: { sourceRecordId: edited.record.id, strategy: "combined", resolvedAt: "2026-11-22T13:30:00.000Z" },
    });
    expect((await readReportRecords(file)).map((item) => item.id)).toEqual([record.id, edited.record.id, combined.record.id]);
  });

  test("rejects stale or post-soft-close conflict resolutions", async () => {
    const file = await storePath();
    const record = makeReportRecord({
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily",
        timezone: "America/Chicago",
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "cursor-stale-resolution",
      sources: [{ id: "event-1", uri: "gh:issue/12350", observedAt: "2026-11-22T05:40:00.000Z" }],
      blocks: [{ id: "change-1", text: "Current stored wording.", relatedAt: "2026-11-22T05:40:00.000Z" }],
    }, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(file, record);
    const staleComparison = compareReportBlock("Older stored wording.", "Generated wording.");

    await expect(appendReportBlockConflictResolution(
      file,
      record,
      "change-1",
      staleComparison,
      "rendered",
      "2026-11-22T13:00:00.000Z",
    )).rejects.toThrow(/stored block has changed/);
    const currentComparison = compareReportBlock("Current stored wording.", "Generated wording.");
    await expect(appendReportBlockConflictResolution(
      file,
      record,
      "change-1",
      currentComparison,
      "rendered",
      "2026-11-22T14:00:00.000Z",
    )).rejects.toThrow(/not allowed after soft close/);
    expect((await readReportRecords(file)).map((item) => item.id)).toEqual([record.id]);
  });

  test("allows only one resolution to win from a report version", async () => {
    const file = await storePath();
    const record = makeReportRecord({
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily",
        timezone: "America/Chicago",
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "cursor-concurrent-resolutions",
      sources: [{ id: "event-1", uri: "gh:issue/12350", observedAt: "2026-11-22T05:40:00.000Z" }],
      blocks: [
        { id: "change-1", text: "Stored one.", relatedAt: "2026-11-22T05:40:00.000Z" },
        { id: "change-2", text: "Stored two.", relatedAt: "2026-11-22T05:40:00.000Z" },
      ],
    }, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(file, record);

    const outcomes = await Promise.allSettled([
      appendReportBlockConflictResolution(file, record, "change-1", compareReportBlock("Stored one.", "Rendered one."), "rendered", "2026-11-22T13:00:00.000Z"),
      appendReportBlockConflictResolution(file, record, "change-2", compareReportBlock("Stored two.", "Rendered two."), "rendered", "2026-11-22T13:00:01.000Z"),
    ]);
    expect(outcomes.filter((outcome) => outcome.status === "fulfilled")).toHaveLength(1);
    expect(outcomes.filter((outcome) => outcome.status === "rejected")).toHaveLength(1);
    expect(await readReportRecords(file)).toHaveLength(2);
  });

  test("remote resolution claim selects one winner across independent store clients", async () => {
    const documents = new Map<string, unknown>();
    const remote = (): AppendOnlyDocumentRemote => ({
      async load(store) {
        return documents.has(store) ? { version: 1, document: structuredClone(documents.get(store)) } : null;
      },
      async insertIfAbsent(store, _version, document) {
        if (documents.has(store)) return false;
        documents.set(store, structuredClone(document));
        return true;
      },
      async listPrefix(prefix) {
        return [...documents.entries()]
          .filter(([store]) => store.startsWith(prefix))
          .map(([store, document]) => ({ store, version: 1, document: structuredClone(document) }));
      },
    });
    const firstClient = remote();
    const secondClient = remote();
    const record = makeReportRecord({
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily",
        timezone: "America/Chicago",
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "cursor-independent-resolution-clients",
      sources: [],
      blocks: [
        { id: "change-1", text: "Stored one.", relatedAt: "2026-11-22T05:40:00.000Z" },
        { id: "change-2", text: "Stored two.", relatedAt: "2026-11-22T05:40:00.000Z" },
      ],
    }, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(firstClient, record);

    const outcomes = await Promise.allSettled([
      appendReportBlockConflictResolution(firstClient, record, "change-1", compareReportBlock("Stored one.", "Rendered one."), "rendered", "2026-11-22T13:00:00.000Z"),
      appendReportBlockConflictResolution(secondClient, record, "change-2", compareReportBlock("Stored two.", "Rendered two."), "rendered", "2026-11-22T13:00:01.000Z"),
    ]);
    expect(outcomes.filter((outcome) => outcome.status === "fulfilled")).toHaveLength(1);
    expect(outcomes.filter((outcome) => outcome.status === "rejected")).toHaveLength(1);
    expect((await readReportRecords(firstClient)).filter((candidate) => candidate.id !== record.id)).toHaveLength(1);
  });

  test("remote current-head claim selects one competing generation across independent clients", async () => {
    const documents = new Map<string, unknown>();
    const remote = (): AppendOnlyDocumentRemote => ({
      async load(store) {
        return documents.has(store) ? { version: 1, document: structuredClone(documents.get(store)) } : null;
      },
      async insertIfAbsent(store, _version, document) {
        if (documents.has(store)) return false;
        documents.set(store, structuredClone(document));
        return true;
      },
      async listPrefix(prefix) {
        return [...documents.entries()]
          .filter(([store]) => store.startsWith(prefix))
          .map(([store, document]) => ({ store, version: 1, document: structuredClone(document) }));
      },
    });
    const firstClient = remote();
    const secondClient = remote();
    const input = {
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily" as const,
        timezone: "America/Chicago" as const,
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sources: [],
      blocks: [{ id: "block", text: "generation", relatedAt: "2026-11-22T05:40:00.000Z" }],
    };
    const original = makeReportRecord({ ...input, sourceCursor: "original" }, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(firstClient, original);

    const outcomes = await Promise.allSettled([
      appendReportRecord(firstClient, makeReportRecord({ ...input, sourceCursor: "generation-a" }, "2026-11-22T05:46:01.000Z")),
      appendReportRecord(secondClient, makeReportRecord({ ...input, sourceCursor: "generation-b" }, "2026-11-22T05:46:02.000Z")),
    ]);
    expect(outcomes.filter((outcome) => outcome.status === "fulfilled")).toHaveLength(1);
    expect(outcomes.filter((outcome) => outcome.status === "rejected")).toHaveLength(1);
    expect(await readCurrentReportRecords(firstClient)).toHaveLength(1);
  });

  test("remote current-head claim arbitrates a generation against a resolution", async () => {
    const documents = new Map<string, unknown>();
    const remote = (): AppendOnlyDocumentRemote => ({
      async load(store) {
        return documents.has(store) ? { version: 1, document: structuredClone(documents.get(store)) } : null;
      },
      async insertIfAbsent(store, _version, document) {
        if (documents.has(store)) return false;
        documents.set(store, structuredClone(document));
        return true;
      },
      async listPrefix(prefix) {
        return [...documents.entries()]
          .filter(([store]) => store.startsWith(prefix))
          .map(([store, document]) => ({ store, version: 1, document: structuredClone(document) }));
      },
    });
    const firstClient = remote();
    const secondClient = remote();
    const input = {
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily" as const,
        timezone: "America/Chicago" as const,
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sources: [],
      blocks: [{ id: "block", text: "stored", relatedAt: "2026-11-22T05:40:00.000Z" }],
    };
    const original = makeReportRecord({ ...input, sourceCursor: "original" }, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(firstClient, original);

    const outcomes = await Promise.allSettled([
      appendReportBlockConflictResolution(
        firstClient,
        original,
        "block",
        compareReportBlock("stored", "rendered"),
        "rendered",
        "2026-11-22T13:00:00.000Z",
      ),
      appendReportRecord(secondClient, makeReportRecord(
        { ...input, sourceCursor: "new-generation", blocks: [{ ...input.blocks[0]!, text: "new generation" }] },
        "2026-11-22T13:00:01.000Z",
      )),
    ]);
    expect(outcomes.filter((outcome) => outcome.status === "fulfilled")).toHaveLength(1);
    expect(outcomes.filter((outcome) => outcome.status === "rejected")).toHaveLength(1);
    expect(await readCurrentReportRecords(firstClient)).toHaveLength(1);
  });

  test("remote retry reuses the winning resolution timestamp after an interrupted event append", async () => {
    const documents = new Map<string, unknown>();
    let failResolutionEventOnce = true;
    const remote: AppendOnlyDocumentRemote = {
      async load(store) {
        return documents.has(store) ? { version: 1, document: structuredClone(documents.get(store)) } : null;
      },
      async insertIfAbsent(store, _version, document) {
        const eventRecord = (document as { record?: { blocks?: { resolution?: unknown }[] } }).record;
        if (store.startsWith("c7-report:v1:report.generated:") &&
            eventRecord?.blocks?.some((block) => block.resolution) && failResolutionEventOnce) {
          failResolutionEventOnce = false;
          throw new Error("simulated event-store interruption");
        }
        if (documents.has(store)) return false;
        documents.set(store, structuredClone(document));
        return true;
      },
      async listPrefix(prefix) {
        return [...documents.entries()]
          .filter(([store]) => store.startsWith(prefix))
          .map(([store, document]) => ({ store, version: 1, document: structuredClone(document) }));
      },
    };
    const record = makeReportRecord({
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily",
        timezone: "America/Chicago",
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "cursor-interrupted-resolution-append",
      sources: [],
      blocks: [{ id: "change-1", text: "Stored wording.", relatedAt: "2026-11-22T05:40:00.000Z" }],
    }, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(remote, record);
    const comparison = compareReportBlock("Stored wording.", "Rendered wording.");
    await expect(appendReportBlockConflictResolution(
      remote,
      record,
      "change-1",
      comparison,
      "rendered",
      "2026-11-22T13:59:59.000Z",
    )).rejects.toThrow("simulated event-store interruption");

    const retry = await appendReportBlockConflictResolution(
      remote,
      record,
      "change-1",
      comparison,
      "rendered",
      "2026-11-22T14:00:01.000Z",
    );
    expect(retry.record.blocks[0]?.resolution?.resolvedAt).toBe("2026-11-22T13:59:59.000Z");
    expect(await readCurrentReportRecords(remote)).toEqual([retry.record]);
  });

  test("uses append-only permanent document keys for report history and snapshots", async () => {
    const documents = new Map<string, unknown>();
    const remote: AppendOnlyDocumentRemote = {
      async load(store) {
        return documents.has(store) ? { version: 1, document: structuredClone(documents.get(store)) } : null;
      },
      async insertIfAbsent(store, _version, document) {
        if (documents.has(store)) return false;
        documents.set(store, structuredClone(document));
        return true;
      },
      async listPrefix(prefix) {
        return [...documents.entries()]
          .filter(([store]) => store.startsWith(prefix))
          .map(([store, document]) => ({ store, version: 1, document: structuredClone(document) }));
      },
    };
    const base = {
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily" as const,
        timezone: "America/Chicago" as const,
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "cursor-1",
      sources: [{ id: "event-1", uri: "gh:issue/12349", observedAt: "2026-11-22T05:40:00.000Z" }],
      blocks: [{ id: "change-1", text: "Original report.", relatedAt: "2026-11-22T05:40:00.000Z" }],
    };
    const record = makeReportRecord(base, "2026-11-22T05:45:01.000Z");
    expect((await appendReportRecord(remote, record)).inserted).toBe(true);
    const rerun = await appendReportRecord(remote, makeReportRecord(base, "2026-11-22T05:46:01.000Z"));
    expect(rerun.inserted).toBe(false);
    expect(rerun.record.generatedAt).toBe(record.generatedAt);
    await expect(
      appendReportRecord(
        remote,
        makeReportRecord(
          { ...base, blocks: [{ ...base.blocks[0]!, text: "Conflicting content." }] },
          "2026-11-22T05:47:01.000Z",
        ),
      ),
    ).rejects.toBeInstanceOf(ReportRecordConflictError);
    const snapshot = await appendReportSnapshot(remote, record, "2026-11-22T14:00:00.000Z");
    const snapshotRerun = await appendReportSnapshot(remote, record, "2026-11-22T14:05:00.000Z");
    expect(snapshot.inserted).toBe(true);
    expect(snapshotRerun.inserted).toBe(false);
    expect(snapshotRerun.snapshot.snapshotAt).toBe("2026-11-22T14:00:00.000Z");
    expect(await readReportRecords(remote)).toEqual([record]);
    expect(await readReportSnapshots(remote)).toEqual([snapshot.snapshot]);
    expect(documents.size).toBe(4); // generation, snapshot, record-ID claim, and current-head claim
  });

  test("remote generation and snapshot conflict checks are atomic across event types", async () => {
    const documents = new Map<string, unknown>();
    const remote: AppendOnlyDocumentRemote = {
      async load(store) {
        return documents.has(store) ? { version: 1, document: structuredClone(documents.get(store)) } : null;
      },
      async insertIfAbsent(store, _version, document) {
        if (documents.has(store)) return false;
        documents.set(store, structuredClone(document));
        return true;
      },
      async listPrefix(prefix) {
        return [...documents.entries()]
          .filter(([store]) => store.startsWith(prefix))
          .map(([store, document]) => ({ store, version: 1, document: structuredClone(document) }));
      },
    };
    const base = {
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily" as const,
        timezone: "America/Chicago" as const,
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "concurrent-cursor",
      sources: [],
      blocks: [{ id: "block", text: "original", relatedAt: "2026-11-22T05:40:00.000Z" }],
    };
    const generated = makeReportRecord(base, "2026-11-22T05:45:01.000Z");
    const conflicting = makeReportRecord(
      { ...base, blocks: [{ ...base.blocks[0]!, text: "different" }] },
      "2026-11-22T05:45:02.000Z",
    );
    const results = await Promise.allSettled([
      appendReportRecord(remote, generated),
      appendReportSnapshot(remote, conflicting, "2026-11-22T14:00:00.000Z"),
    ]);
    expect(results.filter((result) => result.status === "fulfilled")).toHaveLength(1);
    expect(results.filter((result) => result.status === "rejected")).toHaveLength(1);
    const records = await readReportRecords(remote);
    const snapshots = await readReportSnapshots(remote);
    const stored = records[0] ?? snapshots[0]?.record;
    expect(stored).toBeDefined();
    if (stored) {
      expect(records.every((item) => item.blocks[0]?.text === stored.blocks[0]?.text)).toBe(true);
      expect(snapshots.every((item) => item.record.blocks[0]?.text === stored.blocks[0]?.text)).toBe(true);
    }
  });

  test("canonical JSON retries tolerate omitted optional values", async () => {
    const file = await storePath();
    const input = {
      period: {
        id: "daily:2026-11-21:America/Chicago",
        kind: "daily" as const,
        timezone: "America/Chicago" as const,
        periodStart: "2026-11-21T06:00:00.000Z",
        periodEnd: "2026-11-22T06:00:00.000Z",
        softCloseAt: "2026-11-22T14:00:00.000Z",
      },
      asOf: "2026-11-22T05:45:00.000Z",
      templateVersion: "daily-v1",
      sourceCursor: "optional-links",
      sources: [],
      blocks: [
        {
          id: "b",
          text: "Text",
          relatedAt: "2026-11-22T05:40:00.000Z",
          links: undefined,
        },
      ],
    };
    const first = await appendReportRecord(file, makeReportRecord(input, "2026-11-22T05:45:01.000Z"));
    const rerun = await appendReportRecord(file, makeReportRecord(input, "2026-11-22T05:45:02.000Z"));
    expect(first.inserted).toBe(true);
    expect(rerun.inserted).toBe(false);
  });

  test("allows a persistent store path override without a second datastore", () => {
    expect(defaultReportRecordsFile({ PM_AGG_REPORT_RECORDS_FILE: "/var/lib/pm/report-records.jsonl" })).toBe(
      "/var/lib/pm/report-records.jsonl",
    );
  });
});
