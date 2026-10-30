import { describe, expect, test } from "bun:test";
import type { GhItem, ProjectPoolItem, Provenance, RefKey } from "../src/model.ts";
import { blockedSummary, depthLayout, hasGraphEdges } from "../src/graph.ts";
import { buildProjectDetail, projectConfigById } from "../src/project-config.ts";
import { renderProgramProject } from "../src/render.ts";

const R = (n: number): RefKey => `developing-today/code#${n}`;

function item(partial: Partial<ProjectPoolItem> & { number: number }): ProjectPoolItem {
  return {
    id: `PVTI_${partial.number}`,
    url: "https://github.com/orgs/developing-today/projects/4?pane=issue&itemId=1",
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: R(partial.number),
    repo: "developing-today/code",
    issueUrl: `https://github.com/developing-today/code/issues/${partial.number}`,
    title: `item ${partial.number}`,
    state: "open",
    contentKind: "issue",
    status: null,
    area: "pm-dashboard",
    horizon: null,
    kind: null,
    ...partial,
  };
}

describe("hasGraphEdges", () => {
  test("false for a pool with no edges at all", () => {
    expect(hasGraphEdges([item({ number: 1 }), item({ number: 2 })])).toBe(false);
  });
  test("true when any item carries any edge kind", () => {
    expect(hasGraphEdges([item({ number: 1, parentRef: R(9) })])).toBe(true);
    expect(hasGraphEdges([item({ number: 1, blockedBy: [R(9)] })])).toBe(true);
    expect(hasGraphEdges([item({ number: 1, epicRef: R(9) })])).toBe(true);
  });
});

describe("blockedSummary", () => {
  test("sums sized blocked weight, counts unsized separately, finds the longest chain", () => {
    const s = blockedSummary([
      item({ number: 1, labels: ["effort/m"], blockedBy: [R(2)] }),
      item({ number: 2, labels: ["effort/s"], blockedBy: [R(3)] }),
      item({ number: 3, labels: ["effort/xs"] }),
      item({ number: 4, blockedBy: [R(3)] }), // unsized + blocked
      item({ number: 5, labels: ["effort/l"] }), // not blocked
    ]);
    expect(s.blockedCount).toBe(3);
    expect(s.blockedWeight).toBe(12 + 4);
    expect(s.blockedUnsizedCount).toBe(1);
    expect(s.longestChain).toEqual([R(1), R(2), R(3)]);
  });

  test("a blocker the pool holds as closed resolves the block", () => {
    const s = blockedSummary([
      item({ number: 1, labels: ["effort/m"], blockedBy: [R(2)] }),
      item({ number: 2, state: "closed" }),
    ]);
    expect(s.blockedCount).toBe(0);
    expect(s.longestChain).toEqual([]);
  });

  test("an out-of-pool blocker is unresolved, never assumed done", () => {
    const s = blockedSummary([item({ number: 1, labels: ["effort/s"], blockedBy: [R(99)] })]);
    expect(s.blockedCount).toBe(1);
    expect(s.longestChain).toEqual([R(1), R(99)]);
  });

  test("a blocked-by cycle terminates instead of looping", () => {
    const s = blockedSummary([
      item({ number: 1, blockedBy: [R(2)] }),
      item({ number: 2, blockedBy: [R(1)] }),
    ]);
    expect(s.blockedCount).toBe(2);
    expect(new Set(s.longestChain).size).toBe(s.longestChain.length);
  });

  test("shared blocker tails are memoized, not re-enumerated per path", () => {
    // Codex repro on PR #2831: each item blocked by the next two makes
    // naive DFS enumerate exponentially many paths (~7s at 32 items).
    // With suffix memoization this is instant and the longest chain
    // still walks every item.
    const items = Array.from({ length: 32 }, (_, i) => {
      const n = i + 1;
      const blockedBy = [R(n + 1), R(n + 2)].filter((_, j) => n + j + 1 <= 32);
      return item({ number: n, blockedBy });
    });
    const s = blockedSummary(items);
    expect(s.longestChain.length).toBe(32);
    expect(s.longestChain[0]).toBe(R(1));
    expect(s.longestChain[31]).toBe(R(32));
  });

  test("closed items are never counted as blocked", () => {
    const s = blockedSummary([item({ number: 1, state: "closed", blockedBy: [R(2)] }), item({ number: 2 })]);
    expect(s.blockedCount).toBe(0);
  });
});

