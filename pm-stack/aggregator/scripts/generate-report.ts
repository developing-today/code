#!/usr/bin/env bun
// CLI contract: generate-report.ts --kind <daily|weekly|monthly|quarterly|annual> --period <explicit-identity> --as-of <ISO-8601> [--store <jsonl-path>]
// Parse the complete argv before choosing a store or performing any I/O.
import { defaultReportRecordsFile, type ReportRecordLocation } from "../src/report-records.ts";
import { reportPeriod, type ReportPeriodKind } from "../src/report-periods.ts";
import { generateReportFromStoredRecords } from "../src/report-composer.ts";
import { parseStoreBackend } from "../src/store-backend.ts";
import { createTursoDocumentRemote } from "../src/remote-stores.ts";

const USAGE = "usage: generate-report.ts --kind <daily|weekly|monthly|quarterly|annual> --period <explicit-identity> --as-of <ISO-8601> [--store <jsonl-path>]";

interface Command {
  kind: ReportPeriodKind;
  identity: string;
  asOf: string;
  store?: string;
}

const kinds = new Set<ReportPeriodKind>(["daily", "weekly", "monthly", "quarterly", "annual"]);
const valueFlags = new Set(["--kind", "--period", "--as-of", "--store"]);

export function parseArgs(args: string[]): Command {
  const values = new Map<string, string>();
  for (let index = 0; index < args.length; index++) {
    const flag = args[index]!;
    if (!valueFlags.has(flag)) throw new Error(`unknown argument: ${flag}`);
    if (values.has(flag)) throw new Error(`duplicate argument: ${flag}`);
    const value = args[index + 1];
    if (!value || value.startsWith("--")) throw new Error(`missing value for ${flag}`);
    values.set(flag, value);
    index++;
  }
  for (const required of ["--kind", "--period", "--as-of"]) {
    if (!values.has(required)) throw new Error(`missing required argument: ${required}`);
  }
  const kind = values.get("--kind") as ReportPeriodKind;
  if (!kinds.has(kind)) throw new Error(`invalid --kind: ${kind}`);
  const identity = values.get("--period")!;
  const asOf = values.get("--as-of")!;
  if (!Number.isFinite(Date.parse(asOf)) || new Date(asOf).toISOString() !== asOf) {
    throw new Error("--as-of must be a canonical ISO-8601 timestamp");
  }
  // Resolve now so bad/implicit period identities fail before any store I/O.
  reportPeriod(kind, identity);
  const store = values.get("--store")?.trim();
  if (values.has("--store") && !store) throw new Error("--store must not be empty");
  return { kind, identity, asOf, ...(store ? { store } : {}) };
}

async function main(args: string[]): Promise<void> {
  let command: Command;
  try {
    command = parseArgs(args);
  } catch (error) {
    console.error(`generate-report: ${error instanceof Error ? error.message : String(error)}`);
    console.error(USAGE);
    process.exitCode = 2;
    return;
  }
  const period = reportPeriod(command.kind, command.identity);
  const backend = parseStoreBackend();
  if (backend === "turso" && command.store) {
    console.error("generate-report: --store is only valid with PM_AGG_STORE_BACKEND=file");
    console.error(USAGE);
    process.exitCode = 2;
    return;
  }
  const location: ReportRecordLocation = backend === "turso"
    ? createTursoDocumentRemote()
    : command.store ?? defaultReportRecordsFile();
  const result = await generateReportFromStoredRecords(location, period, { asOf: command.asOf });
  console.log(JSON.stringify({ inserted: result.inserted, id: result.record.id, period: result.record.period.id, sourceCursor: result.record.sourceCursor }, null, 2));
}

if (import.meta.main) {
  main(Bun.argv.slice(2)).catch((error: unknown) => {
    console.error(`generate-report: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  });
}
