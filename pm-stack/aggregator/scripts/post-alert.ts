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
import { resolveBwsSecret } from "../src/secrets.ts";

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

async function main(): Promise<void> {
  const source = arg("source");
  const key = arg("key");
  const severity = arg("severity") ?? "critical";
  const title = arg("title");
  const body = arg("body");
  if (!source || !key || !title) {
    console.error("usage: post-alert.ts --source <s> --key <k> --title <t> [--severity critical|warning|info] [--body <b>]");
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