describe("depthLayout", () => {
  test("depth follows the parent chain; epics and modes are carried", () => {
    const layout = depthLayout([
      item({ number: 1, kind: "Epic" }),
      item({ number: 2, parentRef: R(1), labels: ["mode/physical"] }),
      item({ number: 3, parentRef: R(2) }),
      item({ number: 4 }),
    ]);
    const byRef = new Map(layout.nodes.map((n) => [n.ref, n]));
    expect(byRef.get(R(1))?.depth).toBe(0);
    expect(byRef.get(R(2))?.depth).toBe(1);
    expect(byRef.get(R(3))?.depth).toBe(2);
    expect(byRef.get(R(1))?.epic).toBe(true);
    expect(byRef.get(R(2))?.mode).toBe("physical");
    expect(layout.maxDepth).toBe(2);
    expect(layout.edges).toEqual([
      { from: R(1), to: R(2) },
      { from: R(2), to: R(3) },
    ]);
    expect(layout.truncated).toBe(0);
  });

  test("a parent outside the pool makes the item a root", () => {
    const layout = depthLayout([item({ number: 1, parentRef: R(99) })]);
    expect(layout.nodes[0]?.depth).toBe(0);
    expect(layout.edges).toEqual([]);
  });

  test("the cap truncates and says so", () => {
    const items = Array.from({ length: 10 }, (_, i) => item({ number: i + 1 }));
    const layout = depthLayout(items, 4);
    expect(layout.nodes.length).toBe(4);
    expect(layout.truncated).toBe(6);
  });
});

const PROV: Provenance = {
  board: "x",
  roadmap: "x",
  items: "x",
  comments: "x",
  quota: "x",
  usage: "x",
  attention: "x",
  orchestration: "x",
  dns: "x",
  registry: "x",
  aa: "x",
};

function renderWith(pool: ProjectPoolItem[]): string {
  const config = projectConfigById("pm-dashboard")!;
  const items = new Map<RefKey, GhItem>();
  return renderProgramProject(buildProjectDetail(config, items, pool, []), items, PROV);
}

describe("renderProgramProject graph section (#2400)", () => {
  test("a sized + graphed project renders blocked weight, the chain, and the SVG", () => {
    const html = renderWith([
      item({ number: 1, kind: "Epic", labels: ["effort/l"] }),
      item({ number: 2, parentRef: R(1), labels: ["effort/m"], blockedBy: [R(3)] }),
      item({ number: 3, labels: ["effort/s"] }),
    ]);
    expect(html).toContain("Blocked weight: 12");
    expect(html).toContain("longest blocked chain");
    expect(html).toContain("<svg");
    expect(html).toContain("Depth = sub-issue nesting");
    expect(html).not.toContain("Not yet sized/graphed");
  });

  test("an unsized, edgeless project renders the explicit note and no fake percentage", () => {
    const html = renderWith([item({ number: 1 }), item({ number: 2 })]);
    expect(html).toContain("Not yet sized/graphed");
    expect(html).toContain("not a 0% weighted score");
    expect(html).not.toContain("<svg");
  });

  test("a sized but edgeless project says not-yet-graphed, not blocked-zero", () => {
    const html = renderWith([item({ number: 1, labels: ["effort/m"] })]);
    expect(html).toContain("Not yet graphed");
    expect(html).not.toContain("Blocked weight");
  });
});
