// Fixture-snapshot tests for flow analytics (#4829): a complete
// window, a gappy window (gap flagged, not zero-filled), a
// zero-completion week preserved only when coverage is complete,
// and the #3981 throughput unit rule (issues only).
import { describe, expect, test } from "bun:test";
import { computeFlowAnalytics, UNSET_STATUS } from "../src/flow-analytics.ts";
import type { ItemHistoryCoverage, ItemHistoryCutoff, ItemHistoryItem } from "../src/item-history.ts";

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

describe("flow analytics (#4829)", () => {
  test("complete window: time-in-status and throughput per epic, zero week preserved", () => {
    const a = (p: Partial<ItemHistoryItem>) => item(1, { parentRef: EPIC, ...p });
    const b = (p: Partial<ItemHistoryItem>) => item(2, { parentRef: EPIC, ...p });
    const cutoffs = [
      cutoff(W1, [a({}), b({ status: "Todo" }), item(3, {})]),
      cutoff(W2, [a({}), b({ state: "closed", status: "Done" }), item(3, {})]),
      cutoff(W3, [a({ state: "closed", status: "Done" }), b({ state: "closed", status: "Done" }), item(3, {})]),
      cutoff(W4, [a({ state: "closed", status: "Done" }), b({ state: "closed", status: "Done" }), item(3, {})]),
    ];
    const out = computeFlowAnalytics(cutoffs);

    expect(out.window).toEqual({
      fromCutoff: W1,
      toCutoff: W4,
      cutoffCount: 4,
      completeCutoffs: 4,
      gaps: [],
    });

    const epic = out.groups.find((g) => g.groupRef === EPIC)!;
    expect(epic.itemCount).toBe(2);
    // A: In Progress over W1->W2 and W2->W3 (14), Done over W3->W4 (7).
    // B: Todo over W1->W2 (7), Done over W2->W3 and W3->W4 (14).
    expect(epic.timeInStatus).toEqual([
      { status: "Done", itemDays: 21 },
      { status: "In Progress", itemDays: 14 },
      { status: "Todo", itemDays: 7 },
    ]);
    const itemA = epic.items.find((i) => i.ref === `${REPO}#1`)!;
    expect(itemA.byStatus).toEqual([
      { status: "Done", days: 7 },
      { status: "In Progress", days: 14 },
    ]);
    expect(itemA.totalDays).toBe(21);
    // B completes in week W2, A in week W3, week W4 is a preserved zero.
    expect(epic.throughput).toEqual([
      { cutoffAt: W2, completed: 1, completedRefs: [`${REPO}#2`] },
      { cutoffAt: W3, completed: 1, completedRefs: [`${REPO}#1`] },
      { cutoffAt: W4, completed: 0, completedRefs: [] },
    ]);

    const unassigned = out.groups.find((g) => g.groupRef === null)!;
    expect(unassigned.itemCount).toBe(1);
    expect(unassigned.timeInStatus).toEqual([{ status: "In Progress", itemDays: 21 }]);
    expect(unassigned.throughput.every((w) => w.completed === 0)).toBe(true);
  });

  test("gappy window: incomplete cutoff breaks the chain, gap is flagged, nothing zero-filled", () => {
    const a = (p: Partial<ItemHistoryItem>) => item(1, { parentRef: EPIC, ...p });
    const cutoffs = [
      cutoff(W1, [a({})]),
      cutoff(W2, [a({})], { complete: false }),
      cutoff(W3, [a({})]),
      cutoff(W4, [a({ state: "closed", status: "Done" })]),
    ];
    const out = computeFlowAnalytics(cutoffs);

    expect(out.window.cutoffCount).toBe(4);
    expect(out.window.completeCutoffs).toBe(3);
    expect(out.window.gaps).toEqual([{ cutoffAt: W2, complete: false, unavailable: false }]);

    const epic = out.groups.find((g) => g.groupRef === EPIC)!;
    // Only the W3->W4 interval accrues; the two intervals touching
    // the incomplete cutoff accrue nothing.
    expect(epic.timeInStatus).toEqual([{ status: "In Progress", itemDays: 7 }]);
    // The only week is W4 (W1->W2, W2->W3 form no week at all).
    expect(epic.throughput).toEqual([{ cutoffAt: W4, completed: 1, completedRefs: [`${REPO}#1`] }]);
  });

  test("unavailable cutoff is a gap, and items seen only there create no group", () => {
    const cutoffs = [
      cutoff(W1, [item(1, { parentRef: EPIC })]),
      cutoff(W2, [item(9, { parentRef: `${REPO}#999` })], { complete: false, unavailable: true }),
      cutoff(W3, [item(1, { parentRef: EPIC })]),
    ];
    const out = computeFlowAnalytics(cutoffs);
    expect(out.window.gaps).toEqual([{ cutoffAt: W2, complete: false, unavailable: true }]);
    expect(out.groups.map((g) => g.groupRef)).toEqual([EPIC]);
    expect(out.groups[0]!.throughput).toEqual([]);
  });

  test("zero-completion week is preserved when coverage is complete", () => {
    const cutoffs = [cutoff(W1, [item(1, {})]), cutoff(W2, [item(1, {})])];
    const out = computeFlowAnalytics(cutoffs);
    const group = out.groups.find((g) => g.groupRef === null)!;
    expect(group.throughput).toEqual([{ cutoffAt: W2, completed: 0, completedRefs: [] }]);
    expect(group.timeInStatus).toEqual([{ status: "In Progress", itemDays: 7 }]);
  });

  test("throughput counts issues only; PRs still accrue time-in-status", () => {
    const pr = (p: Partial<ItemHistoryItem>) => item(7, { kind: "pr", parentRef: EPIC, ...p });
    const cutoffs = [cutoff(W1, [pr({})]), cutoff(W2, [pr({ state: "merged", status: "Done" })])];
    const out = computeFlowAnalytics(cutoffs);
    const epic = out.groups.find((g) => g.groupRef === EPIC)!;
    expect(epic.throughput).toEqual([{ cutoffAt: W2, completed: 0, completedRefs: [] }]);
    const prRow = epic.items.find((i) => i.ref === `${REPO}#7`)!;
    expect(prRow.kind).toBe("pr");
    expect(prRow.byStatus).toEqual([{ status: "In Progress", days: 7 }]);
  });

  test("unset status is labelled, and empty history yields an empty window", () => {
    const cutoffs = [cutoff(W1, [item(1, { status: null })]), cutoff(W2, [item(1, { status: null })])];
    const out = computeFlowAnalytics(cutoffs);
    expect(out.groups[0]!.timeInStatus).toEqual([{ status: UNSET_STATUS, itemDays: 7 }]);

    const empty = computeFlowAnalytics([]);
    expect(empty.window).toEqual({
      fromCutoff: null,
      toCutoff: null,
      cutoffCount: 0,
      completeCutoffs: 0,
      gaps: [],
    });
    expect(empty.groups).toEqual([]);
  });
  test("throughput credits explicit unassigned membership at the closing cutoff, not the previous parent", () => {
    const cutoffs = [
      cutoff(W1, [item(1, { parentRef: EPIC })]),
      cutoff(W2, [item(1, { state: "closed", status: "Done" })]),
    ];
    const out = computeFlowAnalytics(cutoffs);
    const epic = out.groups.find((g) => g.groupRef === EPIC)!;
    expect(epic.throughput.every((w) => w.completed === 0)).toBe(true);
    expect(epic.throughput.flatMap((w) => w.completedRefs)).toEqual([]);
    const unassigned = out.groups.find((g) => g.groupRef === null)!;
    expect(unassigned.throughput).toEqual([{ cutoffAt: W2, completed: 1, completedRefs: [`${REPO}#1`] }]);
  });
});
