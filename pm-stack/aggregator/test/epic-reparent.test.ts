import { describe, expect, test } from "bun:test";
import {
  applyEpicReparent,
  planEpicReparent,
  selectReparentDestination,
  type ReparentEpic,
  type ReparentGateway,
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
      readChild: async () => issue(),
      isOpenEpic: async () => true,
      replaceNativeParent: async (child, oldParent, newParent) => {
        calls.push(`parent:${child}:${oldParent}:${newParent}`);
      },
      replaceLabels: async (child, labels) => {
        calls.push(`labels:${child}:${labels.join(",")}`);
      },
      addComment: async (ref) => {
        calls.push(`comment:${ref}`);
      },
    };
    const plan = planEpicReparent(issue(), epic(55, { labels: ["epic/id-platform"] }));
    await applyEpicReparent(plan, "developing-today/code#99", gateway);
    expect(calls).toEqual([
      "parent:developing-today/code#10:developing-today/code#1:developing-today/code#55",
      "labels:developing-today/code#10:app/id,primary-parent/id-platform,priority/normal",
      "comment:developing-today/code#10",
      "comment:developing-today/code#99",
    ]);
  });

  test("refuses to mutate if the child's parent changed since the plan was made", async () => {
    const gateway: ReparentGateway = {
      readChild: async () => issue({ parentRef: "developing-today/code#8" }),
      isOpenEpic: async () => true,
      replaceNativeParent: async () => { throw new Error("must not mutate"); },
      replaceLabels: async () => { throw new Error("must not mutate"); },
      addComment: async () => { throw new Error("must not mutate"); },
    };
    const plan = planEpicReparent(issue(), epic(55, { labels: ["epic/id-platform"] }));
    await expect(applyEpicReparent(plan, "developing-today/code#99", gateway)).rejects.toThrow("parent changed");
  });
});
