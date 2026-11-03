// Canonical work-item projection (#1206).
//
// Before this module, every page re-derived the same work from the
// orchestration section independently: Today via derive.ts
// todayFromSection, the Board via live.ts inFlightBoardItems, Needs-you
// via derive.ts buildAttention's tier handling, Projects via
// projects.ts refInOrchestration, and Program via program.ts — four+
// transforms of one section, so counts and states diverged by
// construction.
//
// assembleWorkItems is the ONE assembly: a single pass over the
// section that produces the canonical list of work items with stable
// identity, canonical status, queue rank, blockers, and epic/goal
// linkage. Page projections consume this list and may only re-shape it
// for presentation; they never re-derive identity, status, rank, or
// blockers from the raw section.
//
// Identity: a goal lined-up task is `orch:<goal-slug>/<title-slug>`
// (the id the Board's in-flight entries have always carried), keyed
// for dedupe by its GitHub ref when it has one, else by normalized
// title — the same task lined up under two goals is ONE item. A tier
// item keeps the publisher's id (sessionId | ref | "jules:<id>" |
// tracked goal id); a tier item whose ref (or id-as-ref) names a
// lined-up task MERGES into that task's item: the tier is an attention
// state of the same work, not a second item.
//
// Canonical status: a merged tier wins (needs-drew / needs-orchestrator
// — the item needs a person before it is merely queued), else the
// lined-up status. The lined-up status is preserved verbatim on
// `linedUpStatus` for consumers (the Board) whose contract names it.
//
// Queue rank: needs-drew first, then needs-orchestrator, then lined-up
// work by horizon ladder (today → tomorrow → next-3-days → longer →
// incoming) and, within a horizon, in-progress → blocked → queued →
// done-recently. Lower sorts first; ties keep section order.

import type { DependencyRef, Horizon, LinedUpTask, OrchestrationSection, RefKey, TierItem } from "./model.ts";

export type WorkItemStatus = "needs-drew" | "needs-orchestrator" | LinedUpTask["status"];

export type WorkItemTier = "needs-drew" | "needs-orchestrator";

export interface WorkItem {
  /** Canonical stable identity (see module header). */
  id: string;
  title: string;
  /** Canonical status — the status every page shows for this item. */
  status: WorkItemStatus;
  /** Queue rank across the whole section; lower sorts first. */
  rank: number;
  /** GitHub ref ("owner/repo#123") when the work names one. */
  ref?: RefKey;
  /** The lined-up task's own status, when the item is lined up under a
   *  goal — preserved even when a tier merge upgrades `status`. */
  linedUpStatus?: LinedUpTask["status"];
  /** Ladder bucket of the lined-up task, when lined up. */
  horizon?: Horizon;
  /** Why the task sits in its horizon bucket, when lined up. */
  horizonWhy?: string;
  /** Canonical blockers: the union of every surface's blocked-by refs. */
  blockers: DependencyRef[];
  /** Goal the item is lined up under (epic/goal linkage), when any. */
  goal?: string;
  /** Tracked/user goal id of that goal, when known. */
  goalId?: string | null;
  /** Attention tier the item sits in, when a tier claims it. */
  tier?: WorkItemTier;
  /** One line: why the item sits in its tier, when tier-claimed. */
  reason?: string;
  /** Where the item came from (OrchestrationSource label, or a goal's
   *  lined-up list, recorded as "orchestrator"). */
  source: string;
}

