import type { ProjectPoolItem, RefKey } from "./model.ts";

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

/** Build rollups for epics in the pool, using only explicit epicRef edges. */
export function buildEpicRollups(pool: readonly ProjectPoolItem[]): Map<RefKey, EpicRollup> {
  const byRef = new Map(pool.map((item) => [item.ref, item]));
  const childrenByEpic = new Map<RefKey, ProjectPoolItem[]>();
  for (const item of pool) {
    if (!item.epicRef) continue;
    const children = childrenByEpic.get(item.epicRef) ?? [];
    children.push(item);
    childrenByEpic.set(item.epicRef, children);
  }

  const rollups = new Map<RefKey, EpicRollup>();
  for (const epic of pool) {
    if (epic.kind !== "Epic") continue;
    const children = childrenByEpic.get(epic.ref) ?? [];
    const statuses = new Map<string, number>();
    let done = 0;
    let blocked = 0;
    for (const child of children) {
      const status = child.status?.trim() || "No status";
      statuses.set(status, (statuses.get(status) ?? 0) + 1);
      if (child.state === "closed" || child.state === "merged" || status === "Done") done += 1;
      if (
        child.state === "open" &&
        (child.blockedBy ?? []).some((ref) => {
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
