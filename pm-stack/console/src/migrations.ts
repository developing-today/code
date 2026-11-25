// Running a store migration from the app (#14256).
//
// A migration is a run of one workflow, the way a deploy is a run of
// its workflow: the app starts it by dispatch (ops.ts `storeMigrate`,
// the `deploy()` pattern) and reads its standing from GitHub's own
// record of the workflow's runs (the `platform()` last-run pattern).
// The workflow names the store it migrates in its run name
// (`Migrate <store>`), so a run says which store it was for without
// the app holding any state of its own. The store's credential is
// used by the runner from the repository's secret store; it never
// reaches the app or the Worker.

/** The workflow that applies the shared-schema migrations. */
export const MIGRATION_WORKFLOW = "pm-store-migrate.yml";

/** The stand-in store a migration can be proved against in CI: the
 * workflow stands a disposable database up under this name, so it is
 * always a valid target beside the stores the list of record names. */
export const TEST_STORE = "test";

export interface MigrationRun {
  /** How it stands: once over, how it ended (a failure stays
   * "failure"); before that, where it is. */
  state: string;
  commit: string;
  at: string;
  url: string;
}

export interface StoreMigration {
  name: string;
  title: string;
  /** Its last migration run, or null when it has never had one. */
  run: MigrationRun | null;
}

interface RawRun {
  display_title?: unknown;
  head_sha?: unknown;
  status?: unknown;
  conclusion?: unknown;
  created_at?: unknown;
  html_url?: unknown;
}

/** The store a run was for, from its name (`Migrate <store>`), or
 * null when the run was not named by this workflow's dispatch. */
export function runStore(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const m = /^Migrate ([a-z0-9][a-z0-9-]{0,62})$/.exec(value);
  return m ? m[1] : null;
}

/**
 * Each store's last migration run, from the workflow's runs (newest
 * first, as GitHub lists them): the first run named for a store is
 * its last. A failed run keeps its "failure" state here, so the
 * screen can say it failed and link to it; it is never folded into a
 * success or dropped. Stores with no run get null.
 */
export function lastMigrationRuns(
  runs: unknown,
  stores: { name: string; title: string }[],
): StoreMigration[] {
  const list = Array.isArray(runs) ? (runs as RawRun[]) : [];
  const byStore = new Map<string, MigrationRun>();
  for (const r of list) {
    const store = runStore(r?.display_title);
    if (!store || byStore.has(store)) continue;
    if (typeof r.head_sha !== "string" || typeof r.created_at !== "string" || typeof r.html_url !== "string") continue;
    byStore.set(store, {
      state: r.status === "completed" ? (typeof r.conclusion === "string" ? r.conclusion : "unknown") : String(r.status ?? "unknown"),
      commit: r.head_sha.slice(0, 12),
      at: r.created_at,
      url: r.html_url,
    });
  }
  return stores.map((s) => ({ name: s.name, title: s.title, run: byStore.get(s.name) ?? null }));
}
