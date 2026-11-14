// Per-epic dependency graph for Project-pool items (#3973; design
// doc/2026-11-01T21-05-00Z_design_aggregator_dependency_graph_1257.md,
// merged in PR #3980). Slice A: the data layer — epic selector data
// plus one selected epic's graph. Slice B (render.ts) consumes this.
//
// Pure module: no I/O, no feed access. The graph source is the native
// relation contract from #2399 — parentRef / blockedBy / blocks /
// epicRef on ProjectPoolItem — never prose. Honesty rules, mirroring
// graph.ts:
// - dependency edges stay distinct from hierarchy (parent) edges;
// - a one-hop ref outside the pool is an unresolved external node:
//   canonical ref + a link built from the ref, but no fabricated
//   title or state, and it never counts as resolved;
// - a capped graph reports its true total and the truncated count;
// - an epic with no pool items, or an epic ref the pool does not
//   hold, is "unavailable" — never an empty success.

import type { ProjectPoolItem, RefKey } from "./model.ts";
import { isValidRef } from "./model.ts";
import { buildEpicRollups } from "./epics.ts";
import { effortWeight, modeOf } from "./weights.ts";

export const EPIC_GRAPH_DEFAULT_CAP = 120;

/** The distinct open Epic refs reachable from `ref` through open items along
 * blockedBy edges (the ref itself included when it is an open Epic). One
 * visited set is shared across the whole traversal, so each node is expanded
 * once: diamond graphs stay linear (#6371) and cycles terminate. Reachability
 * does not depend on the path taken, so the set is the same as a per-branch
 * traversal would give. Insertion order is the DFS discovery order. */
function openEpicsReachableFrom(ref: RefKey, byRef: Map<RefKey, ProjectPoolItem>): Set<RefKey> {
  const result = new Set<RefKey>();
  const visited = new Set<RefKey>();
  const visit = (current: RefKey): void => {
    if (visited.has(current)) return;
    visited.add(current);
    const item = byRef.get(current);
    if (!item || item.state !== "open") return;
    if (item.kind === "Epic") result.add(current);
    for (const next of item.blockedBy ?? []) visit(next);
  };
  visit(ref);
  return result;
}

export interface EpicSelectorEntry {
  ref: RefKey;
  number: number;
  title: string;
  issueUrl: string;
  state: string;
  /** Pool items whose epicRef names this epic (the epic itself excluded). */
  itemCount: number;
  /** Open member items waiting on at least one unresolved blocker. */
  blockedCount: number;
  /** Member items done under the one epic done-rule (#3954). */
  doneCount: number;
}

/** Selector data for the per-epic graph: every epic in the pool that
 * has at least one member item, with the rollup counts from
 * buildEpicRollups so the numbers cannot drift from the other epic
 * surfaces. Ordered by blocked count, then item count, then ref —
 * the epics with the most blocked work surface first, ties stable. */
export function epicSelector(pool: readonly ProjectPoolItem[]): EpicSelectorEntry[] {
  const rollups = buildEpicRollups(pool);
  const entries: EpicSelectorEntry[] = [];
  for (const item of pool) {
    if (item.kind !== "Epic") continue;
    const rollup = rollups.get(item.ref);
    if (!rollup || rollup.total === 0) continue;
    entries.push({
      ref: item.ref,
      number: item.number,
      title: item.title,
      issueUrl: item.issueUrl,
      state: item.state,
      itemCount: rollup.total,
      blockedCount: rollup.blocked,
      doneCount: rollup.done,
    });
  }
  entries.sort((a, b) => b.blockedCount - a.blockedCount || b.itemCount - a.itemCount || a.ref.localeCompare(b.ref));
  return entries;
}

export type EpicGraphEdgeType = "dependency" | "hierarchy";

export interface EpicGraphNode {
  ref: RefKey;
  number: number;
  /** Null for an out-of-pool ref: the pool has no title for it and
   * none is fabricated. */
  title: string | null;
  issueUrl: string;
  /** Pool state ("open" / "closed" / "merged"); null when the ref is
   * outside the pool and its state is unknown. */
  state: string | null;
  /** The node is a pool item (member, the epic, or a one-hop pool ref). */
  inPool: boolean;
  /** The node's epicRef names the selected epic (or it is the epic). */
  member: boolean;
  /** Out-of-pool ref: not in this pool, status unknown, unresolved. */
  unresolved: boolean;
  epic: boolean;
  /** mode/* label value for pool items, else null. */
  mode: string | null;
  /** Native blockedBy refs of the pool item (all of them, including
   * any the cap left unshown); empty for external nodes. */
  blockers: RefKey[];
  /** Native blocks refs of the pool item; empty for external nodes. */
  dependents: RefKey[];
  /** Depth in the hierarchy layout (roots are 0). */
  depth: number;
  /** Row in the laid-out graph, DFS order. */
  row: number;
}