const slugify = (s: string | undefined): string =>
  (typeof s === "string" ? s : "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 60) || "task";

const normTitle = (s: string | undefined): string =>
  (typeof s === "string" ? s : "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim();

const HORIZON_RANK: Record<Horizon, number> = {
  today: 0,
  tomorrow: 1,
  "next-3-days": 2,
  longer: 3,
  incoming: 4,
};

const LINED_UP_STATUS_RANK: Record<LinedUpTask["status"], number> = {
  "in-progress": 0,
  blocked: 1,
  queued: 2,
  "done-recently": 3,
};

const TIER_RANK_BASE: Record<WorkItemTier, number> = {
  "needs-drew": 0,
  "needs-orchestrator": 1000,
};

/** Lined-up work sorts after both tiers: 2000 + horizon + status. */
const linedUpRank = (task: LinedUpTask): number =>
  2000 + (HORIZON_RANK[task.horizon] ?? 4) * 100 + (LINED_UP_STATUS_RANK[task.status] ?? 2) * 10;

function unionBlockers(into: DependencyRef[], extra: DependencyRef[]): DependencyRef[] {
  const out = [...into];
  for (const dep of extra) {
    if (!out.some((d) => d.task === dep.task && d.ref === dep.ref)) out.push(dep);
  }
  return out;
}

/** Malformed publishers can omit or mistype the optional blockedBy
 *  field; the canonical layer must not throw on it. Anything but an
 *  array of dependency records becomes no blockers. */
function asBlockers(value: unknown): DependencyRef[] {
  if (!Array.isArray(value)) return [];
  return value.filter((d): d is DependencyRef => {
    if (typeof d !== "object" || d === null) return false;
    const blocker = d as Record<string, unknown>;
    return (
      typeof blocker.task === "string" &&
      ["in-progress", "queued", "stalled", "done", "unknown"].includes(String(blocker.status)) &&
      (blocker.ref === undefined || typeof blocker.ref === "string")
    );
  });
}

/** Assemble the canonical work-item list from one orchestration section. */
export function assembleWorkItems(section: OrchestrationSection): WorkItem[] {
  const items: WorkItem[] = [];
  const byKey = new Map<string, WorkItem>();
  // Items assembled from goal lined-up lists. A ref-free tier item may
  // merge into one of these by title — but never into another tier
  // item: two tier items with different ids and no refs are distinct
  // work even when their titles match.
  const fromGoals = new Set<WorkItem>();
  const keyFor = (ref: RefKey | undefined, title: string): string => ref ?? `title:${normTitle(title)}`;

  for (const goal of section.goals ?? []) {
    for (const task of goal.linedUp ?? []) {
      const key = keyFor(task.ref, task.title);
      const existing = byKey.get(key);
      if (existing) {
        // The same task lined up under a second goal (or twice under
        // one) is one item: first goal wins, blockers union.
        existing.blockers = unionBlockers(existing.blockers, asBlockers(task.blockedBy));
        // A later ACTIVE observation supersedes an earlier completed
        // one: the same ref listed done-recently under one goal and
        // queued/blocked/in-progress under another is live work, and
        // consumers that filter on linedUpStatus (the Board) must not
        // lose it. (Codex P2 on PR #4674.)
        if (existing.linedUpStatus === "done-recently" && task.status !== "done-recently") {
          existing.linedUpStatus = task.status;
          if (!existing.tier) {
            existing.status = task.status;
            existing.rank = linedUpRank(task);
          }
          existing.horizon = task.horizon;
          existing.horizonWhy = task.horizonWhy;
        } else if (
          existing.horizon !== undefined &&
          (HORIZON_RANK[task.horizon] ?? 4) < (HORIZON_RANK[existing.horizon] ?? 4)
        ) {
          // A later observation may schedule the same task sooner
          // (e.g. horizon "today" under a second goal). Canonical
          // linkage keeps the first goal, but must not discard the
          // earlier scheduling: consumers filter on horizon (Today
          // keeps only "today"), so keep the most urgent horizon any
          // observation claims. (Codex P2 on PR #4674.)
          existing.horizon = task.horizon;
          existing.horizonWhy = task.horizonWhy;
          if (!existing.tier && existing.linedUpStatus) {
            existing.rank = linedUpRank({ ...task, status: existing.linedUpStatus });
          }
        }
        continue;
      }
      const item: WorkItem = {
        id: `orch:${slugify(goal.goal)}/${slugify(task.title)}`,
        title: task.title,
        status: task.status,
        rank: linedUpRank(task),
        ref: task.ref,
        linedUpStatus: task.status,
        horizon: task.horizon,
        horizonWhy: task.horizonWhy,
        blockers: asBlockers(task.blockedBy),
        goal: goal.goal,
        goalId: goal.goalId,
        source: "orchestrator",
      };
      byKey.set(key, item);
      fromGoals.add(item);
      // A tier item may name this task by its ref-as-id; index that too.
      if (task.ref) byKey.set(task.ref, item);
      items.push(item);
    }
  }

  const addTier = (tierItems: TierItem[], tier: WorkItemTier): void => {
    tierItems.forEach((t, index) => {
      const titleCandidate = byKey.get(keyFor(t.ref, t.title));
      const match =
        (t.ref ? byKey.get(t.ref) : undefined) ??
        byKey.get(t.id) ??
        (titleCandidate && fromGoals.has(titleCandidate) ? titleCandidate : undefined);
      if (match) {
        // If both tiers claim one item, preserve the higher-priority
        // needs-drew claim (the lower rank number) and its explanation.
        if (!match.tier || TIER_RANK_BASE[tier] < TIER_RANK_BASE[match.tier]) {
          match.tier = tier;
          match.status = tier;
          match.reason = t.reason;
        }
        match.rank = Math.min(match.rank, TIER_RANK_BASE[tier] + index);
        match.blockers = unionBlockers(match.blockers, asBlockers(t.blockedBy));
        if (!match.ref && t.ref) match.ref = t.ref;
        // Register the publisher's own id (and ref) against the merged
        // item: a later tier item that names this work only by that id
        // must merge into the same canonical item, not mint a second
        // one. (Codex P2 on PR #4674.)
        if (t.id) byKey.set(t.id, match);
        if (t.ref) byKey.set(t.ref, match);
        return;
      }
      const item: WorkItem = {
        id: t.id,
        title: t.title,
        status: tier,
        rank: TIER_RANK_BASE[tier] + index,
        ref: t.ref,
        blockers: asBlockers(t.blockedBy),
        tier,
        reason: t.reason,
        source: t.source,
      };
      // Do not put an unreferenced tier item in the title index: titles
      // are not identities for independently published tier items.
      if (t.ref) byKey.set(keyFor(t.ref, t.title), item);
      if (t.ref) byKey.set(t.ref, item);
      byKey.set(t.id, item);
      items.push(item);
    });
  };
  addTier(section.tiers?.needsDrew ?? [], "needs-drew");
  addTier(section.tiers?.needsOrchestrator ?? [], "needs-orchestrator");

  return items
    .map((item, index) => ({ item, index }))
    .sort((a, b) => a.item.rank - b.item.rank || a.index - b.index)
    .map(({ item }) => item);
}

/** The canonical item for one GitHub ref, or undefined. */
export function workItemForRef(items: WorkItem[], ref: RefKey): WorkItem | undefined {
  return items.find((item) => item.ref === ref);
}

/**
 * Every GitHub ref the section mentions as work: canonical item refs,
 * their blockers' refs, and the flat dependency view's refs. This is
 * the set Projects' derived state consults (#1206) — it replaces the
 * per-page rescan of tiers/goals/dependencies.
 */
export function workItemRefSet(section: OrchestrationSection): Set<RefKey> {
  const refs = new Set<RefKey>();
  for (const item of assembleWorkItems(section)) {
    if (item.ref) refs.add(item.ref);
    for (const dep of item.blockers) if (dep.ref) refs.add(dep.ref);
  }
  for (const dep of section.dependencies ?? []) {
    if (dep.ref) refs.add(dep.ref);
    if (dep.taskRef) refs.add(dep.taskRef);
  }
  return refs;
}

/** One-line-per-blocker text, the shape Today/Board render. */
export function formatBlockers(blockers: DependencyRef[], withWorker = false): string {
  return blockers
    .map((d) => `${d.task} (${d.status}${withWorker && d.workerLabel ? `, ${d.workerLabel}` : ""})`)
    .join("; ");
}
