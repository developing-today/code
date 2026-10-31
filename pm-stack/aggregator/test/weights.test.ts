import { describe, expect, test } from "bun:test";
import { effortWeight, handsLine, modeOf, summarizeWeights, weightedLine } from "../src/weights.ts";

type Item = { state: "open" | "closed" | "merged"; labels?: string[] };
const item = (state: Item["state"], labels?: string[]): Item => ({ state, labels });

describe("effortWeight (#1108 slice 1)", () => {
  test("every effort label maps to its spec weight", () => {
    expect(effortWeight(["effort/xs"])).toBe(1);
    expect(effortWeight(["effort/s"])).toBe(4);
    expect(effortWeight(["effort/m"])).toBe(12);
    expect(effortWeight(["effort/l"])).toBe(40);
  });

  test("finds the effort label among others, case-insensitively", () => {
    expect(effortWeight(["kind/task", "Effort/M", "mode/online"])).toBe(12);
  });

  test("unsized: no labels, no effort label, or an unknown size", () => {
    expect(effortWeight(undefined)).toBeNull();
    expect(effortWeight([])).toBeNull();
    expect(effortWeight(["kind/task"])).toBeNull();
    expect(effortWeight(["effort/xl"])).toBeNull();
  });

  test("unsized: prototype-chain members are not weights (#2734)", () => {
    expect(effortWeight(["effort/constructor"])).toBeNull();
    expect(effortWeight(["effort/__proto__"])).toBeNull();
    expect(effortWeight(["effort/hasOwnProperty"])).toBeNull();
    expect(effortWeight(["effort/toString"])).toBeNull();
  });

  test("prototype-labelled items count as unsized with numeric totals (#2734)", () => {
    const s = summarizeWeights([
      item("closed", ["effort/constructor"]),
      item("open", ["effort/__proto__", "kind/task"]),
      item("open", ["effort/xs"]), // 1 open, the only sized item
    ]);
    expect(s.totalWeight).toBe(1);
    expect(s.closedWeight).toBe(0);
    expect(s.openWeight).toBe(1);
    expect(typeof s.totalWeight).toBe("number");
    expect(typeof s.closedWeight).toBe("number");
    expect(s.unsizedCount).toBe(2);
    expect(s.unsizedClosedCount).toBe(1);
    expect(s.unsizedOpenCount).toBe(1);
    expect(s.weightedPct).toBe(0);
    const line = weightedLine(s);
    expect(line).not.toContain("function");
    expect(line).not.toContain("[object Object]");
    expect(line).toContain("2 unsized");
  });
});

describe("modeOf", () => {
  test("reads mode/* labels", () => {
    expect(modeOf(["mode/physical"])).toBe("physical");
    expect(modeOf(["mode/hybrid"])).toBe("hybrid");
    expect(modeOf(["mode/online"])).toBe("online");
    expect(modeOf(["effort/s"])).toBeNull();
    expect(modeOf(undefined)).toBeNull();
  });
});

describe("summarizeWeights", () => {
  test("closed/open split by weight, counts second", () => {
    const s = summarizeWeights([
      item("closed", ["effort/l"]), // 40 closed
      item("merged", ["effort/s"]), // 4 closed (merged counts as closed)
      item("open", ["effort/m"]), // 12 open
      item("open", ["effort/xs"]), // 1 open
    ]);
    expect(s.totalWeight).toBe(57);
    expect(s.closedWeight).toBe(44);
    expect(s.openWeight).toBe(13);
    expect(s.weightedPct).toBe(77.2);
    expect(s.closedCount).toBe(2);
    expect(s.openCount).toBe(2);
    expect(s.unsizedCount).toBe(0);
  });

  test("unsized items are counted separately, never zero-weighted silently", () => {
    const s = summarizeWeights([
      item("closed", ["effort/s"]),
      item("closed", []),
      item("open", ["kind/task"]),
    ]);
    expect(s.totalWeight).toBe(4);
    expect(s.weightedPct).toBe(100);
    expect(s.unsizedCount).toBe(2);
    expect(s.unsizedClosedCount).toBe(1);
    expect(s.unsizedOpenCount).toBe(1);
    expect(weightedLine(s)).toContain("2 unsized");
  });

  test("nothing sized: pct is null, not a fake 0 or 100", () => {
    const s = summarizeWeights([item("open", []), item("closed", [])]);
    expect(s.weightedPct).toBeNull();
    expect(weightedLine(s)).toContain("nothing sized");
  });

  test("empty set", () => {
    const s = summarizeWeights([]);
    expect(s.count).toBe(0);
    expect(s.totalWeight).toBe(0);
    expect(s.weightedPct).toBeNull();
  });

  test("hands queue: open physical + hybrid weight vs online", () => {
    const s = summarizeWeights([
      item("open", ["effort/l", "mode/physical"]), // 40 hands
      item("open", ["effort/s", "mode/hybrid"]), // 4 hands
      item("open", ["effort/m", "mode/online"]), // 12 online
      item("open", ["effort/xs"]), // 1 no mode
      item("closed", ["effort/l", "mode/physical"]), // closed: not in the queue
    ]);
    expect(s.handsWeight).toBe(44);
    expect(s.onlineWeight).toBe(12);
    expect(s.unmodedOpenWeight).toBe(1);
    expect(handsLine(s)).toContain("44 weight waiting on hands");
  });
});