export interface EpicGraphEdge {
  from: RefKey;
  to: RefKey;
  type: EpicGraphEdgeType;
}

export interface RankedBlockerItem {
  ref: RefKey;
  issueUrl: string;
  /** Null when this open item has no recognized effort label. */
  effortWeight: number | null;
  /** Current unresolved direct blockers other than this ranked blocker. */
  additionalBlockers: RefKey[];
}

export interface RankedBlocker {
  ref: RefKey;
  issueUrl: string;
  /** Null for an out-of-pool blocker whose state is unknown. */
  state: string | null;
  /** Distinct open epic members in this blocker's transitive blockedBy ancestry. */
  downstreamCount: number;
  /** Sum over recognized effort labels; unsized items are reported separately. */
  knownEffortWeight: number;
  unsizedCount: number;
  downstreamItems: RankedBlockerItem[];
}

export interface EpicGraph {
  status: "ok" | "unavailable";
  /** Why the graph is unavailable; null when status is "ok". */
  reason: string | null;
  epic: { ref: RefKey; number: number; title: string; issueUrl: string; state: string } | null;
  nodes: EpicGraphNode[];
  edges: EpicGraphEdge[];
  /** Nodes in the full (uncapped) graph. */
  totalNodes: number;
  /** Nodes left out by the cap (0 when everything is shown). */
  truncated: number;
  /** Out-of-pool unresolved refs in the full one-hop set, counted
   * before the node cap: external refs sort last in the node order,
   * so a capped graph would otherwise report zero of them. */
  externalCount: number;
  /** Pool items in the graph whose epicRef names the epic. */
  memberCount: number;
  /** Open member items waiting on at least one unresolved blocker. */
  blockedCount: number;
  /** Every open member waiting on unresolved blockers, computed over
   * the full member set BEFORE the node cap, worst-blocked first. The
   * ranked summary renders from this, never from the capped nodes:
   * a member the cap hid is still blocked. */
  rankedBlocked: {
    ref: RefKey;
    issueUrl: string;
    /** Transitive downstream open work count (distinct open epic items in transitive blockedBy ancestry). */
    downstreamCount: number;
    /** Known effort weight of the blocker, or null if unsized. */
    effortWeight: number | null;
    /** Blockers of this blocker (transitive one level). */
    blockers: RefKey[];
    /** Contributing downstream items that contribute to the downstreamCount. */
    contributingItems: RefKey[];
  }[];
  /** Unresolved blockers ranked by distinct open downstream epic work.
   * Also computed over the full member set before the node cap. */
  rankedBlockers: RankedBlocker[];
  maxDepth: number;
}

function unavailable(reason: string): EpicGraph {
  return {
    status: "unavailable",
    reason,
    epic: null,
    nodes: [],
    edges: [],
    totalNodes: 0,
    truncated: 0,
    externalCount: 0,
    memberCount: 0,
    blockedCount: 0,
    rankedBlocked: [],
    rankedBlockers: [],
    maxDepth: 0,
  };
}

/** Canonical issue URL for a ref ("owner/repo#123"). */
function refIssueUrl(ref: RefKey): string {
  const hash = ref.lastIndexOf("#");
  const slug = hash > 0 ? ref.slice(0, hash) : ref;
  const number = hash > 0 ? ref.slice(hash + 1) : "";
  return `https://github.com/${slug}/issues/${number}`;
}

function refNumber(ref: RefKey): number {
  const hash = ref.lastIndexOf("#");
  const n = hash > 0 ? Number(ref.slice(hash + 1)) : NaN;
  return Number.isFinite(n) ? n : 0;
}

/** The selected epic's graph: the epic, its member items, and the
 * one-hop native references of those items (parent, blockedBy,
 * blocks) — pool items when the pool holds them, unresolved external
 * stubs when it does not. No second hop is followed. */
