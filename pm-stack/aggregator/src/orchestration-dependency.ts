// Wire vocabulary shared by orchestration publishers, the collector gate,
// and dashboard work-item derivation. "blocked" describes a prerequisite
// that is itself waiting, not an unknown or merely queued prerequisite.
const dependencyStatuses = ["in-progress", "queued", "stalled", "blocked", "done", "unknown"] as const;

export type DependencyStatus = (typeof dependencyStatuses)[number];

export function isDependencyStatus(value: unknown): value is DependencyStatus {
  return dependencyStatuses.some((status) => status === value);
}
