import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { epicGraph, epicSelector } from "../src/epic-graph.ts";
import type { OrchestrationSection, ProjectPoolItem, RefKey, TierItem } from "../src/model.ts";
import type { DashAction } from "../src/actions.ts";
import { STANDING_IDENTITIES } from "../src/registry.ts";

const R = (n: number): RefKey => `developing-today/code#${n}` as RefKey;

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

function fixturePool(): ProjectPoolItem[] {
  return [
    item({ number: 1, title: "Epic one", kind: "Epic" }),
    item({ number: 2, epicRef: R(1), parentRef: R(1), blockedBy: [R(99)] }),
    item({ number: 3, epicRef: R(1), parentRef: R(2), blockedBy: [R(2)] }),
    item({ number: 4, epicRef: R(1), state: "closed" }),
    item({ number: 5, title: "Other epic", kind: "Epic" }),
    item({ number: 6, epicRef: R(5), blockedBy: [R(2)] }),
    item({ number: 7, title: "Epic without children", kind: "Epic" }),
    item({ number: 8 }),
  ];
}

describe("epicSelector", () => {
  test("lists eligible epics with item and blocked counts", () => {
    const entries = epicSelector(fixturePool());
    expect(entries.map((e) => e.ref)).toEqual([R(1), R(5)]);
    const one = entries[0]!;
    expect(one.itemCount).toBe(3);
    expect(one.blockedCount).toBe(2); // #2 waits on out-of-pool #99; #3 waits on open #2
    expect(one.doneCount).toBe(1); // #4 closed
    expect(entries[1]!.itemCount).toBe(1);
    expect(entries[1]!.blockedCount).toBe(1); // #6 waits on open #2
  });

  test("an epic with no member items is not eligible", () => {
    const entries = epicSelector(fixturePool());
    expect(entries.some((e) => e.ref === R(7))).toBe(false);
  });

  test("no epics in the pool yields an empty selector", () => {
    expect(epicSelector([item({ number: 1 }), item({ number: 2 })])).toEqual([]);
  });

  test("invalid-only blockers do not inflate selector or graph blocked counts", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), blockedBy: ["812"] }),
      item({ number: 3, epicRef: R(1), blockedBy: [R(99)] }),
    ];
    expect(epicSelector(pool)[0]?.blockedCount).toBe(1);
    const graph = epicGraph(pool, R(1));
    expect(graph.blockedCount).toBe(1);
    expect(graph.rankedBlocked.map((row) => row.ref)).toEqual([R(3)]);
  });
});

