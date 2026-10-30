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
  join(
    HOME,
    "workspace/goals/keep-the-four-repos-healthy/hidden_files/usage-snapshot-outbox.jsonl",
  );

function argValue(flag: string): string | undefined {
  const i = process.argv.indexOf(flag);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

function fetchUsageItems(user: string, year: number, month: number): BillingUsageItem[] {
  let out: string;
  try {
    out = execFileSync(
      "gh",
      ["api", `users/${user}/settings/billing/usage?year=${year}&month=${month}`],
      { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
    );
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
  const year = Number(argValue("--year") ?? now.getUTCFullYear());
  const month = Number(argValue("--month") ?? now.getUTCMonth() + 1);
  const user = argValue("--user") ?? process.env.GH_BILLING_USER ?? "dezren39";
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
