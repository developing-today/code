import { describe, expect, test } from "bun:test";
import {
  applyEpicReparent,
  planEpicReparent,
  setHistoricalParentMetadata,
  selectReparentDestination,
  type ReparentEpic,
  type ReparentGateway,
  type ReparentSelection,
  type ReparentIssue,
} from "../src/epic-reparent.ts";

const issue = (overrides: Partial<ReparentIssue> = {}): ReparentIssue => ({
  ref: "developing-today/code#10",
  parentRef: "developing-today/code#1",
  state: "OPEN",
  labels: ["app/id", "primary-parent/old", "priority/normal"],
  primaryParentChain: ["developing-today/code#1"],
  area: "id",
  ...overrides,
});

const epic = (
  number: number,
  overrides: Partial<ReparentEpic> = {},
): ReparentEpic => ({
  ref: `developing-today/code#${number}`,
  state: "OPEN",
  labels: [`epic/id-work-${number}`],
  primaryParentChain: [],
  area: "other",
  maintenance: false,
  ...overrides,
});

describe("re-parent destination selection", () => {
  test("matches an independent model over generated eligibility and relationship combinations", () => {
    const relations = ["shared_design", "shared_primary_parent", "shared_epic", "shared_area"] as const;
    const child = issue({
      labels: ["design/shared", "epic/shared"],
      primaryParentChain: ["developing-today/code#900"],
      area: "shared",
    });
    const candidatesFor = (number: number, mask: number, open: boolean, maintenance: boolean): ReparentEpic => {
      const labels = [`epic/candidate-${number}`];
      if (mask & 1) labels.push("design/shared");
      if (mask & 4) labels.push("epic/shared");
      return epic(number, {
        state: open ? "OPEN" : "CLOSED",
        labels,
        primaryParentChain: mask & 2 ? ["developing-today/code#900"] : [],
        area: mask & 8 ? "shared" : "other",
        maintenance,
      });
    };
    const options = Array.from({ length: 64 }, (_, index) => ({
      mask: index & 15,
      open: (index & 16) !== 0,
      maintenance: (index & 32) !== 0,
    }));
    const model = (candidates: ReparentEpic[]): ReparentSelection => {
      const eligible = candidates.filter((candidate) => candidate.state === "OPEN" && candidate.ref !== child.ref && candidate.ref !== child.parentRef);
      for (let rank = 0; rank < relations.length; rank += 1) {
        const matches = eligible.filter((candidate) => {
          const index = Number(candidate.ref.split("#").at(-1));
          const option = candidateOptions.get(index)!;
          return (option.mask & (1 << rank)) !== 0;
        });
        if (matches.length === 1) return { kind: "selected", epic: matches[0], reason: relations[rank] };
        if (matches.length > 1) return { kind: "ambiguous", reason: relations[rank], refs: matches.map(({ ref }) => ref).sort() };
      }
      const maintenance = eligible.filter(({ maintenance }) => maintenance);
      if (maintenance.length === 1) return { kind: "selected", epic: maintenance[0], reason: "maintenance_fallback" };
      if (maintenance.length > 1) return { kind: "ambiguous", reason: "shared_area", refs: maintenance.map(({ ref }) => ref).sort() };
      return { kind: "unavailable", reason: "no_open_destination_or_maintenance_epic" };
    };
    const candidateOptions = new Map<number, (typeof options)[number]>();
    for (let left = 0; left < options.length; left += 1) {
      for (let right = 0; right < options.length; right += 1) {
        const leftOption = options[left]!;
        const rightOption = options[right]!;
        const leftNumber = 1000 + left;
        const rightNumber = 2000 + right;
        candidateOptions.set(leftNumber, leftOption);
        candidateOptions.set(rightNumber, rightOption);
        const candidates = [
          candidatesFor(leftNumber, leftOption.mask, leftOption.open, leftOption.maintenance),
          candidatesFor(rightNumber, rightOption.mask, rightOption.open, rightOption.maintenance),
        ];
        expect(selectReparentDestination(child, candidates)).toEqual(model(candidates));
      }
    }
  });

  test("uses the strongest shared relationship, in design/parent/epic/area order", () => {
    const child = issue({ labels: ["design/id-auth", "epic/id-platform"] });
    const area = epic(20, { area: "id" });
    const sharedEpic = epic(21, { labels: ["epic/id-platform"] });
    const sharedDesign = epic(22, { labels: ["design/id-auth"] });
    const result = selectReparentDestination(child, [area, sharedEpic, sharedDesign]);
    expect(result).toEqual({ kind: "selected", epic: sharedDesign, reason: "shared_design" });
  });

  test("returns an ambiguity rather than selecting arbitrarily among equally related epics", () => {
    const child = issue({ labels: ["design/id-auth"] });
    expect(selectReparentDestination(child, [epic(2, { labels: ["design/id-auth"] }), epic(3, { labels: ["design/id-auth"] })])).toMatchObject({
      kind: "ambiguous",
      refs: ["developing-today/code#2", "developing-today/code#3"],
    });
  });

  test("uses the open project maintenance epic only when no related epic exists", () => {
    expect(selectReparentDestination(issue(), [epic(2), epic(3, { maintenance: true })])).toMatchObject({
      kind: "selected",
      epic: { ref: "developing-today/code#3" },
      reason: "maintenance_fallback",
    });
  });

  test("never selects the current parent as the destination", () => {
    const current = epic(1, { labels: ["epic/id-platform", "epic"] });
    expect(selectReparentDestination(issue({ labels: ["epic/id-platform"] }), [current, epic(3, { maintenance: true })])).toMatchObject({
      kind: "selected",
      epic: { ref: "developing-today/code#3" },
      reason: "maintenance_fallback",
    });
  });

  test("plans a parent and primary-parent label change while retaining every other label", () => {
    const result = planEpicReparent(issue(), epic(55, { labels: ["epic/id-platform"] }));
    expect(result).toEqual({
      ref: "developing-today/code#10",
      oldParentRef: "developing-today/code#1",
      newParentRef: "developing-today/code#55",
      expectedLabels: ["app/id", "primary-parent/old", "priority/normal"],
      labelsToApply: ["app/id", "primary-parent/id-platform", "priority/normal"],
    });
  });

  test("refuses a no-op destination", () => {
    expect(() => planEpicReparent(issue(), epic(1))).toThrow("already the child's parent");
  });

  test("re-reads child state, changes the native parent, retains labels, and comments both issues", async () => {
    const calls: string[] = [];
    const gateway: ReparentGateway = {
      readChildLive: async () => issue(),
      isOpenEpicLive: async () => true,
      replaceNativeParent: async (child, oldParent, newParent) => {
        calls.push(`parent:${child}:${oldParent}:${newParent}`);
      },
      removeNativeParent: async (child, parent) => { calls.push(`remove-parent:${child}:${parent}`); },
      replaceLabels: async (child, labels) => {
        calls.push(`labels:${child}:${labels.join(",")}`);
      },
      recordHistoricalParent: async (child, parent) => { calls.push(`history:${child}:${parent}`); },
      removeHistoricalParent: async (child, parent) => { calls.push(`remove-history:${child}:${parent}`); },
      addComment: async (ref) => {
        calls.push(`comment:${ref}`);
      },
    };
    const plan = planEpicReparent(issue(), epic(55, { labels: ["epic/id-platform"] }));
    await applyEpicReparent(plan, "developing-today/code#99", gateway);
    expect(calls).toEqual([
      "parent:developing-today/code#10:developing-today/code#1:developing-today/code#55",
      "labels:developing-today/code#10:app/id,primary-parent/id-platform,priority/normal",
      "history:developing-today/code#10:developing-today/code#1",
      "comment:developing-today/code#10",
      "comment:developing-today/code#99",
    ]);
  });

  test("refuses to mutate if the child's parent changed since the plan was made", async () => {
    const gateway: ReparentGateway = {
      readChildLive: async () => issue({ parentRef: "developing-today/code#8" }),
      isOpenEpicLive: async () => true,
      replaceNativeParent: async () => { throw new Error("must not mutate"); },
      removeNativeParent: async () => { throw new Error("must not mutate"); },
      replaceLabels: async () => { throw new Error("must not mutate"); },
      recordHistoricalParent: async () => { throw new Error("must not mutate"); },
      removeHistoricalParent: async () => { throw new Error("must not mutate"); },
      addComment: async () => { throw new Error("must not mutate"); },
    };
    const plan = planEpicReparent(issue(), epic(55, { labels: ["epic/id-platform"] }));
    await expect(applyEpicReparent(plan, "developing-today/code#99", gateway)).rejects.toThrow("parent changed");
  });

  test("rolls back native parent and labels when label replacement fails", async () => {
    const calls: string[] = [];
    let labelWrites = 0;
    const gateway: ReparentGateway = {
      readChildLive: async () => issue(),
      isOpenEpicLive: async () => true,
      replaceNativeParent: async (_child, oldParent, newParent) => { calls.push(`parent:${oldParent}->${newParent}`); },
      removeNativeParent: async (_child, parent) => { calls.push(`remove-parent:${parent}`); },
      replaceLabels: async (_child, labels) => {
        calls.push(`labels:${labels.includes("primary-parent/old") ? "old" : "new"}`);
        labelWrites += 1;
        if (labelWrites === 1) throw new Error("label update failed");
      },
      recordHistoricalParent: async () => { calls.push("history"); },
      removeHistoricalParent: async () => { calls.push("remove-history"); },
      addComment: async () => { calls.push("comment"); },
    };
    const plan = planEpicReparent(issue(), epic(55, { labels: ["epic/id-platform"] }));
    await expect(applyEpicReparent(plan, "developing-today/code#99", gateway)).rejects.toThrow("label update failed");
    expect(calls).toEqual([
      "parent:developing-today/code#1->developing-today/code#55",
      "labels:new",
      "remove-history",
      "labels:old",
      "parent:developing-today/code#55->developing-today/code#1",
    ]);
  });

  test("writes and removes the former parent in canonical metadata without losing other links", () => {
    const original = "Issue text\n\n<!-- pm-metadata:start -->\n```json\n{\"pm_meta\":1,\"links\":[{\"relation\":\"blockedBy\",\"target\":{\"repo\":\"x/y\",\"number\":7}}]}\n```\n<!-- pm-metadata:end -->";
    const parent = "developing-today/code#12" as const;
    const withHistory = setHistoricalParentMetadata(original, parent, true, "2026-11-22");
    expect(withHistory).toContain('"status": "historical"');
    expect(withHistory).toContain('"reason": "reparent-on-close"');
    expect(withHistory).toContain('"until": "2026-11-22"');
    expect(withHistory).toContain('"number": 7');
    expect(setHistoricalParentMetadata(withHistory, parent, false)).not.toContain('"status": "historical"');
  });

  test("removes a new native edge and restores labels when a parentless child's label update fails", async () => {
    const calls: string[] = [];
    let labelWrites = 0;
    const child = issue({ parentRef: null });
    const gateway: ReparentGateway = {
      readChildLive: async () => child,
      isOpenEpicLive: async () => true,
      replaceNativeParent: async (_child, oldParent, newParent) => { calls.push(`parent:${oldParent}->${newParent}`); },
      removeNativeParent: async (_child, parent) => { calls.push(`remove-parent:${parent}`); },
      replaceLabels: async (_child, labels) => {
        calls.push(`labels:${labels.includes("primary-parent/id-platform") ? "new" : "old"}`);
        labelWrites += 1;
        if (labelWrites === 1) throw new Error("label update failed");
      },
      recordHistoricalParent: async () => { calls.push("history"); },
      removeHistoricalParent: async () => { calls.push("remove-history"); },
      addComment: async () => { calls.push("comment"); },
    };
    const plan = planEpicReparent(child, epic(55, { labels: ["epic/id-platform"] }));
    await expect(applyEpicReparent(plan, "developing-today/code#99", gateway)).rejects.toThrow("label update failed");
    expect(calls).toEqual([
      "parent:null->developing-today/code#55",
      "labels:new",
      "labels:old",
      "remove-parent:developing-today/code#55",
    ]);
  });
});
