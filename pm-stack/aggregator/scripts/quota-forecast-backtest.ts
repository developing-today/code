#!/usr/bin/env bun
// Backtest the Stage-3 v1 quota forecast over the aggregator
// snapshot store and publish the forecast accuracy ledger
// (#12696, slice S2 of epic #12694; design
// doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3 §2/§7).
//
//   bun run scripts/quota-forecast-backtest.ts --store-dir <dir> \
//     [--report-out <md>] [--ledger-out <json>] [--facts-out <jsonl>] \
//     [--as-of <iso>]
//
// (Committed as the package.json script `backtest:quota-forecast`.)
//
// The store is read through SnapshotStore.readAll — read-only; this
// script never writes to the store dir. Outputs are the human
// report (stdout unless --report-out), the accuracy ledger JSON
// (the baselines the S7 weekly review reads), and optionally the
// scored-forecast facts JSONL (the ledger's accumulation unit).
// --as-of defaults to the store's last reading, never the wall
// clock, so a run over a fixed store copy is reproducible.
//
// Argument contract (#3352): the whole command line is declared
// here — the five value options above, in `--opt value` or
// `--opt=value` form — and parsed strictly before any file is read
// or written. An unknown argument, a stray positional, or a
// present-but-valueless option is a usage error (exit 2) naming the
// offending argument.
import { writeFile } from "node:fs/promises";
import { SnapshotStore } from "../src/snapshots.ts";
import { buildForecastAccuracyLedger, renderBacktestReport } from "../src/quota-forecast-accuracy.ts";

const VALUE_OPTIONS = ["--store-dir", "--report-out", "--ledger-out", "--facts-out", "--as-of"] as const;
type ValueOption = (typeof VALUE_OPTIONS)[number];

function usageError(message: string): never {
  console.error(`quota-forecast-backtest: ${message}`);
  console.error(
    "usage: quota-forecast-backtest --store-dir <dir> [--report-out <md>] [--ledger-out <json>] [--facts-out <jsonl>] [--as-of <iso>]",
  );
  process.exit(2);
}

function parseArgs(argv: string[]): Record<ValueOption, string | undefined> {
  const out: Record<ValueOption, string | undefined> = {
    "--store-dir": undefined,
    "--report-out": undefined,
    "--ledger-out": undefined,
    "--facts-out": undefined,
    "--as-of": undefined,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i] as string;
    const eq = arg.indexOf("=");
    const name = (eq >= 0 ? arg.slice(0, eq) : arg) as ValueOption;
    if (!VALUE_OPTIONS.includes(name)) {
      usageError(`unknown argument '${arg}'; refusing to run, no files read or written`);
    }
    let value: string | undefined;
    if (eq >= 0) {
      value = arg.slice(eq + 1);
    } else {
      value = argv[i + 1];
      if (value !== undefined) i++;
    }
    if (value === undefined || value.trim() === "" || value.startsWith("--")) {
      usageError(`option '${name}' requires a non-empty value; refusing to run, no files read or written`);
    }
    out[name] = value;
  }
  return out;
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  const storeDir = args["--store-dir"];
  if (!storeDir) usageError("--store-dir is required; refusing to run, no files read or written");
  let asOfMs: number | undefined;
  if (args["--as-of"] !== undefined) {
    asOfMs = Date.parse(args["--as-of"]);
    if (!Number.isFinite(asOfMs)) usageError(`--as-of '${args["--as-of"]}' is not a parseable instant`);
  }

  const records = await new SnapshotStore(storeDir).readAll();
  if (!records.length) {
    console.error(`quota-forecast-backtest: no snapshot records in ${storeDir} — nothing to score`);
    process.exit(1);
  }
  const { ledger, facts } = buildForecastAccuracyLedger(records, { asOfMs });
  const report = renderBacktestReport(ledger);

  if (args["--report-out"]) {
    await writeFile(args["--report-out"], report, "utf8");
  } else {
    console.log(report);
  }
  if (args["--ledger-out"]) {
    await writeFile(args["--ledger-out"], JSON.stringify(ledger, null, 2) + "\n", "utf8");
  }
  if (args["--facts-out"]) {
    await writeFile(args["--facts-out"], facts.map((f) => JSON.stringify(f)).join("\n") + "\n", "utf8");
  }
  console.error(
    `quota-forecast-backtest: ${ledger.store.records} records, ${facts.length} scored forecasts, ` +
      `${ledger.pools.filter((p) => p.rollup.scoreable).length}/${ledger.pools.length} pools scoreable`,
  );
}

await main();
