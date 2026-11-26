// C7 leaf 3 (#12353): design pages and implementation rollups.
//
// Oracles:
// - a full design-page fixture whose expected member counts are written
//   out by hand, item by item, not recomputed by the code under test;
// - cancelled and duplicate closures are excluded from done and counted
//   apart; a closed issue with no known reason stays unknown;
// - membership and subdesigns come from deriveGenealogy (no second model);
// - change notes come only from stored report records that link the
//   design or one of its members.
import { describe, expect, test } from "bun:test";
import { buildDesignPage, type CloseReason, classifyMember, renderDesignPage } from "../src/design-pages.ts";
import type { ProjectPoolItem, RefKey } from "../src/model.ts";
import { makeReportRecord, type ReportRecord } from "../src/report-records.ts";
import { dailyReportPeriod, monthlyReportPeriod, weeklyReportPeriod } from "../src/report-periods.ts";

const ref = (n: number) => `developing-today/code#${n}` as RefKey;
function item(n: number, partial: Partial<ProjectPoolItem> = {}): ProjectPoolItem {
  return {
    id: `PVTI_${n}`,
    url: "https://example.test/board",
    projectUrl: "https://example.test/board",
    ref: ref(n),
    repo: "developing-today/code",
    number: n,
    issueUrl: `https://github.com/developing-today/code/issues/${n}`,
    title: `Item ${n}`,
    state: "open",
    contentKind: "issue",
    status: null,
    area: null,
    horizon: null,
    kind: null,
    ...partial,
  };
}

const design = (n: number, title: string, partial: Partial<ProjectPoolItem> = {}) =>
  item(n, { title, kind: "Design", labels: ["kind/design"], ...partial });

/** The fixture: roadmap 1 → design 10 → (members 11–19, subdesign 20 → member 21, subdesign 22). */
const pool: ProjectPoolItem[] = [
  item(1, {
    title: "Roadmap",
    kind: "Roadmap item",
    labels: ["kind/roadmap-item"],
  }),
  design(10, "Report pages", { parentRef: ref(1) }),
  item(11, {
    title: "Merged PR",
    contentKind: "pr",
    state: "merged",
    parentRef: ref(10),
  }),
  item(12, { title: "Completed issue", state: "closed", parentRef: ref(10) }),
  item(13, { title: "Not planned", state: "closed", parentRef: ref(10) }),
  item(14, { title: "Duplicate", state: "closed", parentRef: ref(10) }),
  item(15, {
    title: "Closed, reason unknown",
    state: "closed",
    parentRef: ref(10),
  }),
  item(16, { title: "In progress", status: "In Progress", parentRef: ref(10) }),
  item(17, {
    title: "Blocked by open 30",
    parentRef: ref(10),
    blockedBy: [ref(30)],
  }),
  item(18, {
    title: "Was blocked by closed 31",
    parentRef: ref(10),
    blockedBy: [ref(31)],
  }),
  item(19, {
    title: "Open draft PR",
    contentKind: "pr",
    draft: true,
    parentRef: ref(10),
  }),
  design(20, "Subdesign A", { parentRef: ref(10) }),
  item(21, { title: "Member of A", parentRef: ref(20) }),
  design(22, "Subdesign B", { parentRef: ref(10) }),
  item(30, { title: "Open blocker" }),
  item(31, { title: "Closed blocker", state: "closed" }),
  item(40, { title: "Unrelated", labels: ["design/other"] }),
];

const reasons = new Map<RefKey, CloseReason>([
  [ref(12), "completed"],
  [ref(13), "not_planned"],
  [ref(14), "duplicate"],
  [ref(31), "completed"],
]);

function records(): ReportRecord[] {
  const daily = dailyReportPeriod("2026-11-24");
  const weekly = weeklyReportPeriod("2026-11-23");
  const monthly = monthlyReportPeriod("2026-11");
  const base = { templateVersion: "c7/v1", sources: [] };
  return [
    makeReportRecord(
      {
        ...base,
        period: daily,
        asOf: "2026-11-24T18:00:00.000Z",
        sourceCursor: "d1",
        blocks: [
          {
            id: "b1",
            text: "Merged the first page renderer",
            relatedAt: "2026-11-24T17:00:00.000Z",
            links: [ref(11)],
          },
          {
            id: "b2",
            text: "Unrelated fix",
            relatedAt: "2026-11-24T17:00:00.000Z",
            links: [ref(40)],
          },
        ],
      },
      "2026-11-24T18:00:00.000Z",
    ),
    makeReportRecord(
      {
        ...base,
        period: weekly,
        asOf: "2026-11-24T18:00:00.000Z",
        sourceCursor: "w1",
        blocks: [
          {
            id: "w1",
            text: "Design scope settled",
            relatedAt: "2026-11-24T12:00:00.000Z",
            links: [ref(10)],
          },
        ],
      },
      "2026-11-24T18:00:00.000Z",
    ),
    makeReportRecord(
      {
        ...base,
        period: monthly,
        asOf: "2026-11-24T18:00:00.000Z",
        sourceCursor: "m1",
        blocks: [
          {
            id: "m1",
            text: "Month summary",
            relatedAt: "2026-11-24T12:00:00.000Z",
            links: [ref(10)],
          },
        ],
      },
      "2026-11-24T18:00:00.000Z",
    ),
  ];
}

