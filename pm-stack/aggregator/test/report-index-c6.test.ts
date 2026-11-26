import { describe, expect, test } from "bun:test";
import { reportIndexEntries, searchReportIndex } from "../src/report-index.ts";
import type { ReportRecord } from "../src/report-records.ts";

const record = (id: string, title: string, text: string): ReportRecord => ({
  id,
  period: { id: `weekly:${id}:America/Chicago`, kind: "weekly", timezone: "America/Chicago", periodStart: "2026-11-16T06:00:00.000Z", periodEnd: "2026-11-23T06:00:00.000Z", softCloseAt: "2026-11-23T14:00:00.000Z" },
  asOf: "2026-11-23T12:00:00.000Z", generatedAt: "2026-11-23T12:00:00.000Z", templateVersion: "v1", sourceCursor: id,
  sources: [{ id: "issue-15020", uri: "https://github.com/developing-today/code/issues/15020", observedAt: "2026-11-23T12:00:00.000Z" }],
  blocks: [{ id: title, text, relatedAt: "2026-11-22T12:00:00.000Z", links: ["https://github.com/developing-today/code/issues/15020"] }],
});

describe("C6 reports index projection", () => {
  test("searches body, linked issue and title with stable pagination", () => {
    const entries = reportIndexEntries([record("r2", "Second report", "Atlas ingestion recovered") , record("r1", "First report", "Timeline delayed")]);
    expect(searchReportIndex(entries, { q: "ingestion" }).total).toBe(1);
    expect(searchReportIndex(entries, { q: "15020" }).total).toBe(2);
    expect(searchReportIndex(entries, { q: "report", limit: 1 }).records.map((r) => r.id)).toEqual(["r1"]);
  });
});
