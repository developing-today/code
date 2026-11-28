import type { ProjectPoolItem, RefKey } from "./model.ts";
import { isValidRef } from "./model.ts";
import type { NativeSubissue } from "./epic-subtree.ts";

export interface EpicRollup {
  /** Resolved pool items whose epicRef points at this epic. */
  total: number;
  /** Children closed/merged in GitHub or marked Done in Project Status. */
  done: number;
  /** Open children with at least one unresolved blocker. */
  blocked: number;
  /** Counts of the Project Status values, including items with no status. */
  statusMix: { status: string; count: number }[];
}

/** The one epic done-rule (#3954): a child counts as done when it is
 *  closed/merged in GitHub OR its Project Status is Done. Every surface
 *  that reports epic progress (/projects rollups, /roadmap drill-down)
 *  must use this helper so the counts cannot drift apart again. */
export function isEpicChildDone(child: ProjectPoolItem): boolean {
  return child.state === "closed" || child.state === "merged" || child.status?.trim() === "Done";
}

/** Facts required to evaluate the closure policy in design §7. A monitor
 *  must derive these from the full native-parent subtree; this classifier
 *  deliberately does not infer ancestry or label meaning from partial pools. */
export interface EpicClosureGateFacts {
  /** True only when the monitor fetched the complete native-parent subtree. */
  factsComplete: boolean;
  /** Design tracking issues in the subtree that are not done or Drew-gated. */
  unfinishedDesignRefs: readonly RefKey[];
  /** Open descendants already classified as a high/critical bug or follow-up. */
  qualifyingOpenDescendantRefs: readonly RefKey[];
  /** Unfinished children for which no re-parent destination is recorded. */
  unfinishedChildrenWithoutReparentDestinationRefs: readonly RefKey[];
}

export type EpicClosureGateCondition =
  | "source_incomplete"
  | "designs_incomplete"
  | "qualifying_open_descendants"
  | "unfinished_children_without_reparent_destination";

export interface EpicClosureGateBlocker {
  condition: EpicClosureGateCondition;
  refs: RefKey[];
}

export interface EpicClosureGateResult {
  closable: boolean;
  blockers: EpicClosureGateBlocker[];
}

/** Apply the three closure conditions from design §7 to already-derived facts.
 *  The result is deterministic and preserves the blocking refs for reporting. */
export function evaluateEpicClosureGate(facts: EpicClosureGateFacts): EpicClosureGateResult {
  const blockers: EpicClosureGateBlocker[] = [];
  const addBlocker = (condition: EpicClosureGateCondition, refs: readonly RefKey[]) => {
    if (refs.length > 0) blockers.push({ condition, refs: [...refs] });
  };

  if (!facts.factsComplete) blockers.push({ condition: "source_incomplete", refs: [] });
  addBlocker("designs_incomplete", facts.unfinishedDesignRefs);
  addBlocker("qualifying_open_descendants", facts.qualifyingOpenDescendantRefs);
  addBlocker(
    "unfinished_children_without_reparent_destination",
    facts.unfinishedChildrenWithoutReparentDestinationRefs,
  );

  return { closable: blockers.length === 0, blockers };
}

/** Conservative read-only §7 report over a successfully fetched complete tree.
 *  No re-parent plan is yet verified, so every open descendant blocks condition 3.
 *  Only explicit design tracking labels identify designs, not design membership. */
export function deriveEpicClosureGateFacts(
  rootRef: RefKey,
  descendants: readonly NativeSubissue[],
): EpicClosureGateFacts {
  const unfinishedDesignRefs: RefKey[] = [];
  const qualifyingOpenDescendantRefs: RefKey[] = [];
  const unfinishedChildrenWithoutReparentDestinationRefs: RefKey[] = [];

  for (const child of descendants) {
    const labels = new Set(child.labels);
    const done = child.state === "CLOSED";
    const design = labels.has("kind/design") || labels.has("design");
    const drewGated = labels.has("user/escalated");

    if (design && !done && !drewGated) unfinishedDesignRefs.push(child.ref);

    if (child.state === "OPEN") {
      const qualifyingBug =
        (labels.has("kind/bug") || labels.has("bug")) &&
        (labels.has("priority/high") || labels.has("priority/critical"));
      if (qualifyingBug || labels.has("meta/follow-up")) qualifyingOpenDescendantRefs.push(child.ref);

      if (child.parentRef === rootRef) unfinishedChildrenWithoutReparentDestinationRefs.push(child.ref);
    }
  }

  return {
    factsComplete: true,
    unfinishedDesignRefs,
    qualifyingOpenDescendantRefs,
    unfinishedChildrenWithoutReparentDestinationRefs,
  };
}

/** Collapse multiple board memberships to one canonical issue for issue-level progress. */
export function uniqueProjectItemsByRef(pool: readonly ProjectPoolItem[]): ProjectPoolItem[] {
  const byRef = new Map<RefKey, ProjectPoolItem>();
  for (const item of pool) {
    if (!byRef.has(item.ref)) byRef.set(item.ref, item);
  }
  return [...byRef.values()];
}

/** One representative for every ref classified as an Epic on any board. */
export function uniqueEpicItemsByRef(pool: readonly ProjectPoolItem[]): ProjectPoolItem[] {
  const byRef = new Map<RefKey, ProjectPoolItem>();
  for (const item of pool) if (item.kind === "Epic" && !byRef.has(item.ref)) byRef.set(item.ref, item);
  return [...byRef.values()];
}

/** Build rollups for epics in the pool, using only explicit epicRef edges. */
export function buildEpicRollups(pool: readonly ProjectPoolItem[]): Map<RefKey, EpicRollup> {
  const epicRefs = new Set(uniqueEpicItemsByRef(pool).map((epic) => epic.ref));
  const uniquePool = uniqueProjectItemsByRef(pool);
  const byRef = new Map(uniquePool.map((item) => [item.ref, item]));
  const childrenByEpic = new Map<RefKey, ProjectPoolItem[]>();
  for (const item of uniquePool) {
    if (!isValidRef(item.epicRef)) continue;
    const children = childrenByEpic.get(item.epicRef) ?? [];
    children.push(item);
    childrenByEpic.set(item.epicRef, children);
  }

  const rollups = new Map<RefKey, EpicRollup>();
  for (const epic of uniquePool) {
    if (!epicRefs.has(epic.ref)) continue;
    const children = childrenByEpic.get(epic.ref) ?? [];
    const statuses = new Map<string, number>();
    let done = 0;
    let blocked = 0;
    for (const child of children) {
      const status = child.status?.trim() || "No status";
      statuses.set(status, (statuses.get(status) ?? 0) + 1);
      if (isEpicChildDone(child)) done += 1;
      if (
        child.state === "open" &&
        (child.blockedBy ?? []).some((ref) => {
          if (!isValidRef(ref)) return false;
          const blocker = byRef.get(ref);
          return blocker === undefined || blocker.state === "open";
        })
      ) {
        blocked += 1;
      }
    }
    const statusMix = [...statuses]
      .map(([status, count]) => ({ status, count }))
      .sort((a, b) => {
        if (a.status === "No status") return 1;
        if (b.status === "No status") return -1;
        return a.status.localeCompare(b.status);
      });
    rollups.set(epic.ref, { total: children.length, done, blocked, statusMix });
  }
  return rollups;
}
