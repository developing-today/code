#!/usr/bin/env bun
// Bitwarden Secrets Manager count-only collector (#1111; hub design
// §2.2, weekly cadence). Runs `bws secret list` + `bws project list`
// and records ONLY the two array lengths as a dated read in the
// tracker's Bitwarden section (quota-view.md), in the phrasing
// saas-extract's bitwardenMetrics parses — so the read flows to the
// /quota card and, via append-usage-snapshots, the §4 snapshot store.
// Secret/project names, keys, and values are never read into this
// process's outputs: the list JSON is counted by length in
// src/bitwarden-counts.ts and discarded, and nothing but the two
// counts and the read date is ever printed.
//
// The bws access token comes from the caller's environment
// (BWS_ACCESS_TOKEN, resolved by name from the secret store by
// whatever launches this) and is passed through to the child process
// env untouched; it is never printed or written anywhere here.
//
// Command line (operator script argument contract, #3352 — the whole
// line is declared here and parsed strictly BEFORE any subprocess or
// file write; anything unaccounted for is a usage error, exit 2):
//   bun run scripts/collect-bitwarden-counts.ts [--date YYYY-MM-DD]
//       [--view <path>] [--print]
//   --date   read date (default: today, UTC)
//   --view   tracker view path (default: $QUOTA_VIEW_PATH, else the
//            standard quota-view location from src/default-paths.ts)
//   --print  dry run: print the updated bullets + the snapshot records
//            the read yields, write nothing
// Env: QUOTA_VIEW_PATH, BWS_BIN (bws executable override).
import { execFileSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { applyBitwardenRead, collectBitwardenCounts } from "../src/bitwarden-counts.ts";
import { defaultQuotaViewPath } from "../src/default-paths.ts";
import { parseIsoDate } from "../src/validate.ts";
import { extractSaasServices } from "../src/saas-extract.ts";
import { recordsFromSaasServices } from "../src/snapshot-records.ts";

function usageError(message: string): never {
  console.error(`collect-bitwarden-counts: ${message}; refusing to run, no bws call made, no file written`);
  process.exit(2);
}

function parseArgs(argv: string[]): { date: string; view: string; print: boolean } {
  let date: string | null = null;
  let view: string | null = null;
  let print = false;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i]!;
    if (arg === "--print") {
      print = true;
    } else if (arg === "--date" || arg === "--view") {
      const value: string | undefined = argv[i + 1];
      if (value === undefined || value.trim() === "" || value.startsWith("-")) {
        usageError(`${arg} requires a non-empty value`);
      }
      if (arg === "--date") date = value;
      else view = value;
      i++;
    } else {
      usageError(`unrecognized argument ${JSON.stringify(arg)}`);
    }
  }
  const today = new Date().toISOString().slice(0, 10);
  const resolvedDate = date ?? today;
  if (!parseIsoDate(resolvedDate)) {
    usageError(`--date ${JSON.stringify(resolvedDate)} is not a real ISO calendar date`);
  }
  return {
    date: resolvedDate,
    view: view ?? process.env.QUOTA_VIEW_PATH ?? defaultQuotaViewPath(homedir()),
    print,
  };
}

const { date, view: viewPath, print } = parseArgs(process.argv.slice(2));

const bwsBin = process.env.BWS_BIN ?? "bws";
const counts = collectBitwardenCounts((args) =>
  execFileSync(bwsBin, args, { encoding: "utf8", env: process.env }),
);

const view = await readFile(viewPath, "utf8");
const updated = applyBitwardenRead(view, counts, date);

if (print) {
  const bw = extractSaasServices(updated).find((s) => s.id === "bitwarden");
  const records = recordsFromSaasServices(bw ? [bw] : [], "collect-bitwarden-counts");
  console.log(
    JSON.stringify({ date, counts, records }, null, 2),
  );
} else {
  await writeFile(viewPath, updated);
  console.log(
    `collect-bitwarden-counts: recorded ${counts.secrets} secrets, ${counts.projects} projects (bws count-only read ${date}) in ${viewPath}`,
  );
}
