#!/usr/bin/env bun
// Build the C4.3 day-type + spike-day SHADOW records over the
// aggregator snapshot store and a forecast product: per
// pool/window with a live v1 projection, the unadjusted wall,
// the day-type-adjusted wall for the admitted current cell, and
// the explicit spike-day verdict (#13259 criterion 2; C4 design
// §5). Shadow only — this command writes nothing to the store
// or the forecast product; its outputs are the report / JSON
// files named on the command line.
//
//   bun run scripts/quota-daytype-shadow.ts --store-dir <dir> \
//     --forecast <quota-forecast.json> [--report-out <md>] \
//     [--json-out <json>] [--as-of <iso>]
//
// (Committed as the package.json script `shadow:daytype-adjustments`.)
//
// The gatherer tick appends the same records to
// quota-daytype-shadow.jsonl beside the forecast product; this
// command is the inspectable, reproducible form over a fixed
// store copy (--as-of defaults to the forecast's generatedAt,
// never the wall clock).
//
// Argument contract (#3352): the whole command line is declared
// here — the five value options above, in `--opt value` or
// `--opt=value` form — and parsed strictly before any file is
// read or written. An unknown argument, a stray positional, or
// a present-but-valueless option is a usage error (exit 2)
// naming the offending argument.
import { readFile, writeFile } from "node:fs/promises";
import { SnapshotStore } from "../src/snapshots.ts";
import { buildDayTypeShadowRecords, renderDayTypeShadowReport } from "../src/quota-daytype-adjustment.ts";
import type { QuotaForecastFile } from "../src/quota-forecast.ts";

const VALUE_OPTIONS = ["--store-dir", "--forecast", "--report-out", "--json-out", "--as-of"] as const;
type ValueOption = (typeof VALUE_OPTIONS)[number];

function usageError(message: string): never {
  console.error(`quota-daytype-shadow: ${message}`);
  console.error(
    "usage: quota-daytype-shadow --store-dir <dir> --forecast <quota-forecast.json> [--report-out <md>] [--json-out <json>] [--as-of <iso>]",
  );
  process.exit(2);
}

function parseArgs(argv: string[]): Record<ValueOption, string | undefined> {
  const out: Record<ValueOption, string | undefined> = {
    "--store-dir": undefined,
    "--forecast": undefined,
    "--report-out": undefined,
    "--json-out": undefined,
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
  const forecastPath = args["--forecast"];
  if (!forecastPath) usageError("--forecast is required; refusing to run, no files read or written");
  let asOf: Date | undefined;
  if (args["--as-of"] !== undefined) {
    asOf = new Date(args["--as-of"]);
    if (!Number.isFinite(asOf.getTime())) usageError(`--as-of '${args["--as-of"]}' is not a parseable instant`);
  }

  const records = await new SnapshotStore(storeDir).readAll();
  if (!records.length) {
    console.error(`quota-daytype-shadow: no snapshot records in ${storeDir} — nothing to adjust`);
    process.exit(1);
  }
  const forecast = JSON.parse(await readFile(forecastPath, "utf8")) as QuotaForecastFile;
  const shadow = buildDayTypeShadowRecords(forecast, records, new Map(), asOf);
  const report = renderDayTypeShadowReport(shadow);

  if (args["--report-out"]) {
    await writeFile(args["--report-out"], report, "utf8");
  } else {
    console.log(report);
  }
  if (args["--json-out"]) {
    await writeFile(args["--json-out"], JSON.stringify(shadow, null, 2) + "\n", "utf8");
  }
  const adjusted = shadow.filter((r) => r.state === "shadow").length;
  console.error(
    `quota-daytype-shadow: ${records.length} records, ${shadow.length} shadow records (${adjusted} adjusted, ${shadow.length - adjusted} adjustment_unavailable)`,
  );
}

await main();
