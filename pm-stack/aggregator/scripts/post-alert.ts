#!/usr/bin/env bun
// Post an incident to the alert throat (#3676): the one CLI every
// sentinel uses instead of inventing its own alert path.
//
// Usage:
//   bun run scripts/post-alert.ts --source globalping-watch \
//     --key "zettel.drewrypope.com" --severity critical \
//     --title "zettel.drewrypope.com down" --body "probes saw 503"
//
// The intake bearer token is resolved BY NAME from Bitwarden Secrets
// Manager (`pm-aggregator.alerts-intake-token`, via src/secrets.ts —
// the laptop's bws flow) and is only ever sent to the aggregator as
// the Authorization header. It is never printed. Dedupe is the
// server's job: re-posting the same (source, key) bumps the existing
// open incident instead of opening (or pushing) a second one.
//
// Env: ALERTS_INTAKE_URL (default http://127.0.0.1:4870).
//
// Every flag takes a space-separated value. A value that begins with
// "--" is NOT expressible in that form: a flag followed by another
// flag (or by nothing) is a usage error, exit 2, before anything is
// resolved or posted (#3723).
import { resolveBwsSecret } from "../src/secrets.ts";

const USAGE = "usage: post-alert.ts --source <s> --key <k> --title <t> [--severity critical|warning|info] [--body <b>]";

function usageError(message: string): never {
  console.error(`post-alert: ${message}`);
  console.error(USAGE);
  process.exit(2);
}

// A flag's value is the next argv token — but never another flag and
// never nothing (#3723). The unconditional read used to swallow the
// next flag as a value: `--title --body 503` posted a critical
// incident literally titled "--body", and a trailing `--severity`
// silently fell back to the critical default. Same settled guard as
// collect-github-billing.ts / record-claude-launch.ts.
// Unknown or duplicated arguments are usage errors, never silently
// ignored (#7289): argv is probed token-by-token by arg() below, so a
// typo'd --severty fell through to the critical severity default and
// posted a warning incident as critical, a joined --severity=warning
// did the same, and a typo'd --bodi silently dropped the body — all
// exit 0. Mirrors rejectUnknownArgs in generate-digest.ts (#7097) /
// collect-github-billing.ts (#7254). Runs before any token is
// resolved or any fetch is made.
const VALUE_FLAGS = ["--source", "--key", "--severity", "--title", "--body"];
function rejectUnknownArgs(): void {
  const args = process.argv.slice(2);
  const seen = new Set<string>();
  for (let i = 0; i < args.length; i++) {
    const token = args[i] as string;
    if (VALUE_FLAGS.includes(token)) {
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
    usageError(
      `--${name} requires a value that is not a flag (got ${value === undefined ? "no value" : JSON.stringify(value)})`,
    );
  }
  return value;
}

async function main(): Promise<void> {
  const source = arg("source");
  const key = arg("key");
  const severity = arg("severity") ?? "critical";
  const title = arg("title");
  const body = arg("body");
  if (!source || !key || !title) {
    console.error(USAGE);
    process.exit(2);
  }
  const token = await resolveBwsSecret("pm-aggregator.alerts-intake-token");
  if (!token) {
    console.error("post-alert: intake token unavailable (bws secret pm-aggregator.alerts-intake-token)");
    process.exit(1);
  }
  const base = (process.env.ALERTS_INTAKE_URL ?? "http://127.0.0.1:4870").replace(/\/+$/, "");
  const res = await fetch(`${base}/api/alerts`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ source, key, severity, title, body }),
    signal: AbortSignal.timeout(30_000),
  });
  const text = await res.text();
  console.log(text);
  if (!res.ok) process.exit(1);
}

await main();
