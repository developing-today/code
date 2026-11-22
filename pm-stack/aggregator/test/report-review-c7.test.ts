import { describe, expect, test } from "bun:test";
import { renderReportReview, reportBlockReviews, type ReportBlockReview } from "../src/report-review.ts";
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
    const html = renderReportReview(reviews, 2, new Date("2026-11-21T13:59:00.000Z"));
    for (const phrase of ["Human wording &lt;kept&gt;.", "Generated wording.", "Keep stored wording", "Use rendered wording", "Use my edited wording", "Combine the two"]) {
      expect(html).toContain(phrase);
    }
    expect(html).toContain('name="strategy-0" value="stored" required');
    expect(html).toContain("Show indexed change");
    expect(html).toContain("((strategy.value==='edited'||strategy.value==='combined')?{resolvedText:text}:{})");
  });

  test("keeps comparison visible but removes resolution controls at soft close", () => {
    const stored = record("stored", "Human wording.", "2026-11-21T13:00:00.000Z");
    const rendered = record("rendered", "Generated wording.", "2026-11-21T14:00:00.000Z");
    const html = renderReportReview(reportBlockReviews([stored, rendered], [stored]), 2, new Date("2026-11-21T14:00:00.000Z"));

    expect(html).toContain("Human wording.");
    expect(html).toContain("Generated wording.");
    expect(html).toContain("closed for edits");
    expect(html).not.toContain("<form data-report-resolution");
    expect(html).not.toContain("Record resolution");
  });

  test("renders a blank-line deletion differently from an absent diff side", () => {
    const stored = record("stored", "a\n\nb\nold", "2026-11-21T13:00:00.000Z");
    const rendered = record("rendered", "a\nb\nnew", "2026-11-21T14:00:00.000Z");
    const review: ReportBlockReview = {
      periodId: stored.period.id,
      blockId: "narrative",
      storedRecord: stored,
      renderedRecord: rendered,
      comparison: {
        conflict: true,
        storedText: stored.blocks[0]!.text,
        renderedText: rendered.blocks[0]!.text,
        hunks: [{ before: "", after: "", beforeLines: [""], afterLines: [] }],
      },
    };
    const html = renderReportReview([review], 2, new Date("2026-11-21T13:59:00.000Z"));
    expect(html).toContain("<strong>Stored</strong>\n</pre>");
    expect(html).toContain("<strong>Rendered</strong>\n(empty)</pre>");
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
    expect(reportBlockReviews([stored, rendered, resolved, postResolution], [postResolution]).map((review) => review.renderedRecord.id))
      .toEqual(["resolved"]);
    const latest = record("latest", "Latest wording.", "2026-11-21T17:00:00.000Z");
    expect(reportBlockReviews([stored, rendered, resolved, postResolution, latest], [latest]).map((review) => review.renderedRecord.id))
      .toEqual(["post-resolution"]);

    const resolvedAgain = record("resolved-again", "New wording.", "2026-11-21T18:00:00.000Z");
    resolvedAgain.blocks[0].resolution = {
      sourceRecordId: postResolution.id,
      comparedRecordId: latest.id,
      strategy: "stored",
      resolvedAt: resolvedAgain.generatedAt,
    };
    expect(reportBlockReviews([stored, rendered, resolved, postResolution, latest, resolvedAgain], [resolvedAgain])).toHaveLength(0);
  });
});
