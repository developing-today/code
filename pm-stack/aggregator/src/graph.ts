// Issue-graph computations for Project-pool items (#2400, part 2 of
// #1122): blocked weight + longest blocked chain, and the depth layout
// behind the /program/<id> depth graph. Consumes the part-1 edge
// contract (#2399) — parentRef / blockedBy / blocks / epicRef on
// ProjectPoolItem — and the effort weights from weights.ts.
//
// Pure module: no I/O, no feed access. Absence is honest throughout: an
// item with no edges is simply not in the graph, a blocker the pool
// cannot resolve is treated as unresolved (never assumed done), and a
// project with no edges at all reports hasGraphEdges() === false so the
// render layer can say "not yet graphed" instead of drawing an empty
// chart or a fake zero.

import type { ProjectPoolItem, RefKey } from "./model.ts";
import { effortWeight, modeOf } from "./weights.ts";

/** True when any pool item carries at least one graph edge. */
export function hasGraphEdges(items: readonly ProjectPoolItem[]): boolean {
  return items.some(
    (p) => p.parentRef !== undefined || p.epicRef !== undefined || (p.blockedBy?.length ?? 0) > 0 || (p.blocks?.length ?? 0) > 0,
  );
}

export interface BlockedSummary {
  /** Open items waiting on at least one unresolved blocker. */
  blockedCount: number;
  /** Sized effort weight of the blocked open items. */
  blockedWeight: number;
  /** Blocked open items with no effort label (counted, never zeroed). */
  blockedUnsizedCount: number;
  /** Longest blocked-by chain, blocked item first, following blockers
   * that are open in the pool or outside it. Empty when nothing is
   * blocked. Cycle-safe: a ref never repeats. */
  longestChain: RefKey[];
}

/** A blocker is unresolved when the pool holds it as OPEN, or when the
 * pool does not hold it at all — an out-of-pool blocker cannot be shown
 * resolved from feed data, so it must not silently unblock. A blocker
 * the pool holds as closed/merged is resolved. */
function blockerUnresolved(ref: RefKey, byRef: Map<RefKey, ProjectPoolItem>): boolean {
  const item = byRef.get(ref);
  return item === undefined || item.state === "open";
}

export function blockedSummary(items: readonly ProjectPoolItem[]): BlockedSummary {
  const byRef = new Map(items.map((p) => [p.ref, p]));
  const out: BlockedSummary = { blockedCount: 0, blockedWeight: 0, blockedUnsizedCount: 0, longestChain: [] };

  // Longest chain ending anywhere: DFS over blockedBy edges with memo
  // on the longest simple path starting at each ref. The visiting set
  // keeps a blocked-by cycle from looping (GitHub data should be
  // acyclic; a cycle degrades to a path, never a hang).
  const memo = new Map<RefKey, RefKey[]>();
  const chainFrom = (ref: RefKey, visiting: Set<RefKey>): RefKey[] => {
    if (memo.has(ref) && visiting.size === 0) return memo.get(ref)!;
    if (visiting.has(ref)) return [];
    visiting.add(ref);
    const item = byRef.get(ref);
    let best: RefKey[] = [ref];
    for (const blocker of item?.blockedBy ?? []) {
      if (!blockerUnresolved(blocker, byRef)) continue;
      const rest = chainFrom(blocker, visiting);
      if (rest.length + 1 > best.length) best = [ref, ...rest];
    }
    visiting.delete(ref);
    if (visiting.size === 0) memo.set(ref, best);
    return best;
  };

  for (const item of items) {
    if (item.state !== "open") continue;
    const blocked = (item.blockedBy ?? []).some((b) => blockerUnresolved(b, byRef));
    if (!blocked) continue;
    out.blockedCount += 1;
    const w = effortWeight(item.labels);
    if (w === null) out.blockedUnsizedCount += 1;
    else out.blockedWeight += w;
    const chain = chainFrom(item.ref, new Set());
    if (chain.length > out.longestChain.length) out.longestChain = chain;
  }
  return out;
}

export interface DepthNode {
  ref: RefKey;
  number: number;
  title: string;
  issueUrl: string;
  state: string;
  /** Depth in the parent tree (roots are 0). */
  depth: number;
  /** Row in the laid-out graph, DFS order. */
  row: number;
  /** Project Kind === "Epic". */
  epic: boolean;
  /** mode/* label value, or null. */
  mode: string | null;
}

export interface DepthLayout {
  nodes: DepthNode[];
  /** Parent → child edges, both ends present in nodes. */
  edges: { from: RefKey; to: RefKey }[];
  maxDepth: number;
  /** Items left out by the cap (0 when everything is drawn). */
  truncated: number;
}

const DEFAULT_CAP = 120;

/** Lay the pool's parent tree out as (depth, row) coordinates for a
 * server-rendered SVG. Roots are items whose parent is absent or
 * outside the pool; children follow in ref order. A parent cycle (bad
 * data) degrades: the cyclic items become roots rather than looping. */
export function depthLayout(items: readonly ProjectPoolItem[], cap: number = DEFAULT_CAP): DepthLayout {
  const byRef = new Map(items.map((p) => [p.ref, p]));
  const children = new Map<RefKey, ProjectPoolItem[]>();
  const roots: ProjectPoolItem[] = [];
  for (const item of items) {
    const parent = item.parentRef !== undefined ? byRef.get(item.parentRef) : undefined;
    if (parent && parent.ref !== item.ref) {
      const list = children.get(parent.ref) ?? [];
      list.push(item);
      children.set(parent.ref, list);
    } else {
      roots.push(item);
    }
  }
  const byRefOrder = (a: ProjectPoolItem, b: ProjectPoolItem) => a.ref.localeCompare(b.ref);
  roots.sort(byRefOrder);
  for (const list of children.values()) list.sort(byRefOrder);

  const nodes: DepthNode[] = [];
  const edges: { from: RefKey; to: RefKey }[] = [];
  const placed = new Set<RefKey>();
  const visit = (item: ProjectPoolItem, depth: number) => {
    if (placed.has(item.ref) || nodes.length >= cap) return;
    placed.add(item.ref);
    nodes.push({
      ref: item.ref,
      number: item.number,
      title: item.title,
      issueUrl: item.issueUrl,
      state: item.state,
      depth,
      row: nodes.length,
      epic: item.kind === "Epic",
      mode: modeOf(item.labels),
    });
    for (const child of children.get(item.ref) ?? []) {
      if (placed.has(child.ref) || nodes.length >= cap) continue;
      edges.push({ from: item.ref, to: child.ref });
      visit(child, depth + 1);
    }
  };
  for (const root of roots) visit(root, 0);
  // Anything unreachable from a root sits in a parent cycle: lay it
  // out as a root rather than dropping it silently.
  for (const item of [...items].sort(byRefOrder)) {
    if (!placed.has(item.ref)) visit(item, 0);
  }
  return {
    nodes,
    edges: edges.filter((e) => placed.has(e.from) && placed.has(e.to)),
    maxDepth: nodes.reduce((m, n) => Math.max(m, n.depth), 0),
    truncated: items.length - nodes.length,
  };
}
