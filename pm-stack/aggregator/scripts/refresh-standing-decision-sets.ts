#!/usr/bin/env bun
// Refresh the standing open-questions decision set from the live
// pending-questions ledger (#13983 slice 2, remainder of #9354):
// the hourly cadence half of the seeding story. Slice 1
// (scripts/seed-standing-decision-sets.ts) bootstraps the set with a
// placeholder fixture; this script replaces that payload with the
// ledger's currently-open items (status asked/pending) through the
// #9360 refresh path — same setId, token preserved, resolutions
// carried by qid. Driven hourly by systemd/standing-refresh.timer;
// also runnable by hand with the same effect.
//
// A ledger that is unreadable, malformed, or has no open items is a
// hard failure (exit 1) and leaves the store untouched: the standing
// set keeps its last-known-good mirror rather than going empty.
//
// Usage:
//   bun run scripts/refresh-standing-decision-sets.ts [--ledger <path-to-pending-questions.json>] [--store <path-to-decisions.json>] [--source <name>]
//
// Defaults: --ledger resolves via PM_AGG_PENDING_QUESTIONS_FILE or
// ~/workspace/goals/pending-decisions-hourly-questions-for-drew/hidden_files/pending-questions.json;
// --store resolves like the aggregator (PM_AGG_DECISIONS_FILE or
// ~/.local/state/pm-aggregator/decisions.json); --source defaults to
// "decisions-ledger".
//
// Prints one JSON line to stdout:
//   { "setId": "...", "items": 3, "questions": 5, "openItems": 3, "skippedItems": 0, "truncated": false, "created": false, "refreshed": true }
// The per-set token is NEVER printed: it stays in the store file.
import { readFile } from "node:fs/promises";
import { defaultDecisionsFile } from "../src/decisions.ts";
import { STANDING_SEED_SOURCE } from "../src/standing-seed.ts";
import { defaultPendingQuestionsFile, refreshStandingDecisionSets } from "../src/standing-refresh.ts";

const USAGE =
  "usage: refresh-standing-decision-sets.ts [--ledger <path-to-pending-questions.json>] [--store <path-to-decisions.json>] [--source <name>]";

function usageError(message: string): never {
  console.error(`refresh-standing-decision-sets: ${message}`);
  console.error(USAGE);
  process.exit(2);
}

// Unknown or duplicated arguments are usage errors, never silently
// ignored — same discipline as scripts/post-alert.ts (#7289).
function rejectUnknownArgs(): void {
  const args = process.argv.slice(2);
  const seen = new Set<string>();
  for (let i = 0; i < args.length; i++) {
    const token = args[i] as string;
    if (token === "--ledger" || token === "--store" || token === "--source") {
      if (seen.has(token)) usageError(`duplicate ${token}`);
      seen.add(token);
      i++; // skip the value; arg() validates it
      continue;
    }
    usageError(`unknown argument ${JSON.stringify(token)}`);
  }
}
rejectUnknownArgs();

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  if (i < 0) return undefined;
  const value = process.argv[i + 1];
  if (value === undefined || value === "" || value.startsWith("--")) {
    usageError(`--${name} requires a value that is not a flag`);
  }
  return value;
}

async function main(): Promise<void> {
  const ledgerPath = arg("ledger") ?? defaultPendingQuestionsFile();
  const store = arg("store") ?? defaultDecisionsFile();
  const source = arg("source") ?? STANDING_SEED_SOURCE;
  let ledger: unknown;
  try {
    ledger = JSON.parse(await readFile(ledgerPath, "utf8"));
  } catch (err) {
    console.error(
      `refresh-standing-decision-sets: cannot read/parse ledger ${ledgerPath}: ${err instanceof Error ? err.message : err}`,
    );
    process.exit(1);
  }
  let result;
  try {
    result = await refreshStandingDecisionSets(store, ledger, source);
  } catch (err) {
    console.error(
      `refresh-standing-decision-sets: refresh failed: ${err instanceof Error ? err.message : err}`,
    );
    process.exit(1);
  }
  const questions = result.set.items.reduce((n, item) => n + item.questions.length, 0);
  console.log(
    JSON.stringify({
      setId: result.set.setId,
      items: result.set.items.length,
      questions,
      openItems: result.openItems,
      skippedItems: result.skippedItems,
      truncated: result.truncated,
      created: result.created,
      refreshed: result.refreshed,
    }),
  );
}

await main();
