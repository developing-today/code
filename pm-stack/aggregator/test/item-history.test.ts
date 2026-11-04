import { afterEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  blockerFirstObserved,
  captureItemHistory,
  isItemHistoryCutoff,
  ItemHistoryStore,
  weeklyCompletions,
  weeklyTransitions,
  type ItemHistoryCoverage,
  type ItemHistoryCutoff,
} from "../src/item-history.ts";
import type { ProjectPoolItem } from "../src/model.ts";

const REPO = "developing-today/code";
const W1 = "2026-10-05T00:00:00Z";
const W2 = "2026-10-12T00:00:00Z";
const W3 = "2026-10-19T00:00:00Z";
const W4 = "2026-10-26T00:00:00Z";

function coverage(partial: Partial<ItemHistoryCoverage> = {}): ItemHistoryCoverage {
  return {
    source: "projects-pool",
    fetchedAt: W1,
    complete: true,
    unavailable: false,
    ...partial,
  };
}

function poolItem(number: number, partial: Partial<ProjectPoolItem> = {}): ProjectPoolItem {
  return {
    id: `PVTI_${number}`,
    url: `https://github.com/orgs/developing-today/projects/4/views/1?pane=issue&itemId=${number}`,
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: `${REPO}#${number}`,
    repo: REPO,
    number,
    issueUrl: `https://github.com/${REPO}/issues/${number}`,
    title: `item ${number}`,
    state: "open",
    contentKind: "issue",
    status: null,
    area: null,
    horizon: null,
    kind: null,
    ...partial,
  };
}

function cutoff(cutoffAt: string, items: ProjectPoolItem[], cov: Partial<ItemHistoryCoverage> = {}): ItemHistoryCutoff {
  return captureItemHistory(items, coverage({ fetchedAt: cutoffAt, ...cov }), cutoffAt);
}

describe("captureItemHistory", () => {
  test("records scope, state, status, effort, membership and blocker edges", () => {
    const captured = cutoff(W1, [
      poolItem(7, {
        state: "open",
        status: "In Progress",
        labels: ["effort/m", "kind/bug"],
        parentRef: `${REPO}#6`,
        epicRef: `${REPO}#5`,
        blockedBy: [`${REPO}#9`, `${REPO}#8`],
      }),
    ]);
    expect(captured.schemaVersion).toBe(1);
    expect(captured.items).toEqual([
      {
        ref: `${REPO}#7`,
        repo: REPO,
        number: 7,
        kind: "issue",
        state: "open",
        status: "In Progress",
        effort: "m",
        parentRef: `${REPO}#6`,
        epicRef: `${REPO}#5`,
        blockedBy: [`${REPO}#8`, `${REPO}#9`],
      },
    ]);
    expect(isItemHistoryCutoff(captured)).toBe(true);
  });

  test("membership change is visible across cutoffs", () => {
    const before = cutoff(W1, [poolItem(7, { epicRef: `${REPO}#5` })]);
    const after = cutoff(W2, [poolItem(7, { epicRef: `${REPO}#11` })]);
    expect(before.items[0]!.epicRef).toBe(`${REPO}#5`);
    expect(after.items[0]!.epicRef).toBe(`${REPO}#11`);
  });
});

describe("isItemHistoryCutoff date gates (#5172)", () => {
  const valid = () => cutoff(W1, [poolItem(7)]);

  test("valid ISO cutoff and coverage stamps pass (control)", () => {
    expect(isItemHistoryCutoff(valid())).toBe(true);
  });

  test("impossible and non-ISO cutoffAt stamps are refused", () => {
    for (const stamp of ["2026-02-30T00:00:00Z", "2026-04-31T00:00:00Z", "March 2, 2026", "03/02/2026"]) {
      expect(isItemHistoryCutoff({ ...valid(), cutoffAt: stamp })).toBe(false);
    }
  });

  test("impossible and non-ISO coverage.fetchedAt stamps are refused", () => {
    for (const stamp of ["not-a-date", "2026-02-30T00:00:00Z"]) {
      const c = valid();
      expect(isItemHistoryCutoff({ ...c, coverage: { ...c.coverage, fetchedAt: stamp } })).toBe(false);
    }
  });
});