describe("epicGraph", () => {
  test("unknown or non-epic ref is unavailable, never an empty success", () => {
    const missing = epicGraph(fixturePool(), R(42));
    expect(missing.status).toBe("unavailable");
    expect(missing.reason).toContain("not in this pool");
    const notEpic = epicGraph(fixturePool(), R(8));
    expect(notEpic.status).toBe("unavailable");
  });

  test("an epic with no members is unavailable with the reason stated", () => {
    const g = epicGraph(fixturePool(), R(7));
    expect(g.status).toBe("unavailable");
    expect(g.reason).toContain("not yet graphed");
  });

  test("selection yields exactly the epic pool plus one-hop refs", () => {
    const g = epicGraph(fixturePool(), R(1));
    expect(g.status).toBe("ok");
    expect(g.memberCount).toBe(3);
    const refs = g.nodes.map((n) => n.ref).sort();
    // epic #1, members #2 #3 #4, one-hop pool ref #5? no — #6 belongs
    // to epic #5 and blocks nothing here; #2's blocker #99 is external.
    expect(refs).toEqual([R(1), R(2), R(3), R(4), R(99)].sort());
    expect(g.totalNodes).toBe(5);
    expect(g.truncated).toBe(0);
  });

  test("one-hop pool refs from another epic are included as pool nodes", () => {
    const g = epicGraph(fixturePool(), R(5));
    const two = g.nodes.find((n) => n.ref === R(2));
    expect(two).toBeDefined();
    expect(two!.inPool).toBe(true);
    expect(two!.member).toBe(false);
    expect(two!.unresolved).toBe(false);
  });

  test("out-of-pool refs are unresolved external nodes with no fabricated title or state", () => {
    const g = epicGraph(fixturePool(), R(1));
    const ext = g.nodes.find((n) => n.ref === R(99));
    expect(ext).toBeDefined();
    expect(ext!.inPool).toBe(false);
    expect(ext!.unresolved).toBe(true);
    expect(ext!.title).toBeNull();
    expect(ext!.state).toBeNull();
    expect(ext!.issueUrl).toBe("https://github.com/developing-today/code/issues/99");
    expect(g.externalCount).toBe(1);
  });

  test("dependency edges are distinct from hierarchy edges", () => {
    const g = epicGraph(fixturePool(), R(1));
    const kinds = g.edges.map((e) => `${e.type}:${e.from}->${e.to}`).sort();
    expect(kinds).toContain(`hierarchy:${R(1)}->${R(2)}`);
    expect(kinds).toContain(`hierarchy:${R(2)}->${R(3)}`);
    expect(kinds).toContain(`dependency:${R(2)}->${R(99)}`);
    expect(kinds).toContain(`dependency:${R(3)}->${R(2)}`);
  });

  test("blocks[] lands as a dependency edge in item-to-blocker orientation", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), blocks: [R(3)] }),
      item({ number: 3, epicRef: R(1) }),
    ];
    const g = epicGraph(pool, R(1));
    expect(g.edges).toContainEqual({ from: R(3), to: R(2), type: "dependency" });
  });

  test("malformed one-hop refs are omitted while valid blockedBy and blocks edges render", () => {
    const unsafe = "developing-today/code#99999999999999999999";
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), blockedBy: ["812", R(3)], blocks: [unsafe, R(4)] }),
      item({ number: 3 }),
      item({ number: 4 }),
    ];
    const g = epicGraph(pool, R(1));
    expect(g.nodes.map((n) => n.ref)).not.toContain("812");
    expect(g.nodes.map((n) => n.ref)).not.toContain(unsafe);
    expect(g.edges).toContainEqual({ from: R(2), to: R(3), type: "dependency" });
    expect(g.edges).toContainEqual({ from: R(4), to: R(2), type: "dependency" });
    expect(g.edgeDiagnostics).toContainEqual({ sourceRef: R(2), relation: "blockedBy", state: "malformed", target: "812" });
    expect(g.edgeDiagnostics).toContainEqual({ sourceRef: R(2), relation: "blocks", state: "malformed", target: unsafe });
    expect(g.edgeDiagnostics.some((d) => d.target === R(99))).toBe(false);
    expect(g.rankedBlocked.find((r) => r.ref === R(2))?.blockers).toEqual([R(3)]);

    const html = renderWith(pool);
    expect(html).toContain('href="https://github.com/developing-today/code/issues/3"');
    expect(html).toContain('href="https://github.com/developing-today/code/issues/4"');
    expect(html).not.toContain('href="https://github.com/812/issues/"');
    expect(html).not.toContain('href="https://github.com/developing-today/code/issues/99999999999999999999"');
  });

  test("truncation reports the true total and the cut count", () => {
    const g = epicGraph(fixturePool(), R(1), 3);
    expect(g.totalNodes).toBe(5);
    expect(g.nodes.length).toBe(3);
    expect(g.truncated).toBe(2);
    // Edges never point at unshown nodes.
    const shown = new Set(g.nodes.map((n) => n.ref));
    for (const e of g.edges) {
      expect(shown.has(e.from)).toBe(true);
      expect(shown.has(e.to)).toBe(true);
    }
  });

  test("blocked summaries are computed before the node cap", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1) }),
      item({ number: 3, epicRef: R(1) }),
      item({ number: 4, epicRef: R(1) }),
      item({ number: 5, epicRef: R(1), blockedBy: [R(99)] }),
    ];
    const g = epicGraph(pool, R(1), 3);
    expect(g.nodes.map((n) => n.ref)).not.toContain(R(5));
    expect(g.blockedCount).toBe(1); // #5 is blocked even though the cap hides it
    expect(g.rankedBlocked.map((r) => r.ref)).toEqual([R(5)]);
    expect(g.rankedBlocked[0]!.blockers).toEqual([R(99)]);
    expect(g.rankedBlocked[0]!.downstreamCount).toBeGreaterThanOrEqual(0);
    expect(Array.isArray(g.rankedBlocked[0]!.contributingItems)).toBe(true);
  });

  test("external count is computed before the node cap (Codex round-3)", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1) }),
      item({ number: 3, epicRef: R(1) }),
      item({ number: 4, epicRef: R(1), blockedBy: [R(99)] }),
    ];
    const g = epicGraph(pool, R(1), 3);
    // External refs sort last in the node order, so the cap hides the
    // only external node; the summary count must still report it.
    expect(g.nodes.map((n) => n.ref)).not.toContain(R(99));
    expect(g.externalCount).toBe(1);
  });

  test("external nodes keep the dependency links observed for them", () => {
    const g = epicGraph(fixturePool(), R(1));
    const ext = g.nodes.find((n) => n.ref === R(99))!;
    expect(ext.dependents).toEqual([R(2)]); // #2 is blocked by #99, so #99 blocks #2
    expect(ext.blockers).toEqual([]);
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), blocks: [R(98)] }),
    ];
    const g2 = epicGraph(pool, R(1));
    const ext2 = g2.nodes.find((n) => n.ref === R(98))!;
    expect(ext2.blockers).toEqual([R(2)]); // #2 blocks #98, so #98 is blocked by #2
    expect(ext2.dependents).toEqual([]);
  });

  test("a parent cycle degrades to roots instead of looping", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), parentRef: R(3) }),
      item({ number: 3, epicRef: R(1), parentRef: R(2) }),
    ];
    const g = epicGraph(pool, R(1));
    expect(g.status).toBe("ok");
    expect(g.nodes.length).toBe(3);
    expect(new Set(g.nodes.map((n) => n.row)).size).toBe(3);
    expect(g.cycleGroups).toEqual([{ kind: "hierarchy", name: "Hierarchy cycle 1 (2 items)", members: [R(2), R(3)] }]);
    expect(g.nodes.find((n) => n.ref === R(2))?.depth).toBe(g.nodes.find((n) => n.ref === R(3))?.depth);
  });

  test("dependency SCCs are named, stable, and not credited as positive unblock work", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), blockedBy: [R(3)] }),
      item({ number: 3, epicRef: R(1), blockedBy: [R(2)] }),
      item({ number: 4, epicRef: R(1), blockedBy: [R(2)] }),
    ];
    const graph = epicGraph(pool, R(1));
    expect(graph.cycleGroups).toEqual([{ kind: "dependency", name: "Dependency cycle 1 (2 items)", members: [R(2), R(3)] }]);
    expect(graph.rankedBlocked.some((row) => row.ref === R(2) || row.ref === R(3))).toBe(false);
    expect(graph.rankedBlocked.find((row) => row.ref === R(4))?.downstreamCount).toBe(0);
  });

  test("full-pool dependency cycles that affect this epic are warned even beyond one hop", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), blockedBy: [R(3)] }),
      item({ number: 3, blockedBy: [R(4)] }),
      item({ number: 4, blockedBy: [R(2)] }),
    ];
    const graph = epicGraph(pool, R(1));
    expect(graph.nodes.map((node) => node.ref)).not.toContain(R(4));
    expect(graph.cycleGroups).toEqual([{ kind: "dependency", name: "Dependency cycle 1 (3 items)", members: [R(2), R(3), R(4)] }]);
    expect(graph.rankedBlocked.some((row) => row.ref === R(2))).toBe(false);
  });

  test("full-pool hierarchy cycles stay grouped when a member is beyond one hop", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), parentRef: R(3) }),
      item({ number: 3, parentRef: R(4) }),
      item({ number: 4, parentRef: R(2) }),
    ];
    const graph = epicGraph(pool, R(1));
    expect(graph.nodes.map((node) => node.ref)).not.toContain(R(4));
    expect(graph.cycleGroups).toContainEqual({
      kind: "hierarchy",
      name: "Hierarchy cycle 1 (3 items)",
      members: [R(2), R(3), R(4)],
    });
    expect(graph.nodes.find((node) => node.ref === R(2))?.depth).toBe(graph.nodes.find((node) => node.ref === R(3))?.depth);
  });

  test("hierarchy cycles farther up a member's parent chain still produce warning groups", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), parentRef: R(3) }),
      item({ number: 3, parentRef: R(4) }),
      item({ number: 4, parentRef: R(5) }),
      item({ number: 5, parentRef: R(4) }),
    ];
    const graph = epicGraph(pool, R(1));
    expect(graph.nodes.map((node) => node.ref)).not.toContain(R(4));
    expect(graph.nodes.map((node) => node.ref)).not.toContain(R(5));
    expect(graph.cycleGroups).toContainEqual({
      kind: "hierarchy",
      name: "Hierarchy cycle 1 (2 items)",
      members: [R(4), R(5)],
    });
  });

  test("an acyclic dependency graph has no cycle warning groups", () => {
    const graph = epicGraph(fixturePool(), R(1));
    expect(graph.cycleGroups).toEqual([]);
  });

  test("unavailable edge diagnostics stay distinct from valid external refs", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({
        number: 2,
        epicRef: R(1),
        blockedBy: [R(99)],
        edgeDiagnostics: [{ relation: "blockedBy", state: "unavailable", target: null }],
      }),
    ];
    const graph = epicGraph(pool, R(1));
    expect(graph.edgeDiagnostics).toEqual([
      { sourceRef: R(2), relation: "blockedBy", state: "unavailable", target: null },
    ]);
    expect(graph.nodes.find((node) => node.ref === R(99))).toMatchObject({ unresolved: true, title: null, state: null });
    expect(graph.externalCount).toBe(1);
  });

  test("members with no edges still return nodes (absence is data, not an error)", () => {
    const pool = [item({ number: 1, title: "Epic", kind: "Epic" }), item({ number: 2, epicRef: R(1) })];
    const g = epicGraph(pool, R(1));
    expect(g.status).toBe("ok");
    expect(g.nodes.length).toBe(2);
    expect(g.edges).toEqual([]);
    expect(g.blockedCount).toBe(0);
  });
});

