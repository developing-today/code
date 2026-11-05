import { describe, expect, test } from "bun:test";
import { epicGraph, epicSelector } from "../src/epic-graph.ts";
import type { ProjectPoolItem, RefKey } from "../src/model.ts";

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
  });

  test("ranks shared transitive blockers once per open item and shows additional blockers", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), labels: ["effort/s"], blockedBy: [R(90)] }),
      item({ number: 3, epicRef: R(1), labels: ["effort/m"], blockedBy: [R(2), R(91)] }),
      // #4 reaches #90 along two paths (#4 → #2 and #4 → #3 → #2).
      item({ number: 4, epicRef: R(1), labels: ["effort/xs"], blockedBy: [R(2), R(3)] }),
      item({ number: 90, blockedBy: [R(99)] }),
      item({ number: 91, blockedBy: [R(99)] }),
    ];
    const g = epicGraph(pool, R(1));
    const shared = g.rankedBlockers.find((blocker) => blocker.ref === R(99))!;
    expect(shared.state).toBeNull();
    expect(shared.issueUrl).toBe("https://github.com/developing-today/code/issues/99");
    expect(shared.downstreamCount).toBe(3);
    expect(shared.downstreamItems.map((downstream) => downstream.ref)).toEqual([R(2), R(3), R(4)]);
    expect(shared.downstreamItems.find((downstream) => downstream.ref === R(3))!.additionalBlockers).toEqual([R(2), R(91)]);
    expect(shared.downstreamItems.find((downstream) => downstream.ref === R(4))!.additionalBlockers).toEqual([R(2), R(3)]);

    // Ranking summaries cover the complete member set, not only graph nodes.
    expect(epicGraph(pool, R(1), 2).rankedBlockers).toEqual(g.rankedBlockers);
  });

  test("ties use known effort weight then canonical ref, and report unsized contributors", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), labels: ["effort/xs"], blockedBy: [R(90)] }),
      item({ number: 3, epicRef: R(1), labels: ["effort/s"], blockedBy: [R(90)] }),
      item({ number: 4, epicRef: R(1), labels: ["effort/m"], blockedBy: [R(91)] }),
      item({ number: 5, epicRef: R(1), blockedBy: [R(91)] }),
      item({ number: 6, epicRef: R(1), labels: ["effort/s"], blockedBy: [R(92)] }),
      item({ number: 7, epicRef: R(1), labels: ["effort/xs"], blockedBy: [R(92)] }),
      item({ number: 90 }),
      item({ number: 91 }),
      item({ number: 92 }),
    ];
    const ranked = epicGraph(pool, R(1)).rankedBlockers;
    expect(ranked.map((blocker) => blocker.ref)).toEqual([R(91), R(90), R(92)]);
    expect(ranked.map((blocker) => blocker.downstreamCount)).toEqual([2, 2, 2]);
    expect(ranked.map((blocker) => blocker.knownEffortWeight)).toEqual([12, 5, 5]);
    expect(ranked[0]!.unsizedCount).toBe(1);

    const reversed = epicGraph([...pool].reverse(), R(1)).rankedBlockers;
    expect(reversed).toEqual(ranked);
  });

  test("matches a reachability model for every four-member dependency DAG", () => {
    const members = [2, 3, 4, 5];
    const edges: [number, number][] = [
      [2, 3],
      [2, 4],
      [2, 5],
      [3, 4],
      [3, 5],
      [4, 5],
    ];
    const labels = new Map<number, string>([
      [2, "effort/xs"],
      [3, "effort/s"],
      [4, "effort/m"],
      [5, "effort/l"],
    ]);
    const weights = new Map<number, number>([
      [2, 1],
      [3, 4],
      [4, 12],
      [5, 40],
    ]);

    // The oracle is set reachability: for each open member, find every
    // blocker in its ancestry, then aggregate member sets and their weight.
    for (let mask = 0; mask < 1 << edges.length; mask += 1) {
      const blockedBy = new Map<number, number[]>(members.map((number) => [number, []]));
      edges.forEach(([descendant, blocker], bit) => {
        if ((mask & (1 << bit)) !== 0) blockedBy.get(descendant)!.push(blocker);
      });
      const pool = [
        item({ number: 1, title: "Epic", kind: "Epic" }),
        ...members.map((number) =>
          item({
            number,
            epicRef: R(1),
            labels: [labels.get(number)!],
            blockedBy: blockedBy.get(number)!.map(R),
          }),
        ),
      ];
      const descendants = new Map<number, Set<number>>();
      for (const member of members) {
        const visited = new Set<number>();
        const pending = [...blockedBy.get(member)!];
        while (pending.length > 0) {
          const blocker = pending.pop()!;
          if (visited.has(blocker)) continue;
          visited.add(blocker);
          const work = descendants.get(blocker) ?? new Set<number>();
          work.add(member);
          descendants.set(blocker, work);
          pending.push(...blockedBy.get(blocker)!);
        }
      }
      const expected = [...descendants.entries()]
        .map(([blocker, work]) => ({
          ref: R(blocker),
          downstreamCount: work.size,
          knownEffortWeight: [...work].reduce((sum, number) => sum + weights.get(number)!, 0),
          downstreamItems: [...work].sort((a, b) => R(a).localeCompare(R(b))).map(R),
        }))
        .sort(
          (a, b) =>
            b.downstreamCount - a.downstreamCount ||
            b.knownEffortWeight - a.knownEffortWeight ||
            a.ref.localeCompare(b.ref),
        );
      const actual = epicGraph(pool, R(1)).rankedBlockers.map((blocker) => ({
        ref: blocker.ref,
        downstreamCount: blocker.downstreamCount,
        knownEffortWeight: blocker.knownEffortWeight,
        downstreamItems: blocker.downstreamItems.map((downstream) => downstream.ref),
      }));
      expect(actual).toEqual(expected);
    }
  });

  test("closed and merged blockers, their ancestors, and closed descendants do not count", () => {
    const pool = [
      item({ number: 1, title: "Epic", kind: "Epic" }),
      item({ number: 2, epicRef: R(1), blockedBy: [R(90), R(91)] }),
      item({ number: 3, epicRef: R(1), state: "closed", blockedBy: [R(94)] }),
      item({ number: 4, epicRef: R(1), blockedBy: [R(95)] }),
      item({ number: 90, state: "closed", blockedBy: [R(96)] }),
      item({ number: 91, state: "merged", blockedBy: [R(97)] }),
      item({ number: 95, blockedBy: [R(98)] }),
    ];
    const refs = epicGraph(pool, R(1)).rankedBlockers.map((blocker) => blocker.ref);
    expect(refs).toEqual([R(95), R(98)]);
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

function renderWith(pool: ProjectPoolItem[]): string {
  const config = projectConfigById("pm-dashboard")!;
  const items = new Map<RefKey, GhItem>();
  return renderProgramProject(buildProjectDetail(config, items, pool, []), items, PROV);
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

  test("the vertical list names blockers and states as text, external refs as unknown", () => {
    const html = renderWith(fixturePool());
    expect(html).toContain("not in this pool / status unknown — unresolved");
    expect(html).toContain("(unresolved)");
    expect(html).toContain("blocked by:");
    expect(html).toContain("waiting on 1 unresolved blocker");
  });

  test("renders the primary downstream ranking with uncertainty, additional blockers, and separate unsized counts", () => {
    const html = renderWith(fixturePool());
    expect(html).toContain("Primary ranking — blockers by downstream open work");
    expect(html).toContain("2 distinct open downstream items; known effort weight 0; 2 unsized contributors");
    expect(html).toContain("outside this pool; status unknown — unresolved");
    expect(html).toContain("additional unresolved blockers");
    expect(html).toContain("not a promise that every item is immediately unblocked");
    expect(html).toContain("longest blocked chain");
    expect(html).toContain("Open items by current unresolved direct-blocker count");
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
    expect(html).toContain(".epic-graph-selector { position: static; }");
    expect(html).toContain(".epic-graph-selector a { white-space: normal;");
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
    expect(html).toContain(".epic-graph-flex { display: flex;");
    expect(html).toContain(".epic-graph-flex { flex-direction: column; }");
    expect(html).toContain('class="epic-graph-flex"');
    expect(html).toContain('class="epic-graph-list"');
  });
});
