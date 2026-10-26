#!/usr/bin/env bun
// Append usage snapshot records to the writers' outbox (hub design
// §4, writers (b) and (c)). The outbox is a JSONL file beside the
// keeper's provider-usage snapshot; the fleet-feed collector drains
// it into the feed on its next run, and the laptop aggregator ingests
// feed records into the snapshot store, which dedupes on
// (service, metric, ts, value) — so running this twice, or a record
// arriving via two writers, appends nothing the second time.
//
// Modes:
//   (default)             Derive records from quota-view.md + the
//                         provider registry (pools, SaaS services,
//                         PAYG lines) — stewardship-cycle runs this
//                         right after a quota-view refresh, so the
//                         dated reads it just recorded become
//                         snapshots at recording time.
//   --from-provider-usage Derive records from the keeper's existing
//                         provider-usage-snapshot.json (hand runs;
//                         check-usage.py appends the same records
//                         itself on its normal runs).
//
// Usage: bun run scripts/append-usage-snapshots.ts [--from-provider-usage] [--by <name>]
// Env: QUOTA_VIEW_PATH, QUOTA_REGISTRY_PATH, PROVIDER_USAGE_SNAPSHOT_PATH,
//      USAGE_SNAPSHOT_OUTBOX_PATH.
import { appendFile, mkdir, readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { extractQuotaPools } from "../src/quota-extract.ts";
import { extractPaygLines, extractSaasServices } from "../src/saas-extract.ts";
import {
  recordsFromPaygLines,
  recordsFromPools,
  recordsFromProviderUsage,
  recordsFromSaasServices,
} from "../src/snapshot-records.ts";
import { isSnapshotRecord, snapshotKey } from "../src/snapshots.ts";
import type { SnapshotRecord } from "../src/model.ts";

const HOME = homedir();
const HIDDEN = join(HOME, "workspace/goals/keep-the-four-repos-healthy/hidden_files");
const quotaViewPath =
  process.env.QUOTA_VIEW_PATH ??
  join(
    HOME,
    "workspace/goals/stewardship-commission-recurring-review-and-dispatch/hidden_files/visibility/quota-view.md",
  );
const registryPath = process.env.QUOTA_REGISTRY_PATH ?? join(HIDDEN, "provider-quota-registry.md");
const providerUsagePath =
  process.env.PROVIDER_USAGE_SNAPSHOT_PATH ?? join(HIDDEN, "provider-usage-snapshot.json");
const outboxPath = process.env.USAGE_SNAPSHOT_OUTBOX_PATH ?? join(HIDDEN, "usage-snapshot-outbox.jsonl");

async function main(): Promise<void> {
  const fromProviderUsage = process.argv.includes("--from-provider-usage");
  const byArg = process.argv.indexOf("--by");
  let records: SnapshotRecord[] = [];
  if (fromProviderUsage) {
    const observedBy = byArg >= 0 ? process.argv[byArg + 1] : "check-usage";
    const parsed: unknown = JSON.parse(await readFile(providerUsagePath, "utf8"));
    records = recordsFromProviderUsage(parsed, observedBy);
  } else {
    const observedBy = byArg >= 0 ? process.argv[byArg + 1] : "stewardship-cycle";
    const doc = await readFile(quotaViewPath, "utf8");
    const registryText = await readFile(registryPath, "utf8").catch(() => "");
    const now = new Date();
    records = [
      ...recordsFromPools(extractQuotaPools(doc, now), doc, observedBy),
      ...recordsFromSaasServices(extractSaasServices(doc), observedBy),
      ...recordsFromPaygLines(extractPaygLines(doc, registryText), observedBy),
    ];
  }
  const valid = records.filter(isSnapshotRecord);
  const seen = new Set<string>();
  const fresh = valid.filter((r) => {
    const k = snapshotKey(r);
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
  if (!fresh.length) {
    console.log(`append-usage-snapshots: no records derived; outbox untouched (${outboxPath})`);
    return;
  }
  await mkdir(dirname(outboxPath), { recursive: true });
  await appendFile(outboxPath, fresh.map((r) => JSON.stringify(r)).join("\n") + "\n", "utf8");
  console.log(`append-usage-snapshots: appended ${fresh.length} record(s) to ${outboxPath}`);
}

await main();
