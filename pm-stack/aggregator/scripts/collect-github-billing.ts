#!/usr/bin/env bun
// Daily GitHub billing collector (#1051; hub design §2.2/§5): reads
// the unified billing usage endpoint with the existing gh OAuth token
// (no billing-scoped token needed for the own-account read — the old
// per-product endpoints are gone, 410) and appends per-day §4
// snapshot records to the writers' outbox, which the fleet-feed
// collector drains into the feed and the aggregator ingests into the
// snapshot store. Records carry their usage date, so re-running a
// month re-derives identical records and the store dedupes them.
//
// Runtime home: wherever gh carries the `user` scope — the endpoint
// 404s without it. Today that is the sandbox-side writer context (the
// same home as append-usage-snapshots under stewardship); the
// laptop's gh (gist/read:org/repo/workflow) cannot read it until its
// auth gains `user` via an interactive `gh auth refresh -s user`.
//
// Usage: bun run scripts/collect-github-billing.ts [--year YYYY] [--month M]
//        [--user dezren39] [--by <name>] [--print]
//   --print  write records to stdout instead of the outbox (dry run).
// Env: USAGE_SNAPSHOT_OUTBOX_PATH, GH_BILLING_USER.
import { execFileSync } from "node:child_process";
import { appendFile, mkdir } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import type { SnapshotRecord } from "../src/model.ts";
import { isSnapshotRecord, snapshotKey } from "../src/snapshots.ts";
import {
  aggregateBillingUsage,
  recordsFromBillingUsage,
  summarizeBillingDays,
  type BillingUsageItem,
} from "../src/sources/github-billing.ts";

const HOME = homedir();
const outboxPath =
  process.env.USAGE_SNAPSHOT_OUTBOX_PATH ??
  join(HOME, "workspace/goals/keep-the-four-repos-healthy/hidden_files/usage-snapshot-outbox.jsonl");

function argValue(flag: string): string | undefined {
  const i = process.argv.indexOf(flag);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

// Validate explicit --year/--month/--user values BEFORE any gh call
// (#3419). Malformed values used to flow straight into the billing API
// URL (month=NaN, month=13, users/--print) and the failure was then
// misreported as a gh token-scope problem. A missing, flag-like, or
// out-of-range value is a usage error (exit 2); omitted flags keep
// their defaults, and the token-scope hint in fetchUsageItems is only
// reachable with validated values.
function usageError(message: string): never {
  console.error(`collect-github-billing: ${message}; refusing to run, no gh call made`);
  process.exit(2);
}

function explicitArg(flag: string): string {
  const value: string | undefined = process.argv[process.argv.indexOf(flag) + 1];
  if (value === undefined || value.startsWith("--"))
    usageError(`${flag} requires a value (got ${value === undefined ? "no value" : JSON.stringify(value)})`);
  return value;
}

function resolveYear(now: Date): number {
  if (!process.argv.includes("--year")) return now.getUTCFullYear();
  const raw = explicitArg("--year");
  const year = /^\d+$/.test(raw.trim()) ? Number(raw.trim()) : NaN;
  if (!Number.isInteger(year) || year < 2000 || year > now.getUTCFullYear())
    usageError(`--year must be an integer from 2000 through ${now.getUTCFullYear()} (got ${JSON.stringify(raw)})`);
  return year;
}

function resolveMonth(now: Date): number {
  if (!process.argv.includes("--month")) return now.getUTCMonth() + 1;
  const raw = explicitArg("--month");
  const month = /^\d+$/.test(raw.trim()) ? Number(raw.trim()) : NaN;
  if (!Number.isInteger(month) || month < 1 || month > 12)
    usageError(`--month must be an integer from 1 through 12 (got ${JSON.stringify(raw)})`);
  return month;
}

function resolveUser(): string {
  if (!process.argv.includes("--user")) return process.env.GH_BILLING_USER ?? "dezren39";
  const raw = explicitArg("--user");
  if (raw.trim() === "") usageError(`--user requires a non-empty name (got ${JSON.stringify(raw)})`);
  return raw;
}

function fetchUsageItems(user: string, year: number, month: number): BillingUsageItem[] {
  let out: string;
  try {
    out = execFileSync("gh", ["api", `users/${user}/settings/billing/usage?year=${year}&month=${month}`], {
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    });
  } catch (e) {
    throw new Error(
      `gh api billing usage failed — the endpoint needs a gh token with the "user" scope ` +
        `(a 404 here usually means the scope, not a missing user). Underlying error: ${String(e)}`,
    );
  }
  const parsed: unknown = JSON.parse(out);
  const items = (parsed as { usageItems?: unknown }).usageItems;
  if (!Array.isArray(items)) throw new Error("billing usage response has no usageItems array");
  return items as BillingUsageItem[];
}

async function main(): Promise<void> {
  const now = new Date();
  const year = resolveYear(now);
  const month = resolveMonth(now);
  const user = resolveUser();
  const observedBy = argValue("--by") ?? "collect-github-billing";
  const items = fetchUsageItems(user, year, month);
  const totals = summarizeBillingDays(aggregateBillingUsage(items));
  console.log(
    `collect-github-billing: ${user} ${year}-${String(month).padStart(2, "0")}: ` +
      `${items.length} items — actions ${totals.actionsMinutes} min, ` +
      `codespaces ${totals.codespacesHours} h, storage ` +
      `${Math.round((totals.actionsStorageGbHours + totals.codespacesStorageGbHours) * 1000) / 1000} GB-hours, ` +
      `gross $${totals.grossUsd}, net $${totals.netUsd} ` +
      `(source: GET /users/${user}/settings/billing/usage)`,
  );
  const records: SnapshotRecord[] = recordsFromBillingUsage(items, observedBy);
  const valid = records.filter(isSnapshotRecord);
  if (valid.length !== records.length)
    throw new Error(`${records.length - valid.length} derived record(s) failed validation`);
  const seen = new Set<string>();
  const fresh = valid.filter((r) => {
    const k = snapshotKey(r);
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
  if (process.argv.includes("--print")) {
    for (const r of fresh) console.log(JSON.stringify(r));
    console.log(`collect-github-billing: ${fresh.length} record(s) (dry run, outbox untouched)`);
    return;
  }
  if (!fresh.length) {
    console.log(`collect-github-billing: no records derived; outbox untouched (${outboxPath})`);
    return;
  }
  await mkdir(dirname(outboxPath), { recursive: true });
  await appendFile(outboxPath, fresh.map((r) => JSON.stringify(r)).join("\n") + "\n", "utf8");
  console.log(`collect-github-billing: appended ${fresh.length} record(s) to ${outboxPath}`);
}

await main();
