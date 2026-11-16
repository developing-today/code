#!/usr/bin/env bun
// Backfill the four aggregator stores that #6815 moved from laptop-local
// files to Supabase muse-dev2:
//
//   actions.json              -> public.agg_actions            (20261105_0001)
//   activity-ingestion.json   -> public.agg_activity_ingestion + *_gaps
//   snapshots/usage-*.jsonl   -> public.agg_usage_snapshots
//   aa-last-good.json         -> public.agg_store_documents ("aa-last-good")
//
// The script is a read-only reader of the file stores and an idempotent
// writer of the remote ones: every remote write is an upsert on the same
// key the store itself uses (action id, source, gap id, snapshot key,
// document name), so running it twice appends/overwrites nothing new.
//
// Cutover order (see pm-stack/aggregator/README.md, "Store backend"):
//   1. apply shared-schema/migrations/20261105_0001_action_event_store
//   2. apply shared-schema/migrations/20261116_0001_aggregator_remote_stores
//   3. run this script (--dry-run first to see the counts)
//   4. set PM_AGG_STORE_BACKEND=supabase and deploy
// The file stores stay untouched, so the rollback target is intact.
//
// Usage: bun run scripts/migrate-stores-to-supabase.ts [--dry-run] [--help]
// Env:   PM_AGG_STATE_DIR      default ~/.local/state/pm-aggregator
//        PM_AGG_SNAPSHOTS_DIR  default <state-dir>/snapshots
//        SUPABASE_PROJECT_REF / BWS secrets resolve as everywhere else
//          (src/supabase.ts).
// Exit:  0 migrated (or dry run), 2 usage error, 1 remote/file failure.
import { homedir } from "node:os";
import { join } from "node:path";
import { ActionsStore } from "../src/actions.ts";
import { IngestionStore } from "../src/activity.ts";
import { AaLastGoodStore } from "../src/aa-last-good.ts";
import { AA_LAST_GOOD_DOCUMENT } from "../src/store-backend.ts";
import { SnapshotStore } from "../src/snapshots.ts";
import { createSupabaseRemotes } from "../src/remote-stores.ts";

const USAGE = `Usage: bun run scripts/migrate-stores-to-supabase.ts [--dry-run] [--help]

Reads the four file stores under the aggregator state dir and upserts
them into Supabase (muse-dev2). Idempotent; never deletes file state.

  --dry-run   read and validate the files, print counts, write nothing
  --help      print this help

Env:
  PM_AGG_STATE_DIR      default ~/.local/state/pm-aggregator
  PM_AGG_SNAPSHOTS_DIR  default <state-dir>/snapshots`;

const KNOWN_FLAGS = new Set(["--dry-run", "--help"]);

function usageError(message: string): never {
  console.error(`migrate-stores-to-supabase: ${message}`);
  console.error(USAGE);
  process.exit(2);
}

// Strict argument contract (#3352): every argv must be declared, and a
// malformed command line exits 2 before any file or network access.
const args = process.argv.slice(2);
let dryRun = false;
for (const arg of args) {
  if (!KNOWN_FLAGS.has(arg)) usageError(`unknown argument ${JSON.stringify(arg)}`);
  if (arg === "--help") {
    console.log(USAGE);
    process.exit(0);
  }
  if (arg === "--dry-run") dryRun = true;
}

const stateDir = (process.env.PM_AGG_STATE_DIR ?? join(homedir(), ".local", "state", "pm-aggregator")).trim();
if (stateDir === "") usageError("PM_AGG_STATE_DIR is set but empty");
const snapshotsDir = (process.env.PM_AGG_SNAPSHOTS_DIR ?? join(stateDir, "snapshots")).trim();
if (snapshotsDir === "") usageError("PM_AGG_SNAPSHOTS_DIR is set but empty");

const actionsFile = join(stateDir, "actions.json");
const activityFile = join(stateDir, "activity-ingestion.json");
const aaLastGoodFile = join(stateDir, "aa-last-good.json");

async function main(): Promise<void> {
  // File stores are only readers here; each one already validates its
  // own shape and treats a missing file as an empty start.
  const actionsStore = new ActionsStore(actionsFile);
  await actionsStore.load();
  const actions = await actionsStore.all();

  const activityStore = new IngestionStore(activityFile);
  await activityStore.load();
  const activity = await activityStore.documentForExport();

  const snapshots = await new SnapshotStore(snapshotsDir).readAll();
  const aaLastGood = await new AaLastGoodStore(aaLastGoodFile).read();

  console.error(
    `migrate-stores-to-supabase: read ${actions.length} action(s), ` +
      `${Object.keys(activity.checkpoints).length} activity source(s), ${activity.gaps.length} gap(s), ` +
      `${snapshots.length} usage snapshot(s), aa-last-good ${aaLastGood === null ? "absent" : "present"}`,
  );

  if (dryRun) {
    console.error("migrate-stores-to-supabase: --dry-run, nothing written");
    return;
  }

  const remotes = createSupabaseRemotes(undefined, "pm-aggregator-migrate");
  await remotes.actions.saveAll(actions);
  await remotes.activity.save(activity);
  const inserted = await remotes.usage.append(snapshots);
  if (aaLastGood !== null) {
    await remotes.aaLastGood.save(AA_LAST_GOOD_DOCUMENT, 1, aaLastGood);
  }
  console.error(
    `migrate-stores-to-supabase: wrote actions=${actions.length}, ` +
      `activitySources=${Object.keys(activity.checkpoints).length}, gaps=${activity.gaps.length}, ` +
      `snapshotsInserted=${inserted}/${snapshots.length}, aaLastGood=${aaLastGood === null ? "skipped" : "written"}`,
  );
}

try {
  await main();
} catch (err) {
  console.error(`migrate-stores-to-supabase: failed (${err instanceof Error ? err.message : String(err)})`);
  process.exit(1);
}
