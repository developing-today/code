// Store migrations, started from the app (#14256).
//
// The shared-schema migrations run in one workflow, pm-store-migrate,
// which alone reads a store's token (by secret name, in the runner
// only). This module is the console side: it validates which store to
// migrate, builds the workflow's dispatch inputs (a store name and its
// confirmation, never a token), and reads the workflow's runs into the
// last run per store, so the Stores section can show when each store
// was last migrated, from which commit, and whether it failed.

import { OpError } from "./errors.ts";

export const STORE_MIGRATE_WORKFLOW = "pm-store-migrate.yml";

/** The workflow's own choices, in its order. `stand-in` is the CI
 * proof against a throwaway store; the rest are real stores, named as
 * in pm-stack/platform/stores.json. */
export const MIGRATE_TARGETS = [
  "stand-in",
  "fleet-queue",
  "fleet-platform",
  "fleet-mirror",
  "voice-line-ledger",
] as const;
export type MigrateTarget = (typeof MIGRATE_TARGETS)[number];

/** The prefix of every migration run's title: the workflow names each
 * run `pm-store-migrate <store>`, which is how a run is matched to its
 * store (GitHub's run list does not return a run's inputs). */
export const MIGRATE_RUN_PREFIX = "pm-store-migrate ";

function isMigrateTarget(value: unknown): value is MigrateTarget {
  return MIGRATE_TARGETS.some((target) => target === value);
}

export interface StoreMigrateRequest {
  store: MigrateTarget;
  /** The store's name repeated, for a real store; empty for the
   * stand-in. The workflow refuses a real store without it, and so
   * does this module, before anything is dispatched. */
  confirm: string;
}

/** Validate console arguments into one migration run. A real store must
 * have its name repeated in `confirm_store`, as the workflow requires:
 * a mistyped confirmation is refused here rather than started and
 * failed. */
export function storeMigrateRequest(args: Record<string, unknown>): StoreMigrateRequest {
  const store = args.store;
  if (!isMigrateTarget(store)) {
    throw new OpError(400, `store: one of ${MIGRATE_TARGETS.join(", ")}`);
  }
  if (store === "stand-in") return { store, confirm: "" };
  if (args.confirm_store !== store) {
    throw new OpError(400, `confirm_store: repeat the store name (${store}) to migrate a real store`);
  }
  return { store, confirm: store };
}

/** The workflow's dispatch inputs: text only, as GitHub takes them. */
export function storeMigrateInputs(req: StoreMigrateRequest): Record<string, string> {
  return req.confirm ? { store: req.store, confirm_store: req.confirm } : { store: req.store };
}

export interface MigrationRun {
  state: string;
  /** True only when the run finished and did not succeed: the caller
   * must show it as a failure. */
  failed: boolean;
  at: string;
  commit: string;
  url: string;
}

export interface GitHubRun {
  display_title?: string;
  name?: string;
  status: string;
  conclusion: string | null;
  created_at: string;
  head_sha: string;
  html_url: string;
}

/** One run as the Stores section says it: while it runs, where it is;
 * once over, how it ended, with a failure kept a failure. */
export function migrationRun(run: GitHubRun): MigrationRun {
  const state = run.status === "completed" ? (run.conclusion ?? "unknown") : run.status;
  return {
    state,
    failed: run.status === "completed" && state !== "success",
    at: run.created_at,
    commit: run.head_sha.slice(0, 12),
    url: run.html_url,
  };
}

/** The newest run per store, from GitHub's run list (newest first).
 * Runs whose title names no known store (those started before the
 * workflow named its runs) are passed over, not guessed at. */
export function lastMigrationRuns(runs: GitHubRun[]): Record<string, MigrationRun> {
  const out: Record<string, MigrationRun> = {};
  for (const run of runs) {
    const title = run.display_title ?? "";
    if (!title.startsWith(MIGRATE_RUN_PREFIX)) continue;
    const store = title.slice(MIGRATE_RUN_PREFIX.length).trim();
    if (!isMigrateTarget(store) || Object.hasOwn(out, store)) continue;
    out[store] = migrationRun(run);
  }
  return out;
}