describe("C7 leaf 3 design page", () => {
  const page = buildDesignPage(ref(10), pool, reasons, records(), "2026-11-25T00:00:00.000Z");

  test("full design-page fixture has hand-written member counts", () => {
    // 11 merged + 12 completed = 2 done; 16 status + 19 open PR = 2 in progress;
    // 17 = 1 blocked; 18 (its only blocker is closed) = 1 queued;
    // 13 cancelled, 14 duplicate, 15 unknown. 21 belongs to subdesign A, not here.
    expect(page.counts).toEqual({
      done: 2,
      inProgress: 2,
      blocked: 1,
      queued: 1,
      cancelled: 1,
      duplicate: 1,
      unknown: 1,
      total: 9,
    });
    expect(page.members.map((m) => [m.ref, m.class])).toEqual([
      [ref(11), "implemented"],
      [ref(12), "implemented"],
      [ref(13), "cancelled"],
      [ref(14), "duplicate"],
      [ref(15), "closed-unclassified"],
      [ref(16), "in-progress"],
      [ref(17), "blocked"],
      [ref(18), "queued"],
      [ref(19), "in-progress"],
    ]);
  });

  test("cancelled and duplicate closures never count as done", () => {
    const allDone = buildDesignPage(
      ref(10),
      pool,
      new Map<RefKey, CloseReason>([
        [ref(12), "not_planned"],
        [ref(13), "not_planned"],
        [ref(14), "duplicate"],
      ]),
      [],
      "x",
    );
    expect(allDone.counts.done).toBe(1); // only the merged PR
    expect(allDone.counts.cancelled).toBe(2);
    expect(allDone.counts.duplicate).toBe(1);
  });

  test("a closed issue with no reason stays unknown, never done", () => {
    const none = buildDesignPage(ref(10), pool, new Map(), [], "x");
    expect(none.counts.done).toBe(1);
    expect(none.counts.unknown).toBe(4);
    expect(classifyMember(item(99, { state: "closed" }), undefined, new Set()).class).toBe("closed-unclassified");
  });

  test("blocker reasons name only open blockers", () => {
    expect(page.members.find((m) => m.ref === ref(17))?.blockedBy).toEqual([ref(30)]);
    expect(page.members.find((m) => m.ref === ref(18))?.blockedBy).toEqual([]);
  });

  test("origin and subdesign tree come from the genealogy read model", () => {
    expect(page.origin).toEqual([
      { ref: ref(1), title: "Roadmap", relation: "parent" },
      { ref: ref(1), title: "Roadmap", relation: "roadmap" },
    ]);
    expect(page.subdesigns).toEqual([
      { ref: ref(20), title: "Subdesign A", children: [] },
      { ref: ref(22), title: "Subdesign B", children: [] },
    ]);
    const sub = buildDesignPage(ref(20), pool, reasons, [], "x");
    expect(sub.members.map((m) => m.ref)).toEqual([ref(21)]);
  });

  test("status ladder marks unseen stages unknown, not unreached", () => {
    expect(page.ladder.map((s) => [s.stage, s.state])).toEqual([
      ["idea", "reached"],
      ["planned", "reached"],
      ["initial issue", "reached"],
      ["issue built out", "reached"],
      ["fully designed", "unknown"],
      ["implementation issues", "reached"],
    ]);
    const bare = buildDesignPage(ref(22), pool, reasons, [], "x");
    expect(bare.ladder.filter((s) => s.state === "unknown").map((s) => s.stage)).toEqual([
      "issue built out",
      "fully designed",
      "implementation issues",
    ]);
  });

  test("change notes and longer-period links come only from records that link the design or a member", () => {
    expect(page.changeNotes.map((n) => [n.periodKind, n.blockId, n.text])).toEqual([
      ["daily", "b1", "Merged the first page renderer"],
      ["weekly", "w1", "Design scope settled"],
    ]);
    expect(page.periodPages.map((p) => [p.periodKind, p.periodId])).toEqual([
      ["monthly", "monthly:2026-11:America/Chicago"],
    ]);
  });

  test("the rendered page shows done and the excluded closures separately", () => {
    const md = renderDesignPage(page);
    expect(md).toContain("2 done · 2 in progress · 1 blocked · 1 queued (of 9)");
    expect(md).toContain("1 cancelled · 1 duplicate · 1 closed with unknown reason");
    expect(md).toContain("blocked by developing-today/code#30");
    expect(md).toContain("- fully designed: unknown");
    expect(md).not.toContain("Unrelated fix");
  });

  test("a ref that is not a design is refused", () => {
    expect(() => buildDesignPage(ref(11), pool, reasons, [], "x")).toThrow(/not a design/);
    expect(() => buildDesignPage(ref(999), pool, reasons, [], "x")).toThrow(/not in the pool/);
  });
});
