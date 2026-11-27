// C7 leaf 4 (#12351): project spaces and reports timeline index integration.
//
// Oracles:
// - project spaces keep the four stable landings with visibly-unavailable
//   (never invented) missing sections;
// - the current quarter is discoverable even with no stored records;
// - timeline nests quarter -> month -> week -> day from stored report
//   records and references (never owns) design milestones and special
//   reports; C6 index/mobile and C8 ingestion stay external interfaces.
import { describe, expect, test } from "bun:test";
import {
  SPACES,
  buildProjectSpace,
  buildTimelineIndex,
  renderProjectSpace,
  timelineIndexEntries,
  verifyTimelineLinks,
} from "../src/project-spaces-timeline.ts";
import { makeReportRecord } from "../src/report-records.ts";
import {
  dailyReportPeriod,
  monthlyReportPeriod,
  quarterlyReportPeriod,
  weeklyReportPeriod,
} from "../src/report-periods.ts";

function record(kind: "daily" | "weekly" | "monthly" | "quarterly", id: string, cursor: string) {
  const period =
    kind === "daily"
      ? dailyReportPeriod(id)
      : kind === "weekly"
        ? weeklyReportPeriod(id)
        : kind === "monthly"
          ? monthlyReportPeriod(id)
          : quarterlyReportPeriod(id);
  return makeReportRecord(
    {
      period,
      asOf: "2026-11-24T18:00:00.000Z",
      templateVersion: "c7/v1",
      sourceCursor: cursor,
      sources: [],
      blocks: [{ id: `b-${cursor}`, text: `block ${cursor}`, relatedAt: "2026-11-24T17:00:00.000Z" }],
    },
    "2026-11-24T18:00:00.000Z",
  );
}

describe("project spaces", () => {
  test("four stable landings; missing sections stay visibly unavailable", () => {
    expect(SPACES).toEqual(["Fleet", "PM stack", "Atlas", "ID"]);
    const space = buildProjectSpace({
      key: "Fleet",
      title: "Fleet",
      exists: "queue moves",
    });
    expect(space.key).toBe("Fleet");
    const wanted = space.sections.find((s) => s.name === "wanted")!;
    expect(wanted.available).toBe(false);
    expect(wanted.text).toBeNull();
    const rendered = renderProjectSpace(space);
    expect(rendered).toContain("# Fleet");
    expect(rendered).toContain("wanted: not available");
    expect(rendered).not.toContain("wanted: invented");
  });

  test("related spaces link only the four stable keys", () => {
    expect(() => buildProjectSpace({ key: "Atlas", title: "Atlas", related: ["ID", "Nope" as never] })).toThrow(
      /unknown related space/,
    );
  });
});

describe("timeline index", () => {
  test("current quarter is discoverable with no stored records", () => {
    const timeline = buildTimelineIndex({ records: [], asOf: "2026-11-24T18:00:00.000Z" });
    const current = timeline.quarters.find((q) => q.current);
    expect(current).toBeDefined();
    expect(current!.quarter).toBe("2026-Q4");
    expect(current!.empty).toBe(true);
    expect(current!.months).toEqual([]);
  });

  test("records nest quarter -> month -> week -> day; milestones attach by day", () => {
    const timeline = buildTimelineIndex({
      records: [
        record("quarterly", "2026-Q4", "q1"),
        record("monthly", "2026-11", "m1"),
        record("weekly", "2026-11-23", "w1"),
        record("daily", "2026-11-24", "d1"),
      ],
      asOf: "2026-11-24T18:00:00.000Z",
      designMilestones: [{ ref: "developing-today/code#10", title: "Report pages", day: "2026-11-24" }],
      specialReports: [{ id: "special-1", title: "Outage note", day: "2026-11-24" }],
    });
    const quarter = timeline.quarters.find((q) => q.quarter === "2026-Q4")!;
    expect(quarter.empty).toBe(false);
    const month = quarter.months.find((m) => m.month === "2026-11")!;
    const week = month.weeks.find((w) => w.week === "2026-11-23")!;
    const day = week.days.find((d) => d.day === "2026-11-24")!;
    expect(day.recordIds.length).toBe(1);
    expect(day.designMilestones.map((m) => m.ref)).toEqual(["developing-today/code#10"]);
    expect(day.specialReports.map((s) => s.id)).toEqual(["special-1"]);
    const missing = verifyTimelineLinks(timeline, {
      recordIds: new Set(timeline.records.map((r) => r.recordId)),
      designRefs: new Set(["developing-today/code#10"]),
    });
    expect(missing).toEqual([]);
  });

  test("C6/C8 stay external: only reference entries are exported, no search is implemented", () => {
    const timeline = buildTimelineIndex({
      records: [record("daily", "2026-11-24", "d9")],
      asOf: "2026-11-24T18:00:00.000Z",
    });
    const entries = timelineIndexEntries(timeline);
    expect(entries.length).toBeGreaterThan(0);
    for (const e of entries) expect(typeof e.text).toBe("string");
    // The module exposes references for external indexers; it owns no index.
    expect("search" in timeline).toBe(false);
  });
});
