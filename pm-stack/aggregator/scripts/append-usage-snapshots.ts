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
// Defaults: with no env override set, the view/registry resolve to
// the laptop's ~/hatch-personal mirror of the goal workspaces when
// present and the outbox to the collector's state dir
// (~/.local/state/pm-aggregator), falling back to the ~/workspace
// goal tree (src/default-paths.ts, #4889) — the ~/workspace tree on
// the laptop is a stub with no quota-view.md, which is what used
// to kill this step with ENOENT.
import { appendFile, mkdir, readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { extractQuotaPools } from "../src/quota-extract.ts";
import { defaultHiddenDir, defaultQuotaViewPath, defaultUsageOutboxPath } from "../src/default-paths.ts";
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
const HIDDEN = defaultHiddenDir(HOME);
const quotaViewPath = process.env.QUOTA_VIEW_PATH ?? defaultQuotaViewPath(HOME);
const registryPath = process.env.QUOTA_REGISTRY_PATH ?? join(HIDDEN, "provider-quota-registry.md");
const providerUsagePath = process.env.PROVIDER_USAGE_SNAPSHOT_PATH ?? join(HIDDEN, "provider-usage-snapshot.json");
const outboxPath = process.env.USAGE_SNAPSHOT_OUTBOX_PATH ?? defaultUsageOutboxPath(HOME);

// Resolve the --by value BEFORE any file is read or written (#3295).
// A missing, empty, or flag-like value used to flow into every derived
// record as an empty observedBy, fail isSnapshotRecord for all of them,
// and exit 0 with nothing appended — a silent loss of the whole run.
// Now it is a usage error (exit 2) and the outbox stays untouched.
function resolveObservedBy(defaultName: string): string {
  const byArg = process.argv.indexOf("--by");
  if (byArg < 0) return defaultName;
  const value: string | undefined = process.argv[byArg + 1];
  if (value === undefined || value.trim() === "" || value.startsWith("-")) {
    console.error(
      "append-usage-snapshots: --by requires a non-empty name as its value " +
        `(got ${value === undefined ? "no value" : JSON.stringify(value)}); refusing to run, outbox untouched`,
    );
    process.exit(2);
  }
  return value;
}

// Reject unknown arguments BEFORE any file is read or written (#6961).
// The argv used to be probed token-by-token and everything unrecognized
// silently dropped: a misspelled --from-provider-usage fell through to
// the default derivation mode and lost (or replaced) the whole run's
// records with exit 0, and a typo'd --byy silently discarded the
// requested observedBy. The token after --by is its value slot: it is
// consumed here and judged by resolveObservedBy (#3295), never
// re-interpreted as a flag.
function rejectUnknownArgs(): void {
  const args = process.argv.slice(2);
  for (let i = 0; i < args.length; i++) {
    const arg = args[i] as string;
    if (arg === "--from-provider-usage") continue;
    if (arg === "--by") {
      i++;
      continue;
    }
    console.error(
      `append-usage-snapshots: unknown argument '${arg}' ` +
        "(usage: [--from-provider-usage] [--by <name>]); refusing to run, outbox untouched",
    );
    process.exit(2);
  }
}

async function main(): Promise<void> {
  rejectUnknownArgs();
  const fromProviderUsage = process.argv.includes("--from-provider-usage");
  let records: SnapshotRecord[] = [];
  if (fromProviderUsage) {
    const observedBy = resolveObservedBy("check-usage");
    const parsed: unknown = JSON.parse(await readFile(providerUsagePath, "utf8"));
    records = recordsFromProviderUsage(parsed, observedBy);
  } else {
    const observedBy = resolveObservedBy("stewardship-cycle");
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
