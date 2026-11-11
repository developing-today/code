// Fixture tests for the flow-analytics charts (#4830, step 3 of the
// #1281 split): the rendered charts must carry exactly the #4829
// twin's values for the same window — every throughput count and
// time-in-status value present, the window stated, gaps named (not
// smoothed over), and zero-completion weeks rendered as zeros.
import { describe, expect, test } from "bun:test";
import { computeFlowAnalytics } from "../src/flow-analytics.ts";
import { flowChartsHtml } from "../src/flow-charts.ts";
import type {
  ItemHistoryCoverage,
  ItemHistoryCutoff,
  ItemHistoryItem,
} from "../src/item-history.ts";

const REPO = "developing-today/code";
const W1 = "2026-10-05T00:00:00Z";
const W2 = "2026-10-12T00:00:00Z";
const W3 = "2026-10-19T00:00:00Z";
const W4 = "2026-10-26T00:00:00Z";
const EPIC = `${REPO}#100`;

function coverage(partial: Partial<ItemHistoryCoverage> = {}): ItemHistoryCoverage {
  return { source: "projects-pool", fetchedAt: W1, complete: true, unavailable: false, ...partial };
}

function item(number: number, partial: Partial<ItemHistoryItem> = {}): ItemHistoryItem {
  return {
    ref: `${REPO}#${number}`,
    repo: REPO,
    number,
    kind: "issue",
    state: "open",
    status: "In Progress",
    effort: null,
    blockedBy: [],
    ...partial,
  };
}

function cutoff(cutoffAt: string, items: ItemHistoryItem[], cov?: Partial<ItemHistoryCoverage>): ItemHistoryCutoff {
  return { schemaVersion: 1, cutoffAt, coverage: coverage(cov), items };
}

function completeWindow(): ItemHistoryCutoff[] {
  const a = (p: Partial<ItemHistoryItem>) => item(1, { parentRef: EPIC, ...p });
  const b = (p: Partial<ItemHistoryItem>) => item(2, { parentRef: EPIC, ...p });
  return [
    cutoff(W1, [a({}), b({ status: "Todo" }), item(3, {})]),
    cutoff(W2, [a({}), b({ state: "closed", status: "Done" }), item(3, {})]),
    cutoff(W3, [a({ state: "closed", status: "Done" }), b({ state: "closed", status: "Done" }), item(3, {})]),
    cutoff(W4, [a({ state: "closed", status: "Done" }), b({ state: "closed", status: "Done" }), item(3, {})]),
  ];
}

describe("flow charts (#4830)", () => {
  test("complete window: rendered values match the JSON twin exactly", () => {
    const twin = computeFlowAnalytics(completeWindow());
    const html = flowChartsHtml(twin);

    // The window is stated on the section.
    expect(html).toContain(`data-from="${W1}"`);
    expect(html).toContain(`data-to="${W4}"`);
    expect(html).toContain("4 of 4 cutoffs complete");
    expect(html).not.toContain("flow-gap");

    // Every group, week, and status value in the twin appears in the
    // render with the twin's own number — no second computation.
    for (const group of twin.groups) {
      expect(html).toContain(`data-group="${group.groupRef ?? ""}"`);
      for (const week of group.throughput) {
        expect(html).toContain(`data-cutoff="${week.cutoffAt}" data-completed="${week.completed}"`);
      }
      for (const s of group.timeInStatus) {
        expect(html).toContain(`data-status="${s.status}" data-item-days="${s.itemDays}"`);
      }
    }

    // The known fixture values (same expectations as the #4829 test):
    // B completes in W2, A in W3, W4 is a preserved zero week.
    expect(html).toContain(`data-cutoff="${W2}" data-completed="1"`);
    expect(html).toContain(`data-cutoff="${W3}" data-completed="1"`);
    expect(html).toContain(`data-cutoff="${W4}" data-completed="0"`);
    expect(html).toContain('data-status="Done" data-item-days="21"');
    expect(html).toContain('data-status="In Progress" data-item-days="14"');
    expect(html).toContain('data-status="Todo" data-item-days="7"');
  });

  test("gappy window: the gap is named on the chart, never smoothed over", () => {
    const cutoffs = [
      cutoff(W1, [item(1, { parentRef: EPIC })]),
      cutoff(W2, [item(1, { parentRef: EPIC })], { complete: false }),
      cutoff(W3, [item(1, { parentRef: EPIC, state: "closed", status: "Done" })]),
    ];
    const twin = computeFlowAnalytics(cutoffs);
    expect(twin.window.gaps).toEqual([{ cutoffAt: W2, complete: false, unavailable: false }]);

    const html = flowChartsHtml(twin);
    expect(html).toContain("2 of 3 cutoffs complete");
    expect(html).toContain(`data-gap="${W2}"`);
    expect(html).toContain("(incomplete)");
    expect(html).toContain("intervals spanning a gap are excluded");
    // The gap cutoff is not drawn as a throughput week for any group.
    expect(html).not.toContain(`data-cutoff="${W2}"`);
  });

  test("wide history: the chart grows per week and keeps every week (Codex P2)", () => {
    const base = Date.parse(W1);
    const cutoffs = Array.from({ length: 60 }, (_, i) =>
      cutoff(new Date(base + i * 7 * 86_400_000).toISOString(), [item(1, { parentRef: EPIC })]),
    );
    const twin = computeFlowAnalytics(cutoffs);
    const html = flowChartsHtml(twin);
    const epic = twin.groups.find((g) => g.groupRef === EPIC)!;
    // 60 cutoffs bound 59 weekly intervals in the twin; the chart
    // draws every one of them, at a full slot per week.
    expect(epic.throughput.length).toBe(59);
    expect((html.match(/data-cutoff=/g) ?? []).length).toBe(59);
    expect(html).toContain('viewBox="0 0 3304 110"');
    expect(html).toContain('class="flow-scroll"');
  });

  test("empty history: the section states the empty window, no charts invented", () => {
    const html = flowChartsHtml(computeFlowAnalytics([]));
    expect(html).toContain("no item history stored yet");
    expect(html).not.toContain("flow-tp-week");
  });
});
