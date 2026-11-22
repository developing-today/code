import { describe, expect, test } from "bun:test";
import { renderReportReview, reportBlockReviews } from "../src/report-review.ts";
import type { ReportRecord } from "../src/report-records.ts";

const record = (id: string, text: string, generatedAt: string): ReportRecord => ({
  id,
  generatedAt,
  period: {
    id: "daily:2026-11-21:America/Chicago",
    kind: "daily",
    timezone: "America/Chicago",
    periodStart: "2026-11-21T06:00:00.000Z",
    periodEnd: "2026-11-22T06:00:00.000Z",
    softCloseAt: "2026-11-21T14:00:00.000Z",
  },
  asOf: "2026-11-21T12:00:00.000Z",
  templateVersion: "c7-rollup-v1",
  sourceCursor: id,
  sources: [],
  blocks: [{ id: "narrative", text, relatedAt: "2026-11-21T12:00:00.000Z" }],
});

describe("C7 report review UI", () => {
  test("shows both prose versions and requires an explicit stored/rendered/edited/combined choice", () => {
    const stored = record("stored", "Human wording <kept>.", "2026-11-21T13:00:00.000Z");
    const rendered = record("rendered", "Generated wording.", "2026-11-21T14:00:00.000Z");
    const reviews = reportBlockReviews([stored, rendered], [stored]);

    expect(reviews).toHaveLength(1);
    const html = renderReportReview(reviews, 2);
    for (const phrase of ["Human wording &lt;kept&gt;.", "Generated wording.", "Keep stored wording", "Use rendered wording", "Use my edited wording", "Combine the two"]) {
      expect(html).toContain(phrase);
    }
    expect(html).toContain('name="strategy-0" value="stored" required');
    expect(html).toContain("Show indexed change");
  });
});
