// Shared default locations for the usage-snapshot pipeline's files
// (#4889). The stewardship goal workspaces exist in two home trees: the
// sandbox's ~/workspace/goals/... and the laptop's mirror under
// ~/hatch-personal/workspace/goals/... . Scripts deployed on the laptop
// (append-usage-snapshots, collect-fleet-feed, backfill-usage-snapshots)
// used to default only to the ~/workspace tree, which on the laptop is a
// stub with no quota-view.md: the stewardship append step died with
// ENOENT, and the collector silently drained a stub outbox instead of
// the real one in ~/.local/state/pm-aggregator.
//
// Explicit env overrides (QUOTA_VIEW_PATH, QUOTA_REGISTRY_PATH,
// USAGE_SNAPSHOT_OUTBOX_PATH, ...) always win. These helpers only pick
// the default when no override is set: prefer the hatch-personal mirror
// (and, for the snapshot outbox, the collector's state dir) when it is
// present on this host, and fall back to the historical ~/workspace
// defaults otherwise — so sandbox and CI behavior is unchanged.
import { existsSync } from "node:fs";
import { join } from "node:path";

const KEEP_FOUR_HIDDEN = "workspace/goals/keep-the-four-repos-healthy/hidden_files";
const STEWARDSHIP_VIEW =
  "workspace/goals/stewardship-commission-recurring-review-and-dispatch/hidden_files/visibility/quota-view.md";

/** Default keep-the-four-repos-healthy hidden_files dir for this host. */
export function defaultHiddenDir(home: string): string {
  const preferred = join(home, "hatch-personal", KEEP_FOUR_HIDDEN);
  return existsSync(preferred) ? preferred : join(home, KEEP_FOUR_HIDDEN);
}

/** Default stewardship quota-view.md for this host. */
export function defaultQuotaViewPath(home: string): string {
  const preferred = join(home, "hatch-personal", STEWARDSHIP_VIEW);
  return existsSync(preferred) ? preferred : join(home, STEWARDSHIP_VIEW);
}

/** Default usage-snapshot outbox: the fleet-feed collector's state dir
 *  when it exists, else the hidden_files copy. */
export function defaultUsageOutboxPath(home: string): string {
  const stateDir = join(home, ".local/state/pm-aggregator");
  if (existsSync(stateDir)) {
    return join(stateDir, "usage-snapshot-outbox.jsonl");
  }
  return join(defaultHiddenDir(home), "usage-snapshot-outbox.jsonl");
}