describe("weeklyTransitions / weeklyCompletions", () => {
  test("close counts once; a reopen leaves the completed count until re-closed", () => {
    const cutoffs = [
      cutoff(W1, [poolItem(7), poolItem(8)]),
      cutoff(W2, [poolItem(7, { state: "closed" }), poolItem(8)]),
      cutoff(W3, [poolItem(7), poolItem(8)]), // #7 reopened
      cutoff(W4, [poolItem(7, { state: "closed" }), poolItem(8)]),
    ];
    const weeks = weeklyTransitions(cutoffs);
    expect(weeks[0]).toEqual({ cutoffAt: W2, completedRefs: [`${REPO}#7`], reopenedRefs: [] });
    expect(weeks[1]).toEqual({ cutoffAt: W3, completedRefs: [], reopenedRefs: [`${REPO}#7`] });
    expect(weeks[2]).toEqual({ cutoffAt: W4, completedRefs: [`${REPO}#7`], reopenedRefs: [] });
    expect(weeklyCompletions(cutoffs).map((w) => w.completed)).toEqual([1, 0, 1]);
  });

  test("a zero-completion week is preserved, not dropped", () => {
    const cutoffs = [cutoff(W1, [poolItem(7)]), cutoff(W2, [poolItem(7)]), cutoff(W3, [poolItem(7)])];
    expect(weeklyCompletions(cutoffs)).toEqual([
      { cutoffAt: W2, completed: 0 },
      { cutoffAt: W3, completed: 0 },
    ]);
  });

  test("a missing-source week is excluded, never read as zero", () => {
    const cutoffs = [
      cutoff(W1, [poolItem(7), poolItem(8)]),
      cutoff(W2, [], { complete: false, unavailable: true }),
      cutoff(W3, [poolItem(7, { state: "closed" }), poolItem(8)]),
      cutoff(W4, [poolItem(7, { state: "closed" }), poolItem(8, { state: "closed" })]),
    ];
    // The incomplete cutoff breaks the chain: #7's closure spans the
    // gap, so it is attributed to NO week (never to W3's bucket, and
    // the gap is not read as a zero week either). Buckets resume at
    // the first uninterrupted pair (W3 -> W4), where #8 closes.
    expect(weeklyTransitions(cutoffs)).toEqual([{ cutoffAt: W4, completedRefs: [`${REPO}#8`], reopenedRefs: [] }]);
    expect(weeklyCompletions(cutoffs)).toEqual([{ cutoffAt: W4, completed: 1 }]);
    expect(cutoffs[1]!.coverage.unavailable).toBe(true);
    expect(cutoffs[1]!.items).toEqual([]);
  });

  test("PR transitions never count as completed units", () => {
    const cutoffs = [
      cutoff(W1, [poolItem(7), poolItem(8, { contentKind: "pr" }), poolItem(9, { contentKind: "pr" })]),
      cutoff(W2, [
        poolItem(7, { state: "closed" }),
        poolItem(8, { contentKind: "pr", state: "merged" }),
        poolItem(9, { contentKind: "pr", state: "closed" }),
      ]),
    ];
    // Only the issue is a unit (design SS1): the merged PR and the
    // unmerged-but-closed PR contribute nothing, and an issue plus
    // its PR can never double-count.
    expect(weeklyTransitions(cutoffs)).toEqual([{ cutoffAt: W2, completedRefs: [`${REPO}#7`], reopenedRefs: [] }]);
    expect(cutoffs[0]!.items.find((i) => i.number === 8)!.kind).toBe("pr");
    expect(cutoffs[0]!.items.find((i) => i.number === 7)!.kind).toBe("issue");
  });
});

describe("blockerFirstObserved", () => {
  test("age is a labelled lower bound from first observation", () => {
    const cutoffs = [
      cutoff(W1, [poolItem(7)]),
      cutoff(W2, [poolItem(7, { blockedBy: [`${REPO}#9`] })]),
      cutoff(W4, [poolItem(7, { blockedBy: [`${REPO}#9`] })]),
    ];
    expect(blockerFirstObserved(cutoffs, `${REPO}#7`, `${REPO}#9`)).toEqual({
      firstObservedAt: W2,
      lowerBoundAgeDays: 14,
      lowerBound: true,
    });
    expect(blockerFirstObserved(cutoffs, `${REPO}#7`, `${REPO}#42`)).toBeNull();
  });

  test("a re-added edge starts a new episode at its re-observation", () => {
    const cutoffs = [
      cutoff(W1, [poolItem(7, { blockedBy: [`${REPO}#9`] })]),
      cutoff(W2, [poolItem(7)]), // complete cutoff: edge observed absent
      cutoff(W3, [poolItem(7, { blockedBy: [`${REPO}#9`] })]), // re-added
      cutoff(W4, [poolItem(7, { blockedBy: [`${REPO}#9`] })]),
    ];
    // Age runs from the current episode's first observation (W3),
    // never backdated across the W2 removal: 7 days, not 21.
    expect(blockerFirstObserved(cutoffs, `${REPO}#7`, `${REPO}#9`)).toEqual({
      firstObservedAt: W3,
      lowerBoundAgeDays: 7,
      lowerBound: true,
    });
  });

  test("an edge removed at the last complete cutoff has no current episode", () => {
    const cutoffs = [cutoff(W1, [poolItem(7, { blockedBy: [`${REPO}#9`] })]), cutoff(W2, [poolItem(7)])];
    expect(blockerFirstObserved(cutoffs, `${REPO}#7`, `${REPO}#9`)).toBeNull();
  });
});

describe("ItemHistoryStore", () => {
  let dir: string | null = null;
  afterEach(async () => {
    if (dir) await rm(dir, { recursive: true, force: true });
    dir = null;
  });

  test("append/read round-trips, dedupes a repeated cutoff, skips bad lines", async () => {
    dir = await mkdtemp(join(tmpdir(), "item-history-"));
    const file = join(dir, "item-history.jsonl");
    const store = new ItemHistoryStore(file);
    expect(await store.readAll()).toEqual([]);

    const first = cutoff(W1, [poolItem(7)]);
    expect(await store.append(first)).toBe(true);
    expect(await store.append(first)).toBe(false);
    expect(await store.append(cutoff(W2, [poolItem(7, { state: "closed" })]))).toBe(true);

    const { appendFile } = await import("node:fs/promises");
    await appendFile(file, "not json\n", "utf8");

    const all = await store.readAll();
    expect(all.map((c) => c.cutoffAt)).toEqual([W1, W2]);
    expect(weeklyCompletions(all)).toEqual([{ cutoffAt: W2, completed: 1 }]);
  });
});
