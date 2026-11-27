#!/usr/bin/env bun
// Seed the standing open-questions decision set (#13983 slice 1,
// remainder of #9354): the offline half of the seeding story. Reads an
// optional payload JSON file (default: the built-in bootstrap fixture
// in src/standing-seed.ts — the same shape validateDecisionSet
// checks), validates it locally, and applies it through
// refreshDecisionSet — the #9360 refresh path — against a store file.
//
// The live half is scripts/submit-decision-set.ts --refresh, which
// POSTs the same shape to POST /api/decision-sets with refresh:true
// (needs the intake bearer token and a running aggregator). This
// script needs neither: it writes the store file directly, so it
// works for bootstrapping, tests, and environments without server
// access. The hourly job (slice 2, same issue) refreshes the same
// setId from the live pending-questions ledger on cadence.
//
// Usage:
//   bun run scripts/seed-standing-decision-sets.ts [--store <path-to-decisions.json>] [--payload <path-to-json>] [--source <name>]
//
// Defaults: --store resolves like the aggregator (PM_AGG_DECISIONS_FILE
// or ~/.local/state/pm-aggregator/decisions.json); --payload defaults
// to the built-in fixture; --source defaults to "decisions-ledger".
//
// Prints one JSON line to stdout:
//   { "setId": "...", "items": 1, "questions": 2, "created": true, "refreshed": false }
// Exactly one of created/refreshed is true, mirroring the submit API.
// The per-set token is NEVER printed: it stays in the store file,
// like scripts/raise-decision-set.ts (which withholds the page URL).
//
// Re-running the seed is idempotent: the existing set keeps its
// setId, token, createdAt and original source; resolutions carry over
// by qid. Do not use this for ordinary sets: refresh is for STANDING
// ledger-mirror sets only.
import { readFile } from "node:fs/promises";
import { defaultDecisionsFile, validateDecisionSet } from "../src/decisions.ts";
import { seedStandingDecisionSets, standingSeedInput, STANDING_SEED_SOURCE } from "../src/standing-seed.ts";

const USAGE =
  "usage: seed-standing-decision-sets.ts [--store <path-to-decisions.json>] [--payload <path-to-json>] [--source <name>]";

function usageError(message: string): never {
  console.error(`seed-standing-decision-sets: ${message}`);
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
    if (token === "--store" || token === "--payload" || token === "--source") {
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
  const store = arg("store") ?? defaultDecisionsFile();
  const payloadPath = arg("payload");
  const source = arg("source") ?? STANDING_SEED_SOURCE;
  let input = standingSeedInput();
  if (payloadPath) {
    let raw: unknown;
    try {
      raw = JSON.parse(await readFile(payloadPath, "utf8"));
    } catch (err) {
      console.error(
        `seed-standing-decision-sets: cannot read/parse payload: ${err instanceof Error ? err.message : err}`,
      );
      process.exit(1);
    }
    const envelope = raw && typeof raw === "object" && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
    const candidate = envelope.decisionSet !== undefined ? envelope.decisionSet : raw;
    const v = validateDecisionSet(candidate);
    if (!v.ok) {
      console.error(`seed-standing-decision-sets: payload invalid: ${v.error}`);
      process.exit(1);
    }
    input = v.value;
  }
  let result;
  try {
    result = await seedStandingDecisionSets(store, input, source);
  } catch (err) {
    console.error(`seed-standing-decision-sets: seed failed: ${err instanceof Error ? err.message : err}`);
    process.exit(1);
  }
  const questions = result.set.items.reduce((n, item) => n + item.questions.length, 0);
  console.log(
    JSON.stringify({
      setId: result.set.setId,
      items: result.set.items.length,
      questions,
      created: result.created,
      refreshed: result.refreshed,
    }),
  );
}

await main();
