#!/usr/bin/env bun
// Compact old usage snapshot months into rollups (hub design §6,
// #1117): raw monthly files are kept 13 months, then replaced by a
// `usage-YYYY-MM.rollup.jsonl` holding min/max/last per
// service+metric, kept indefinitely. SnapshotStore.readAll expands
// rollups, so readers need no changes.
//
// The default mode is --dry-run: it prints the per-service
// before/after report and changes nothing. Pass --apply to perform
// the compaction. Either way the full report is printed as JSON on
// stdout (attach it to #1117 before the first real run).
//
// Usage: bun run scripts/compact-usage-snapshots.ts [--apply] [--months <n>]
// Env: PM_AGG_SNAPSHOTS_DIR (default: the aggregator package's
// snapshots/ directory — the same store default as src/config.ts,
// backfill-usage-snapshots.ts, and the quota gatherer, so running
// this script in the standard installation compacts the store the
// aggregator actually reads and writes).
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { SnapshotStore } from "../src/snapshots.ts";

const PKG = join(dirname(fileURLToPath(import.meta.url)), "..");
const dir = process.env.PM_AGG_SNAPSHOTS_DIR ?? join(PKG, "snapshots");
// Reject unknown or misspelled arguments BEFORE the store is read or
// written (#6967): a typo'd --month silently fell back to the default
// 13-month retention, so an --apply run compacted (and deleted) raw
// months a longer requested retention meant to keep, exiting 0. The
// token after --months is its value slot: it is consumed here and
// judged by the --months validation below (#3285), never re-interpreted
// as a flag. Mirrors rejectUnknownArgs in append-usage-snapshots.ts
// (#6961).
function rejectUnknownArgs(): void {
  const args = process.argv.slice(2);
  for (let i = 0; i < args.length; i++) {
    const arg = args[i] as string;
    if (arg === "--apply") continue;
    if (arg === "--months") {
      i++;
      continue;
    }
    console.error(
      "compact-usage-snapshots: unknown argument '" + arg + "' " +
        "(usage: [--apply] [--months <n>]); refusing to run, store untouched",
    );
    process.exit(2);
  }
}
rejectUnknownArgs();

const apply = process.argv.includes("--apply");
const monthsArg = process.argv.indexOf("--months");
const retentionMonths = monthsArg >= 0 ? Number(process.argv[monthsArg + 1]) : 13;
if (!Number.isInteger(retentionMonths) || retentionMonths < 1) {
  console.error(`compact-usage-snapshots: invalid --months '${process.argv[monthsArg + 1]}'`);
  process.exit(2);
}

const report = await new SnapshotStore(dir).compact({ dryRun: !apply, retentionMonths });
console.log(JSON.stringify({ storeDir: dir, ...report }, null, 2));
if (!report.months.length) {
  console.error(
    `compact-usage-snapshots: no months at or before the ${report.cutoffMonth} cutoff — nothing to do (${apply ? "apply" : "dry-run"})`,
  );
} else {
  for (const m of report.months) {
    console.error(
      `compact-usage-snapshots: ${m.month}: ${m.recordsBefore} records -> ${m.recordsAfter} rollups (${m.dryRun ? "dry-run, unchanged" : "compacted"})`,
    );
  }
}
