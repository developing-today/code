#!/usr/bin/env bun
// Re-measure day-type sample sufficiency over the aggregator
// snapshot store: per pool/window/day-type cell, how many
// independent samples the store holds against the C4 design §5
// gate (≥8), and which cells are admitted vs adjustment_unavailable
// (#13259, slice C4.3 of #9572).
//
//   bun run scripts/quota-daytype-sufficiency.ts --store-dir <dir> \
//     [--report-out <md>] [--json-out <json>] [--as-of <iso>]
//
// (Committed as the package.json script `measure:daytype-sufficiency`.)
//
// The store is read through SnapshotStore.readAll — read-only; this
// script never writes to the store dir. Outputs are the human
// report (stdout unless --report-out) and optionally the full
// per-cell measurement as JSON. --as-of defaults to the store's
// last reading, never the wall clock, so a run over a fixed store
// copy is reproducible.
//
// Argument contract (#3352): the whole command line is declared
// here — the four value options above, in `--opt value` or
// `--opt=value` form — and parsed strictly before any file is read
// or written. An unknown argument, a stray positional, or a
// present-but-valueless option is a usage error (exit 2) naming
// the offending argument.
import { writeFile } from "node:fs/promises";
import { SnapshotStore } from "../src/snapshots.ts";
import {
  measureDayTypeSufficiency,
  renderDayTypeSufficiencyReport,
} from "../src/quota-daytype-sufficiency.ts";

const VALUE_OPTIONS = ["--store-dir", "--report-out", "--json-out", "--as-of"] as const;
type ValueOption = (typeof VALUE_OPTIONS)[number];

function usageError(message: string): never {
  console.error(`quota-daytype-sufficiency: ${message}`);
  console.error(
    "usage: quota-daytype-sufficiency --store-dir <dir> [--report-out <md>] [--json-out <json>] [--as-of <iso>]",
  );
  process.exit(2);
}

function parseArgs(argv: string[]): Record<ValueOption, string | undefined> {
  const out: Record<ValueOption, string | undefined> = {
    "--store-dir": undefined,
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
  let asOfMs: number | undefined;
  if (args["--as-of"] !== undefined) {
    asOfMs = Date.parse(args["--as-of"]);
    if (!Number.isFinite(asOfMs)) usageError(`--as-of '${args["--as-of"]}' is not a parseable instant`);
  }

  const records = await new SnapshotStore(storeDir).readAll();
  if (!records.length) {
    console.error(`quota-daytype-sufficiency: no snapshot records in ${storeDir} — nothing to measure`);
    process.exit(1);
  }
  const measurement = measureDayTypeSufficiency(records, { asOfMs });
  const report = renderDayTypeSufficiencyReport(measurement);

  if (args["--report-out"]) {
    await writeFile(args["--report-out"], report, "utf8");
  } else {
    console.log(report);
  }
  if (args["--json-out"]) {
    await writeFile(args["--json-out"], JSON.stringify(measurement, null, 2) + "\n", "utf8");
  }
  const cells = measurement.poolWindows.flatMap((pw) => pw.cells);
  const admitted = cells.filter((c) => c.state === "admitted").length;
  console.error(
    `quota-daytype-sufficiency: ${measurement.store.records} records, ` +
      `${measurement.poolWindows.length} pool/windows, ${admitted}/${cells.length} cells admitted`,
  );
}

await main();
