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
    expect(html).toContain("((strategy.value==='edited'||strategy.value==='combined')?{resolvedText:text}:{})");
  });

  test("selects the newest substantive alternative past a whitespace-only version", () => {
    const original = record("original", "Original wording.", "2026-11-21T10:00:00.000Z");
    const whitespace = record("whitespace", "Changed  wording.", "2026-11-21T11:00:00.000Z");
    const current = record("current", "Changed wording.", "2026-11-21T12:00:00.000Z");

    const reviews = reportBlockReviews([original, whitespace, current], [current]);
    expect(reviews).toHaveLength(1);
    expect(reviews[0]?.renderedRecord.id).toBe("original");
  });

  test("does not re-offer resolved history, but surfaces a later substantive conflict", () => {
    const stored = record("stored", "Human wording.", "2026-11-21T13:00:00.000Z");
    const rendered = record("rendered", "Generated wording.", "2026-11-21T14:00:00.000Z");
    const resolved = record("resolved", "Human wording.", "2026-11-21T15:00:00.000Z");
    resolved.blocks[0].resolution = {
      sourceRecordId: stored.id,
      strategy: "stored",
      resolvedAt: resolved.generatedAt,
    };

    expect(reportBlockReviews([stored, rendered, resolved], [resolved])).toHaveLength(0);

    const postResolution = record("post-resolution", "New wording.", "2026-11-21T16:00:00.000Z");
    const latest = record("latest", "Latest wording.", "2026-11-21T17:00:00.000Z");
    expect(reportBlockReviews([stored, rendered, resolved, postResolution, latest], [latest]).map((review) => review.renderedRecord.id))
      .toEqual(["post-resolution"]);
  });
});
