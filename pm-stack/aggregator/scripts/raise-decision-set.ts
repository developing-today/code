#!/usr/bin/env bun
// Raise a decision SET on the aggregator (#8766): the producer half of
// the decisions page. Reads a payload JSON file holding the
// `decisionSet` extension shape ({ setId, title, items: [...] }) —
// built from the Muse-side decisions ledger by whoever is presenting
// a batch — validates it locally against the same validator the server
// runs, then posts it to the alert throat (POST /api/alerts) with the
// decisionSet extension. The aggregator stores the set, and the alert
// engine sends Drew ONE push for the whole set whose tap opens the
// decisions page.
//
// Usage:
//   bun run scripts/raise-decision-set.ts --payload <path-to-json>
//
// The intake bearer <redacted> is resolved BY NAME from Bitwarden Secrets
// Manager (`pm-aggregator.alerts-intake-token`, via src/secrets.ts) and
// is only ever sent to the aggregator as the Authorization header. The
// server's response carries the token-carrying page URL; this script
// deliberately does NOT print it (Drew reaches the page from the push;
// the URL is re-derivable from the store by anyone with laptop access).
// It prints only the set id and item/question counts.
//
// Env: ALERTS_INTAKE_URL (default http://127.0.0.1:4870).
//
// Re-raising the same setId is idempotent server-side: the existing
// set (and its token and resolutions) is returned unchanged.
import { readFile } from "node:fs/promises";
import { resolveBwsSecret } from "../src/secrets.ts";
import { validateDecisionSet } from "../src/decisions.ts";

const USAGE = "usage: raise-decision-set.ts --payload <path-to-json>";

function usageError(message: string): never {
  console.error(`raise-decision-set: ${message}`);
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
    if (token === "--payload") {
      if (seen.has(token)) usageError("duplicate --payload");
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
  const payloadPath = arg("payload");
  if (!payloadPath) {
    console.error(USAGE);
    process.exit(2);
  }
  let raw: unknown;
  try {
    raw = JSON.parse(await readFile(payloadPath, "utf8"));
  } catch (err) {
    console.error(`raise-decision-set: cannot read/parse payload: ${err instanceof Error ? err.message : err}`);
    process.exit(1);
  }
  const v = validateDecisionSet(raw);
  if (!v.ok) {
    console.error(`raise-decision-set: payload invalid: ${v.error}`);
    process.exit(1);
  }
  const set = v.value;
  const questionCount = set.items.reduce((n, i) => n + i.questions.length, 0);
  const token = await resolveBwsSecret("pm-aggregator.alerts-intake-token");
  if (!token) {
    console.error("raise-decision-set: intake token unavailable (bws secret pm-aggregator.alerts-intake-token)");
    process.exit(1);
  }
  const base = (process.env.ALERTS_INTAKE_URL ?? "http://127.0.0.1:4870").replace(/\/+$/, "");
  const res = await fetch(`${base}/api/alerts`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({
      source: "decisions-ledger",
      key: `decision-set:${set.setId}`,
      severity: "info",
      title: set.title,
      body: `${set.items.length} items, ${questionCount} questions — raised for the decisions page.`,
      decisionSet: set,
    }),
    signal: AbortSignal.timeout(30_000),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    console.error(`raise-decision-set: intake returned HTTP ${res.status}: ${text.slice(0, 300)}`);
    process.exit(1);
  }
  const out = (await res.json().catch(() => null)) as { decisionSet?: { setId?: string } } | null;
  if (!out?.decisionSet?.setId) {
    console.error("raise-decision-set: intake response carried no decisionSet (extension not applied)");
    process.exit(1);
  }
  console.log(
    `raised decision set ${out.decisionSet.setId}: ${set.items.length} items, ${questionCount} questions (page URL withheld — it rides the push)`,
  );
}

await main();
