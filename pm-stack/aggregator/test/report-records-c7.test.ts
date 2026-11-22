import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test } from "bun:test";
import {
  appendReportRecord,
  appendReportSnapshot,
  assertReportUpdateAllowed,
  defaultReportRecordsFile,
  makeReportRecord,
  readCurrentReportRecords,
  readReportRecords,
  readReportSnapshots,
  reportBlockConflict,
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
      sources: [{ id: "source-event-42", uri: "gh:developing-today/code/issues/12349", observedAt: "2026-11-22T05:40:00.000Z" }],
      blocks: [{ id: "change-12349", text: "Added the period boundary foundation.", relatedAt: "2026-11-22T05:40:00.000Z" }],
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
      sourceCursor: "source-event-42",
      sources: [{ id: "source-event-42", uri: "gh:issue/12349", observedAt: "2026-11-22T05:40:00.000Z" }],
      blocks: [{ id: "change-12349", text: "Canonical prose.", relatedAt: "2026-11-22T05:40:00.000Z" }],
    }, "2026-11-22T05:45:01.000Z");

    await expect(appendReportRecord(file, { ...canonical, id: "report:forged" }))
      .rejects.toThrow(/canonical contents and ID/);
    await expect(appendReportSnapshot(file, { ...canonical, id: "report:forged" }, "2026-11-22T14:00:00.000Z"))
      .rejects.toThrow(/canonical contents and ID/);
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
      sources: [{ id: "source-event-42", uri: "gh:developing-today/code/issues/12349", observedAt: "2026-11-22T05:40:00.000Z" }],
      blocks: [{ id: "change-12349", text: "Original prose.", relatedAt: "2026-11-22T05:40:00.000Z" }],
    };
    const record = makeReportRecord(input, "2026-11-22T05:45:01.000Z");
    await appendReportRecord(file, record);

    const conflictingRecord = makeReportRecord({
      ...input,
      blocks: [{ ...input.blocks[0]!, text: "Silently changed prose." }],
    }, "2026-11-22T05:46:01.000Z");
    await expect(appendReportRecord(file, conflictingRecord)).rejects.toBeInstanceOf(ReportRecordConflictError);
    expect((await readReportRecords(file))[0]?.blocks[0]?.text).toBe("Original prose.");

    const snapshot = await appendReportSnapshot(file, record, "2026-11-22T14:00:00.000Z");
    expect(snapshot.inserted).toBe(true);
    const snapshotRerun = await appendReportSnapshot(file, makeReportRecord(input, "2026-11-22T05:46:01.000Z"), "2026-11-22T14:01:00.000Z");
    expect(snapshotRerun.inserted).toBe(false);
    expect(snapshotRerun.snapshot.snapshotAt).toBe("2026-11-22T14:00:00.000Z");
    const mutatedSnapshotSource = makeReportRecord({
      ...input,
      blocks: [{ ...input.blocks[0]!, text: "Changed after rollover." }],
    }, "2026-11-22T05:47:01.000Z");
    await expect(appendReportSnapshot(file, mutatedSnapshotSource, "2026-11-22T14:00:00.000Z"))
      .rejects.toBeInstanceOf(ReportRecordConflictError);
    expect((await readReportSnapshots(file))[0]?.record.blocks[0]?.text).toBe("Original prose.");
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
    expect(() => assertReportUpdateAllowed(period, "replace", "2026-11-22T14:00:00.000Z"))
      .toThrow(/not allowed after soft close/);
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
    const second = makeReportRecord({ ...base, sourceCursor: "cursor-2", asOf: "2026-11-22T05:50:00.000Z" }, "2026-11-22T05:50:01.000Z");
    await appendReportRecord(file, first);
    await appendReportRecord(file, second);

    expect((await readReportRecords(file)).map((record) => record.sourceCursor)).toEqual(["cursor-1", "cursor-2"]);
    expect((await readCurrentReportRecords(file)).map((record) => record.sourceCursor)).toEqual(["cursor-2"]);
    expect(reportBlockConflict("Stored  paragraph.\n", "Stored paragraph.")).toBe(false);
    expect(reportBlockConflict("Stored paragraph.", "Human changed paragraph.")).toBe(true);
  });

  test("allows a persistent store path override without a second datastore", () => {
    expect(defaultReportRecordsFile({ PM_AGG_REPORT_RECORDS_FILE: "/var/lib/pm/report-records.jsonl" }))
      .toBe("/var/lib/pm/report-records.jsonl");
  });
});
