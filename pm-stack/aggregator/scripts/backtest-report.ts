#!/usr/bin/env bun
// Run the rolling-origin forecast backtest and print the calibration
// report (#3986):
//
//   bun run scripts/backtest-report.ts \
//     --snapshots data/forecast-snapshots.jsonl \
//     --outcomes outcomes.json [--as-of <iso>]
//
// --outcomes is a JSON file holding ForecastOutcome[]:
//   { subjectRef, completedAt (iso|null), recordedAt (iso) }
// One report is printed per model version in the store; aggregates
// never mix versions. The report's calibration gate is enforced in
// src/backtest.ts: below 30 resolved forecasts across 8 cutoffs the
// state is not_yet_calibrated, whatever the point metrics say.
import { readFile } from "node:fs/promises";
import { evaluateBacktest, loadSnapshots, type ForecastOutcome } from "../src/backtest.ts";

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

const snapshotsPath = arg("snapshots");
const outcomesPath = arg("outcomes");
if (!snapshotsPath || !outcomesPath) {
  console.error("usage: backtest-report.ts --snapshots <jsonl> --outcomes <json> [--as-of <iso>]");
  process.exit(2);
}
const snapshots = await loadSnapshots(snapshotsPath);
const outcomes = JSON.parse(await readFile(outcomesPath, "utf8")) as ForecastOutcome[];
const reports = evaluateBacktest({ snapshots, outcomes, asOf: arg("as-of") ?? new Date().toISOString() });
console.log(JSON.stringify(reports, null, 2));
