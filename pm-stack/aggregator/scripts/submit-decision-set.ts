#!/usr/bin/env bun
// Submit a decision SET through the v2 submit API (#9335): the
// programmatic producer for Decision Sets v2. Reads a payload JSON
// file holding the set shape ({ setId?, title, items: [...] } — the
// same shape validateDecisionSet checks, v2 fields included) or the
// { decisionSet: {...} } envelope, validates it locally against the
// same validator the server runs, then POSTs it to
// POST /api/decision-sets with the intake bearer token.
//
// Unlike scripts/raise-decision-set.ts (which deliberately withholds
// the page URL — Drew reaches that page from the push), this script
// RETURNS the result to the authenticated submitter on stdout as one
// JSON line: { "setId": "...", "pageUrl": "...", "created": true }.
// The setId is a UUID minted by the server (or here, for local
// validation) when the payload omits it. The token-carrying pageUrl
// is printed ONLY here, to the submitter's own stdout: it is never
// written to the repo, a log file, or an issue. Treat it like the
// credential it is.
//
// Usage:
//   bun run scripts/submit-decision-set.ts --payload <path-to-json>
//
// The intake bearer token is resolved BY NAME from Bitwarden Secrets
// Manager (`pm-aggregator.alerts-intake-token`, via src/secrets.ts)
// and is only ever sent to the aggregator as the Authorization header.
//
// Env: ALERTS_INTAKE_URL (default http://127.0.0.1:4870).
//
// Re-submitting the same setId is idempotent server-side: the existing
// set (and its token and resolutions) is returned unchanged with
// "created": false, and no second push is sent.
import { readFile } from "node:fs/promises";
import { resolveBwsSecret } from "../src/secrets.ts";
import { validateDecisionSet, withGeneratedSetId } from "../src/decisions.ts";

const USAGE = "usage: submit-decision-set.ts --payload <path-to-json>";

function usageError(message: string): never {
  console.error(`submit-decision-set: ${message}`);
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
    console.error(`submit-decision-set: cannot read/parse payload: ${err instanceof Error ? err.message : err}`);
    process.exit(1);
  }
  const prepared = withGeneratedSetId(raw);
  const envelope =
    prepared && typeof prepared === "object" && !Array.isArray(prepared) ? (prepared as Record<string, unknown>) : {};
  const candidate = envelope.decisionSet !== undefined ? envelope.decisionSet : prepared;
  const v = validateDecisionSet(candidate);
  if (!v.ok) {
    console.error(`submit-decision-set: payload invalid: ${v.error}`);
    process.exit(1);
  }
  const token = await resolveBwsSecret("pm-aggregator.alerts-intake-token");
  if (!token) {
    console.error("submit-decision-set: intake token unavailable (bws secret pm-aggregator.alerts-intake-token)");
    process.exit(1);
  }
  const base = (process.env.ALERTS_INTAKE_URL ?? "http://127.0.0.1:4870").replace(/\/+$/, "");
  const res = await fetch(`${base}/api/decision-sets`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ decisionSet: v.value, source: "decisions-ledger" }),
    signal: AbortSignal.timeout(30_000),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    console.error(`submit-decision-set: submit API returned HTTP ${res.status}: ${text.slice(0, 300)}`);
    process.exit(1);
  }
  const out = (await res.json().catch(() => null)) as { setId?: string; pageUrl?: string; created?: boolean } | null;
  if (!out?.setId || !out?.pageUrl) {
    console.error("submit-decision-set: submit API response carried no setId/pageUrl");
    process.exit(1);
  }
  // The one place the token page URL is printed: the authenticated
  // submitter's own stdout, as the API contract promises.
  console.log(JSON.stringify({ setId: out.setId, pageUrl: out.pageUrl, created: out.created ?? false }));
}

await main();