export function epicGraph(
  pool: readonly ProjectPoolItem[],
  epicRef: RefKey,
  cap: number = EPIC_GRAPH_DEFAULT_CAP,
): EpicGraph {
  const byRef = new Map(pool.map((p) => [p.ref, p]));
  const epicItem = byRef.get(epicRef);
  if (!epicItem || epicItem.kind !== "Epic") {
    return unavailable(`epic ${epicRef} is not in this pool, so its dependency graph is unavailable`);
  }
  const members = pool.filter((p) => p.epicRef === epicRef);
  if (members.length === 0) {
    return unavailable(`no pool items belong to epic ${epicRef} yet — it is not yet graphed`);
  }

  // Deterministic node order: the epic, members by ref, one-hop pool
  // refs by ref, external refs by ref. The cap keeps the head of this
  // order; the tail is reported as truncated, never silently dropped.
  const memberSet = new Set<RefKey>([epicRef, ...members.map((m) => m.ref)]);
  const oneHopPool = new Map<RefKey, ProjectPoolItem>();
  const externalRefs = new Set<RefKey>();
  const collectRefs = (item: ProjectPoolItem) => {
    const targets = [
      ...(isValidRef(item.parentRef) ? [item.parentRef] : []),
      ...(item.blockedBy ?? []).filter(isValidRef),
      ...(item.blocks ?? []).filter(isValidRef),
    ];
    for (const target of targets) {
      if (memberSet.has(target)) continue;
      const held = byRef.get(target);
      if (held) oneHopPool.set(target, held);
      else externalRefs.add(target);
    }
  };
  collectRefs(epicItem);
  for (const m of members) collectRefs(m);

  const byRefOrder = (a: RefKey, b: RefKey) => a.localeCompare(b);
  const ordered: { item: ProjectPoolItem | null; ref: RefKey }[] = [
    { item: epicItem, ref: epicRef },
    ...[...members].sort((a, b) => byRefOrder(a.ref, b.ref)).map((m) => ({ item: m, ref: m.ref })),
    ...[...oneHopPool.values()].sort((a, b) => byRefOrder(a.ref, b.ref)).map((m) => ({ item: m, ref: m.ref })),
    ...[...externalRefs].sort(byRefOrder).map((ref) => ({ item: null, ref })),
  ];
  const totalNodes = ordered.length;
  const shown = ordered.slice(0, Math.max(0, cap));
  const shownRefs = new Set(shown.map((s) => s.ref));

  // Edges among shown nodes only. Dependency edges point from the
  // blocked item to its blocker (blockedBy) and from the blocked item
  // to the item a blocks[] entry names, so both native directions
  // land on the same item → blocker orientation; hierarchy edges
  // point parent → child. A pair joined by both keeps both edges.
  const edgeKeys = new Set<string>();
  const edges: EpicGraphEdge[] = [];
  const addEdge = (from: RefKey, to: RefKey, type: EpicGraphEdgeType) => {
    if (from === to || !shownRefs.has(from) || !shownRefs.has(to)) return;
    const key = `${type}:${from}->${to}`;
    if (edgeKeys.has(key)) return;
    edgeKeys.add(key);
    edges.push({ from, to, type });
  };
  for (const { item } of shown) {
    if (!item) continue;
    if (isValidRef(item.parentRef)) addEdge(item.parentRef, item.ref, "hierarchy");
    for (const blocker of item.blockedBy ?? []) {
      if (isValidRef(blocker)) addEdge(item.ref, blocker, "dependency");
    }
    for (const blocked of item.blocks ?? []) {
      if (isValidRef(blocked)) addEdge(blocked, item.ref, "dependency");
    }
  }

  // Hierarchy layout, same discipline as depthLayout: DFS from roots
  // (parent absent or outside the shown set), children in ref order,
  // parent cycles degrade to roots instead of looping.
  const children = new Map<RefKey, RefKey[]>();
  const hasShownParent = new Set<RefKey>();
  for (const e of edges) {
    if (e.type !== "hierarchy") continue;
    const list = children.get(e.from) ?? [];
    list.push(e.to);
    children.set(e.from, list);
    hasShownParent.add(e.to);
  }
  for (const list of children.values()) list.sort(byRefOrder);
  const roots = shown
    .map((s) => s.ref)
    .filter((ref) => !hasShownParent.has(ref))
    .sort((a, b) => (a === epicRef ? -1 : b === epicRef ? 1 : byRefOrder(a, b)));
  const layout = new Map<RefKey, { depth: number; row: number }>();
  const visit = (ref: RefKey, depth: number) => {
    if (layout.has(ref)) return;
    layout.set(ref, { depth, row: layout.size });
    for (const child of children.get(ref) ?? []) visit(child, depth + 1);
  };
  for (const root of roots) visit(root, 0);
  for (const { ref } of shown) visit(ref, 0); // cycle leftovers become roots

  // Summaries cover the FULL member set, before the node cap: a member
  // hidden by the cap is still blocked, still counted, still ranked.
  const unresolvedBlockersOf = (item: ProjectPoolItem): RefKey[] =>
    (item.blockedBy ?? []).filter(isValidRef).filter((b) => {
      const blocker = byRef.get(b);
      return blocker === undefined || blocker.state === "open";
    });
  const blockedCount = members.filter((m) => m.state === "open" && unresolvedBlockersOf(m).length > 0).length;

  // Compute rankedBlocked with transitive downstream open work ranking
  const rankedBlocked = members
    .filter((m) => m.ref !== epicRef && m.state === "open")
    .map((m) => {
      const blockers = unresolvedBlockersOf(m);

      // For each blocker, compute its downstream open epic descendant count
      const blockerScores = blockers.map((blockerRef) => {
        // Distinct open epic items in the transitive blockedBy ancestry of this
        // blocker; they are both the count and the contributing items.
        const downstream = openEpicsReachableFrom(blockerRef, byRef);

        // Get effort weight of the blocker
        const blockerItem = byRef.get(blockerRef);
        const weight = blockerItem ? effortWeight(blockerItem.labels) : null;

        return {
          ref: blockerRef,
          downstreamCount: downstream.size,
          effortWeight: weight,
          contributingItems: Array.from(downstream),
        };
      });

      return {
        ref: m.ref,
        issueUrl: m.issueUrl,
        blockers: blockers,
        // Sort blockers by downstreamCount (desc), then effortWeight (desc), then ref
        sortedBlockers: [...blockerScores].sort(
          (a, b) =>
            b.downstreamCount - a.downstreamCount ||
            (b.effortWeight === null ? -1 : b.effortWeight) - (a.effortWeight === null ? -1 : a.effortWeight) ||
            byRefOrder(a.ref, b.ref),
        ),
      };
    })
    .filter((r) => r.blockers.length > 0)
    // Sort members by their worst blocker's downstreamCount, then effortWeight, then ref
    .sort((a, b) => {
      const aWorst = a.sortedBlockers[0];
      const bWorst = b.sortedBlockers[0];
      return (
        (bWorst?.downstreamCount ?? 0) - (aWorst?.downstreamCount ?? 0) ||
        (bWorst?.effortWeight === null ? -1 : bWorst.effortWeight) -
          (aWorst?.effortWeight === null ? -1 : aWorst.effortWeight) ||
        byRefOrder(a.ref, b.ref)
      );
    })
    .map((r) => ({
      ref: r.ref,
      issueUrl: r.issueUrl,
      downstreamCount: r.sortedBlockers[0]?.downstreamCount ?? 0,
      effortWeight: r.sortedBlockers[0]?.effortWeight ?? null,
      blockers: r.sortedBlockers.map((b) => b.ref),
      contributingItems: [...new Set(r.sortedBlockers.flatMap((b) => b.contributingItems))],
    }));

  // For each open member, walk its complete unresolved blockedBy ancestry.
  // The per-member visited set means shared paths and cycles contribute
  // that member only once to each blocker. Unknown refs remain unresolved
  // candidates but cannot be traversed beyond the ref itself.
  const contributorsByBlocker = new Map<RefKey, Map<RefKey, ProjectPoolItem>>();
  for (const member of members) {
    if (member.ref === epicRef || member.state !== "open") continue;
    const visited = new Set<RefKey>();
    const visitBlocker = (ref: RefKey): void => {
      if (visited.has(ref)) return;
      visited.add(ref);
      const blocker = byRef.get(ref);
      if (blocker && blocker.state !== "open") return;

      // A member is downstream work only for other blockers, never itself
      // when a malformed self-edge or dependency cycle loops back to it.
      if (ref !== member.ref) {
        const contributors = contributorsByBlocker.get(ref) ?? new Map<RefKey, ProjectPoolItem>();
        contributors.set(member.ref, member);
        contributorsByBlocker.set(ref, contributors);
      }

      if (!blocker) return;
      for (const ancestor of unresolvedBlockersOf(blocker).sort(byRefOrder)) visitBlocker(ancestor);
    };
    for (const blocker of unresolvedBlockersOf(member).sort(byRefOrder)) visitBlocker(blocker);
  }
  const rankedBlockers: RankedBlocker[] = [...contributorsByBlocker.entries()]
    .map(([ref, contributors]) => {
      const downstreamItems: RankedBlockerItem[] = [...contributors.values()]
        .sort((a, b) => byRefOrder(a.ref, b.ref))
        .map((item) => ({
          ref: item.ref,
          issueUrl: item.issueUrl,
          effortWeight: effortWeight(item.labels),
          additionalBlockers: [...new Set(unresolvedBlockersOf(item).filter((b) => b !== ref))].sort(byRefOrder),
        }));
      const knownEffortWeight = downstreamItems.reduce((sum, item) => sum + (item.effortWeight ?? 0), 0);
      const candidate = byRef.get(ref);
      return {
        ref,
        issueUrl: candidate?.issueUrl ?? refIssueUrl(ref),
        state: candidate?.state ?? null,
        downstreamCount: downstreamItems.length,
        knownEffortWeight,
        unsizedCount: downstreamItems.filter((item) => item.effortWeight === null).length,
        downstreamItems,
      };
    })
    .sort(
      (a, b) =>
        b.downstreamCount - a.downstreamCount || b.knownEffortWeight - a.knownEffortWeight || byRefOrder(a.ref, b.ref),
    );

  const nodes: EpicGraphNode[] = shown.map(({ item, ref }) => {
    const pos = layout.get(ref) ?? { depth: 0, row: 0 };
    if (!item) {
      return {
        ref,
        number: refNumber(ref),
        title: null,
        issueUrl: refIssueUrl(ref),
        state: null,
        inPool: false,
        member: false,
        unresolved: true,
        epic: false,
        mode: null,
        blockers: [],
        dependents: [],
        depth: pos.depth,
        row: pos.row,
      };
    }
    return {
      ref,
      number: item.number,
      title: item.title,
      issueUrl: item.issueUrl,
      state: item.state,
      inPool: true,
      member: memberSet.has(ref),
      unresolved: false,
      epic: item.kind === "Epic",
      mode: modeOf(item.labels),
      blockers: (item.blockedBy ?? []).filter(isValidRef),
      dependents: (item.blocks ?? []).filter(isValidRef),
      depth: pos.depth,
      row: pos.row,
    };
  });
  nodes.sort((a, b) => a.row - b.row);

  // External nodes have no pool item to read relations from, but the
  // edges already observed name them: a member blocked by an external
  // ref means that ref blocks the member, and a member's blocks[] entry
  // naming an external ref means the ref is blocked by the member.
  // Fill the external nodes' lists from the edges so the text list
  // agrees with the graph instead of reporting "none observed".
  const nodeByRef = new Map(nodes.map((n) => [n.ref, n]));
  for (const e of edges) {
    if (e.type !== "dependency") continue;
    const from = nodeByRef.get(e.from);
    const to = nodeByRef.get(e.to);
    if (from?.unresolved && !from.blockers.includes(e.to)) from.blockers.push(e.to);
    if (to?.unresolved && !to.dependents.includes(e.from)) to.dependents.push(e.from);
  }
  for (const n of nodes) {
    if (!n.unresolved) continue;
    n.blockers.sort(byRefOrder);
    n.dependents.sort(byRefOrder);
  }

  return {
    status: "ok",
    reason: null,
    epic: {
      ref: epicItem.ref,
      number: epicItem.number,
      title: epicItem.title,
      issueUrl: epicItem.issueUrl,
      state: epicItem.state,
    },
    nodes,
    edges,
    totalNodes,
    truncated: totalNodes - shown.length,
    externalCount: externalRefs.size,
    memberCount: members.length,
    blockedCount,
    rankedBlocked,
    rankedBlockers,
    maxDepth: nodes.reduce((m, n) => Math.max(m, n.depth), 0),
  };
}