import type { GhItem, Provenance } from "../src/model.ts";
import { buildProjectDetail, projectConfigById } from "../src/project-config.ts";
import { renderProgramProject } from "../src/render.ts";

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

function renderWith(pool: ProjectPoolItem[], graphActions?: Parameters<typeof renderProgramProject>[5]): string {
  const config = projectConfigById("pm-dashboard")!;
  const items = new Map<RefKey, GhItem>();
  return renderProgramProject(buildProjectDetail(config, items, pool, []), items, PROV, undefined, pool, graphActions);
}

describe("renderProgramProject per-epic dependency graph (#3973)", () => {
  test("selector lists the epic with counts and an anchor to its section", () => {
    const html = renderWith(fixturePool());
    expect(html).toContain("Per-epic dependency graph");
    expect(html).toContain('href="#epic-graph-developing-today-code-1"');
    expect(html).toContain('id="epic-graph-developing-today-code-1"');
    expect(html).toContain("3 items · 2 blocked · 1 done");
  });

  test("dependency edges render dashed and hierarchy edges solid, with a text legend", () => {
    const html = renderWith(fixturePool());
    expect(html).toContain('stroke-dasharray="5 3"');
    expect(html).toContain("dashed amber lines are dependency edges");
    expect(html).toContain("Solid grey lines are sub-issue hierarchy");
  });

  test("cycle groups render member links and do not promise one member clears a cycle", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), blockedBy: [R(3)] }),
      item({ number: 3, epicRef: R(1), blockedBy: [R(2)] }),
    ];
    const html = renderWith(pool);
    expect(html).toContain("Dependency cycle 1 (2 items)");
    expect(html).toContain("Resolving one member alone does not clear this cycle");
    expect(html).toContain('href="https://github.com/developing-today/code/issues/2"');
    expect(html).toContain('href="https://github.com/developing-today/code/issues/3"');
  });

  test("hierarchy cycles render as warning groups rather than false roots", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), parentRef: R(3) }),
      item({ number: 3, epicRef: R(1), parentRef: R(2) }),
    ];
    const html = renderWith(pool);
    expect(html).toContain("Hierarchy cycle 1 (2 items)");
    expect(html).toContain("not independent roots");
    expect(html).toContain('href="https://github.com/developing-today/code/issues/2"');
    expect(html).toContain('href="https://github.com/developing-today/code/issues/3"');
  });

  test("only published graph items get audited triage actions and existing feedback", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1) }),
      item({ number: 3, epicRef: R(1) }),
    ];
    const proposalAt = new Date().toISOString();
    const published: TierItem = {
      id: "graph-task-2",
      kind: "task",
      title: "Published graph item",
      source: "tracked",
      reason: "Needs a decision",
      since: "2026-11-08T00:00:00Z",
      lastSeenAt: "2026-11-08T00:00:00Z",
      ref: R(2),
      whyUser: "decision",
      delegation: {
        mode: "first-pass",
        status: "proposal-ready",
        updatedAt: proposalAt,
        runId: "proposal-run",
        proposal: {
          summary: "Published proposal summary",
          detail: "Published proposal detail",
          proposedChanges: ["Published proposal change"],
          at: proposalAt,
        },
      },
    };
    const orchestration: OrchestrationSection = {
      version: 1,
      generatedAt: "2026-11-08T00:00:00Z",
      runId: "orch-test",
      provenance: { sources: ["test"] },
      tiers: { needsDrew: [published], needsOrchestrator: [] },
      dependencies: [],
      goals: [],
      actions: [],
    };
    const action = (over: Partial<DashAction>): DashAction => ({
      id: "action-1",
      clientToken: "token-1",
      kind: "triage",
      verb: "acknowledge",
      objectRef: `gh:${R(2)}`,
      payload: {},
      author: { kind: "drew", id: "drew", label: "Drew" },
      createdAt: "2026-11-08T00:00:00Z",
      state: "new",
      ...over,
    });
    const html = renderWith(pool, {
      orchestration,
      orchestrationState: "live",
      identities: STANDING_IDENTITIES,
      actionsByObject: new Map([
        [
          `gh:${R(2)}`,
          [
            action({}),
            action({ id: "action-2", clientToken: "token-2", verb: "snooze", state: "rejected", resultNote: "Consumer declined" }),
          ],
        ],
      ]),
    });
    expect(html).toContain('data-kind="triage"');
    expect(html).toContain('data-object="gh:developing-today/code#2"');
    expect(html).toContain('data-verb="acknowledge"');
    expect(html).toContain("Acknowledge sent · pending");
    expect(html).toContain("Consumer declined");
    expect(html).toContain("Published proposal summary");
    expect(html).toContain("Published proposal detail");
    expect(html).toContain("Published proposal change");
    expect(html).toContain('data-verb="approve-proposal"');
    expect(html).toContain('data-verb="reject-proposal"');
    expect(html.indexOf('class="propbox"')).toBeLessThan(html.indexOf('data-verb="approve-proposal"'));
    expect(html).toContain('fetch("/api/actions"');
    expect(html).toContain("No inline action: this item is not in the currently published Needs User or Needs Orchestrator tier.");
    expect(html).not.toContain('data-object="gh:developing-today/code#3"');
  });

  test("malformed and unavailable relation data is explained separately from external refs", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({
        number: 2,
        epicRef: R(1),
        blockedBy: ["not-a-ref", R(99)],
        edgeDiagnostics: [{ relation: "blocks", state: "unavailable", target: null }],
      }),
    ];
    const html = renderWith(pool);
    expect(html).toContain("Malformed relation reference");
    expect(html).toContain("Unavailable edge data");
    expect(html).toContain("not in this pool / status unknown — unresolved");
    expect(html).not.toContain("Malformed relation reference: <a href=\"https://github.com/developing-today/code/issues/99\"");
  });

  test("the vertical list names blockers and states as text, external refs as unknown", () => {
    const html = renderWith(fixturePool());
    expect(html).toContain("not in this pool / status unknown — unresolved");
    expect(html).toContain("(unresolved)");
    expect(html).toContain("blocked by:");
    expect(html).toContain("waiting on");
    expect(html).toContain("distinct downstream open epic item");
  });

  test("a pool with no eligible epic says unavailable, not an empty graph", () => {
    const html = renderWith([item({ number: 1 }), item({ number: 2, parentRef: R(1) })]);
    expect(html).toContain("Dependency graph unavailable: no epic in this pool has items yet");
    expect(html).not.toContain('id="epic-graph-');
  });

  test("an epic whose items carry no edges says not yet graphed and still lists items", () => {
    const html = renderWith([item({ number: 1, title: "Epic", kind: "Epic" }), item({ number: 2, epicRef: R(1) })]);
    expect(html).toContain("Not yet graphed: this epic's items carry no parent / blocked-by edges yet");
    expect(html).toContain("blocked by: none observed");
  });

  test("epics with the same number in different repos get distinct anchors", () => {
    const other = (n: number): RefKey => `dezren39/nix#${n}` as RefKey;
    const pool = [
      item({ number: 1, title: "Epic one", kind: "Epic" }),
      item({ number: 2, epicRef: R(1) }),
      item({
        number: 1,
        title: "Nix epic one",
        kind: "Epic",
        ref: other(1),
        repo: "dezren39/nix",
        issueUrl: "https://github.com/dezren39/nix/issues/1",
      }),
      item({
        number: 2,
        epicRef: other(1),
        ref: other(2),
        repo: "dezren39/nix",
        issueUrl: "https://github.com/dezren39/nix/issues/2",
      }),
    ];
    const html = renderWith(pool);
    expect(html).toContain('id="epic-graph-developing-today-code-1"');
    expect(html).toContain('id="epic-graph-dezren39-nix-1"');
    expect(html).toContain('href="#epic-graph-dezren39-nix-1"');
  });

  test("the epic selector nav is isolated from the global nav styles", () => {
    const html = renderWith(fixturePool());
    expect(html).toContain('class="epic-graph-selector"');
    // Phase C: page CSS is the external /assets/app.css build; its rules
    // live in the stylesheet source (src/ui.css), not in the page HTML.
    expect(html).toContain('href="/assets/app.css"');
    const uiCss = readFileSync(join(import.meta.dir, "..", "src", "ui.css"), "utf8");
    expect(uiCss).toContain(".epic-graph-selector { position: static; }");
    expect(uiCss).toContain(".epic-graph-selector a { white-space: normal;");
  });

  test("the graph resolves against the full pool, not the Area-filtered detail pool", () => {
    const pool = [
      item({ number: 1, title: "Epic one", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), blockedBy: [R(3)] }),
      // The blocker is closed but lives in another Area, so the
      // Area-filtered detail pool does not hold it; the full epicPool does.
      item({ number: 3, state: "closed", area: "other-area" }),
    ];
    const config = projectConfigById("pm-dashboard")!;
    const items = new Map<RefKey, GhItem>();
    const detail = buildProjectDetail(config, items, pool, []);
    expect(detail.poolItems.map((p) => p.ref)).not.toContain(R(3));
    const html = renderProgramProject(detail, items, PROV, undefined, pool);
    expect(html).toContain("0 blocked on an unresolved blocker");
    expect(html).not.toContain("1 blocked on an unresolved blocker");
  });

  test("selector counts resolve against the full pool, entries stay project-scoped (Codex round-3)", () => {
    const pool = [
      item({ number: 1, title: "Epic one", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), blockedBy: [R(3)] }),
      // Closed blocker in another Area: only the full pool resolves it.
      item({ number: 3, state: "closed", area: "other-area" }),
    ];
    const config = projectConfigById("pm-dashboard")!;
    const items = new Map<RefKey, GhItem>();
    const detail = buildProjectDetail(config, items, pool, []);
    const html = renderProgramProject(detail, items, PROV, undefined, pool);
    // The selector entry agrees with the section: 0 blocked, not 1.
    expect(html).toContain("1 item \u00b7 0 blocked \u00b7 0 done");
    expect(html).not.toContain("1 blocked \u00b7");
  });

  test("dependency edges carry an arrowhead marker pointing at the blocker (Codex round-2)", () => {
    const html = renderWith(fixturePool());
    expect(html).toContain('<marker id="dep-arrow-epic-graph-developing-today-code-1"');
    expect(html).toContain('marker-end="url(#dep-arrow-epic-graph-developing-today-code-1)"');
    expect(html).toContain("with an arrowhead pointing at the blocker");
  });

  test("selector and sections are scoped to the project pool even when the full pool is passed (Codex round-2)", () => {
    const pool = [
      item({ number: 1, title: "Epic one", kind: "Epic" }),
      item({ number: 2, epicRef: R(1) }),
      // Another Area's epic, present only in the full epicPool the
      // server passes for membership/blocker resolution.
      item({ number: 9, title: "Other area epic", kind: "Epic", area: "other-area" }),
      item({ number: 10, epicRef: R(9), area: "other-area" }),
    ];
    const config = projectConfigById("pm-dashboard")!;
    const items = new Map<RefKey, GhItem>();
    const detail = buildProjectDetail(config, items, pool, []);
    expect(detail.poolItems.map((p) => p.ref)).not.toContain(R(9));
    const html = renderProgramProject(detail, items, PROV, undefined, pool);
    expect(html).toContain('id="epic-graph-developing-today-code-1"');
    expect(html).not.toContain('id="epic-graph-developing-today-code-9"');
    expect(html).not.toContain("Other area epic");
  });

  test("the epic-graph flex layout and its narrow-screen stacking ship in the stylesheet", () => {
    const html = renderWith(fixturePool());
    const uiCss = readFileSync(join(import.meta.dir, "..", "src", "ui.css"), "utf8");
    expect(uiCss).toContain(".epic-graph-flex { display: flex;");
    expect(uiCss).toContain(".epic-graph-flex { flex-direction: column; }");
    expect(html).toContain('class="epic-graph-flex"');
    expect(html).toContain('class="epic-graph-list"');
  });
});
