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
// Env: PM_AGG_SNAPSHOTS_DIR (default ~/.local/state/pm-aggregator/snapshots).
import { homedir } from "node:os";
import { join } from "node:path";
import { SnapshotStore } from "../src/snapshots.ts";

const dir =
  process.env.PM_AGG_SNAPSHOTS_DIR ?? join(homedir(), ".local/state/pm-aggregator/snapshots");
const apply = process.argv.includes("--apply");
const monthsArg = process.argv.indexOf("--months");
const retentionMonths = monthsArg >= 0 ? Number(process.argv[monthsArg + 1]) : 13;
if (!Number.isInteger(retentionMonths) || retentionMonths < 0) {
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
