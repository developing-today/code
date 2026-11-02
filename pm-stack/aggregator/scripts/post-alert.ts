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

type ValueFlag = "source" | "key" | "severity" | "title" | "body";

type PostAlertValues = {
  source: string;
  key: string;
  severity: string;
  title: string;
  body?: string;
};

type ParseResult = { ok: true; value: PostAlertValues } | { ok: false; error: string };

const VALUE_FLAGS: ValueFlag[] = ["source", "key", "severity", "title", "body"];

export function parsePostAlertArgs(argv: string[]): ParseResult {
  const values: Partial<Record<ValueFlag, string>> = {};
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i].startsWith("--") ? argv[i].slice(2) : "";
    if (!VALUE_FLAGS.includes(flag as ValueFlag)) continue;

    const value = argv[i + 1];
    if (!value || value.startsWith("--")) {
      return { ok: false, error: `post-alert: --${flag} requires a value` };
    }
    if (values[flag as ValueFlag] === undefined) values[flag as ValueFlag] = value;
    i++;
  }

  const { source, key, severity, title, body } = values;
  if (!source || !key || !title) {
    return {
      ok: false,
      error: "usage: post-alert.ts --source <s> --key <k> --title <t> [--severity critical|warning|info] [--body <b>]",
    };
  }
  return { ok: true, value: { source, key, severity: severity ?? "critical", title, body } };
}

export type PostAlertDependencies = {
  resolveToken: (name: string) => Promise<string | null>;
  fetch: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;
};

export async function runPostAlert(
  argv: string[],
  deps: PostAlertDependencies = { resolveToken: resolveBwsSecret, fetch: globalThis.fetch },
): Promise<number> {
  const parsed = parsePostAlertArgs(argv);
  if (!parsed.ok) {
    console.error(parsed.error);
    return 2;
  }

  const { source, key, severity, title, body } = parsed.value;
  const token = await deps.resolveToken("pm-aggregator.alerts-intake-token");
  if (!token) {
    console.error("post-alert: intake token unavailable (bws secret pm-aggregator.alerts-intake-token)");
    return 1;
  }
  const base = (process.env.ALERTS_INTAKE_URL ?? "http://127.0.0.1:4870").replace(/\/+$/, "");
  const res = await deps.fetch(`${base}/api/alerts`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ source, key, severity, title, body }),
    signal: AbortSignal.timeout(30_000),
  });
  const text = await res.text();
  console.log(text);
  return res.ok ? 0 : 1;
}

if (import.meta.main) process.exitCode = await runPostAlert(process.argv.slice(2));
