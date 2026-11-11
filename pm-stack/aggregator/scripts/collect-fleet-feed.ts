#!/usr/bin/env bun
// Fleet feed collector (#890): runs on the sandbox host, where the
// quota sources of record live, and emits fleet-feed.json — the file
// the laptop aggregator renders for its quota (#851) and sandbox
// attention (#852) facets.
//
// Sources (all read-only, all fail-soft):
//   - the stewardship quota view (dated per-pool reads) — extracted by
//     src/quota-extract.ts; a pool whose read pattern stops matching
//     renders with identity + a "no current read" note.
//   - the sandbox OpenCode server API — sandbox session feed entries
//     (shared mapper, src/sources/opencode.ts) + live per-pool burn
//     attribution for the quota view.
//   - charges-candidates.json — the Muse-side producer's
//     (#1057, scripts/produce-charge-candidates.ts) Plaid + Gmail
//     charge candidates for the period. The collector runs the pure
//     matcher (src/charges-match.ts) over them, merges the rows with
//     the hand-fed registry/quota-view charges (auto wins on a
//     duplicate), and surfaces unmatched SaaS/AI-looking charges in
//     the feed's chargesAttention exactly once — charges-surfaced-
//     state.json persists the keys already shown. Absent file = the
//     pre-#1057 hand-fed charges only. No balances or debt data ever
//     pass through: candidates are charge evidence only.
//   - orchestration.json — the orchestration job's latest section
//     (#899: tiers, dependencies, goal measurements), embedded only
//     after normalizeOrchestrationSection accepts it (the collector
//     gate in the publisher -> collector -> aggregator contract,
//     #1176); absent/malformed = section omitted. The publisher's
//     pre-publish gate is `bun run contract:check -- --publisher
//     <path>` (scripts/check-contract-corpus.ts), which runs this
//     same validator; the shared corpus lives in
//     test/contract-corpus/v1/.
//   - the org GitHub Project "Developing Today" (#4, #910 stage 5) —
//     read via GraphQL Projects v2 into the feed's `projects` pool.
//     Token plumbing: GITHUB_PROJECT_TOKEN / GH_TOKEN from the env
//     when present; otherwise the collector resolves the Bitwarden
//     secret named `github.project-token` itself (see
//     resolveProjectToken) using ~/tools/bin/bws + the machine-account
//     access-token file under $HOME/.config/bitwarden — both already
//     present under the collector systemd unit's pinned HOME, so no
//     unit change is needed. The token value is never logged, never
//     written to the feed, and only ever sent to api.github.com.
//     Any failure (no token, bws missing, GraphQL error) omits the
//     section and logs only that it was omitted.
//   - the AA cost explorer data — Supabase `ainventory` (the AA
//     import: v_rankings, v_threshold_metrics,
//     benchmark_results — both the AA Intelligence Index rows and the
//     Coding Agent Index rows, kept as separate benchmark families —
//     the Test_Runs data, and the price event log's v_current_prices),
//     read through v_latest_import_batch via the Management API SQL endpoint.
//     Token plumbing mirrors the Projects read: the Bitwarden secret
//     named `supabase.muse-dev2.access-token-muse-cli` (the sbp_ management
//     token), resolved by resolveBwsSecret and sent only to
//     api.supabase.com. Any failure embeds the section in its absent
//     form ({status:"absent", reason}) — the feed never fails over it.
//     Pool/catalog coverage is computed against the repo's own
//     .opencode/opencode.jsonc (provider catalog + model_routes
//     leaves); an unreadable config omits only the coverage.
//
// Usage: bun run scripts/collect-fleet-feed.ts [--out <path>]
// Env overrides: QUOTA_VIEW_PATH, OPENCODE_SERVER_URL
// (SANDBOX_OPENCODE_URL legacy alias), OPENCODE_SESSION_SOURCE,
// OPENCODE_SERVER_PASSWORD_FILE, ORCHESTRATION_PATH, DNS_REGISTER_PATH,
// CLAUDE_LAUNCH_RECORDS_PATH, QUOTA_REGISTRY_PATH (the hub's PAYG/charges
// sections read the provider registry's billing incidents, fail-soft),
// USAGE_SNAPSHOT_OUTBOX_PATH (the §4 writers' outbox JSONL — stewardship
// and check-usage append records there; each collection drains it into
// the feed's snapshots field for the laptop store to ingest),
// BURN_HISTORY_STATE_PATH (#3933: the collector's persisted
// pool-window observations the burn history + measured rates are
// derived from; beside the charges-surfaced state by default),
// CHARGE_CANDIDATES_PATH / CHARGES_SURFACED_STATE_PATH (#1057: the
// producer's candidates file and the collector's surfaced-once state),
// SUPABASE_PROJECT_REF (else the Bitwarden supabase.muse-dev2.project-ref secret;
// fallback default: the muse-dev2 project),
// AA_OPENCODE_CONFIG_PATH (default: the repo's .opencode/opencode.jsonc).
import { execFile } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { access, mkdir, open, readFile, rename, stat, unlink, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { dnsSectionFromRegister, dnsSectionFromRegisterJson, parseDnsRegister } from "../src/dns.ts";
import {
  disabledProvidersFromConfig,
  aaAbsent,
  aaSectionFromSupabase,
  isAaSection,
  type AaRawPayload,
} from "../src/aa.ts";
import { AA_BATCH_INDEX_SQL, AA_PRICES_SQL, aaPayloadSql } from "../src/aa-batch-sql.ts";
import {
  aaBatchFingerprint,
  aaCacheFileName,
  aaPricesFingerprint,
  isAaBatchCacheEntry,
  parseBatchIndex,
  summarizeBatches,
  type AaBatchCacheEntry,
  type AaBatchIndexRow,
} from "../src/aa-batches.ts";
import {
  feedSectionStatus,
  normalizeOrchestrationSection,
  orchestrationSectionProblem,
  sanitizeCollectionError,
  type FeedSectionStatus,
  type FleetFeed,
} from "../src/feed.ts";
import { computeCheckoutDrift } from "../src/checkout-drift.ts";
import { assembleCollectorFeed } from "../src/feed-contract.ts";
import { defaultNeedsDrewFile, listAnswers } from "../src/needs-drew.ts";
import type {
  AaBatchSummary,
  AaFeedSection,
  AaSection,
  DnsSection,
  OrchestrationSection,
  BurnSegment,
  QuotaPool,
  SessionFeedEntry,
  SessionRecord,
  SessionSource,
} from "../src/model.ts";
import { matchCharges, type UnmatchedCharge } from "../src/charges-match.ts";
import { chargeServiceCatalog } from "../src/charges-catalog.ts";
import { filterNewUnmatched, mergeChargeRows, parseChargeCandidates } from "../src/charges-collect.ts";
import { extractQuotaPools } from "../src/quota-extract.ts";
import { computeChoices } from "../src/quota-guidance.ts";
import {
  deriveBurnHistory,
  deriveMeasuredRatesFromPoolTokens,
  mergeObservations,
  mergePoolTokenObservations,
  type PoolTokenObservation,
  observationsFromPools,
  type PoolWindowObservation,
} from "../src/burn-history.ts";
import {
  extractCharges,
  extractPaygLines,
  extractSaasServices,
  extractSubscriptions,
  extractUninstrumentedServices,
} from "../src/saas-extract.ts";
import type { ChargeRow, PaygLine, SaasService, SnapshotRecord } from "../src/model.ts";
import { recordsFromDns, recordsFromPaygLines, recordsFromSaasServices } from "../src/snapshot-records.ts";
import { isSnapshotRecord, snapshotKey } from "../src/snapshots.ts";
import { fetchProjectSnapshot } from "../src/projects.ts";
import type { ProjectPoolItem } from "../src/model.ts";
import { REPOS } from "../src/model.ts";
import { staleIssueSearchQuery, type RepoCensus } from "../src/coverage.ts";
import { parseLaunchRecords, recordFromSessionEntry } from "../src/registry.ts";
import { mapServerSessions, poolBurn } from "../src/sources/opencode.ts";
import type { FreshnessInput } from "../src/freshness.ts";

const PKG = join(dirname(fileURLToPath(import.meta.url)), "..");
const HOME = homedir();
const HIDDEN = join(HOME, "workspace/goals/keep-the-four-repos-healthy/hidden_files");

export type FleetFeedPublication = Readonly<{
  /** Absolute destination path that was replaced. */
  path: string;
  /** SHA-256 of the exact UTF-8 bytes published. */
  sha256: string;
  /** Number of UTF-8 bytes published. */
  sizeBytes: number;
}>;

type PublicationTempFile = {
  writeFile(contents: string): Promise<void>;
  close(): Promise<void>;
};

/** File operations are injectable so failure cleanup can be exercised without platform-specific permissions. */
export type FleetFeedPublicationFileOps = {
  openTemp(path: string): Promise<PublicationTempFile>;
  rename(from: string, to: string): Promise<void>;
  unlink(path: string): Promise<void>;
};

const publicationFileOps: FleetFeedPublicationFileOps = {
  async openTemp(path) {
    const file = await open(path, "wx");
    return {
      async writeFile(contents) {
        await file.writeFile(contents, { encoding: "utf8" });
      },
      close: () => file.close(),
    };
  },
  rename,
  unlink,
};

/**
 * Publish one complete feed atomically. The returned metadata exists only
 * after the same-directory rename succeeds, so callers can safely use it
 * as the basis for a post-publication freshness invalidation.
 */
export async function publishFleetFeed(
  outputPath: string,
  contents: string,
  fileOps: FleetFeedPublicationFileOps = publicationFileOps,
): Promise<FleetFeedPublication> {
  const path = resolve(outputPath);
  const tempPath = join(dirname(path), "." + basename(path) + "." + process.pid + "." + randomUUID() + ".tmp");
  let tempCreated = false;
  try {
    const tempFile = await fileOps.openTemp(tempPath);
    tempCreated = true;
    try {
      await tempFile.writeFile(contents);
    } finally {
      await tempFile.close();
    }
    await fileOps.rename(tempPath, path);
  } catch (error) {
    if (tempCreated) await fileOps.unlink(tempPath).catch(() => {});
    throw error;
  }

  return {
    path,
    sha256: createHash("sha256").update(contents, "utf8").digest("hex"),
    sizeBytes: Buffer.byteLength(contents, "utf8"),
  };
}

/** The notice is submitted only after the atomic replacement succeeds.
 *  The generation + digest identity makes a retried notice idempotent. */
export async function publishFleetFeedWithInvalidation(
  outputPath: string,
  contents: string,
  generatedAt: string,
  notify: (input: FreshnessInput) => Promise<unknown>,
  fileOps: FleetFeedPublicationFileOps = publicationFileOps,
): Promise<FleetFeedPublication> {
  const publication = await publishFleetFeed(outputPath, contents, fileOps);
  await notify({
    eventId: `fleet-feed:${generatedAt}:${publication.sha256}`,
    source: "fleet-feed",
    scope: ["feed"],
    occurredAt: Number.isFinite(Date.parse(generatedAt)) ? new Date(generatedAt).toISOString() : null,
    reason: "source-event",
  });
  return publication;
}

// Strict operator-argument contract (#3352, #3509): the only accepted
// forms are no arguments or one documented separated `--out <path>`.
// Every other token — a trailing/empty/flag-like --out value, an
// --out=<path> join, a duplicate --out, or any unknown or positional
// token — is a usage error (exit 2) raised here, before any collection,
// network, subprocess, lock, or file-write side effect.
function usageError(message: string): never {
  console.error(`collect-fleet-feed: ${message}; usage: bun run scripts/collect-fleet-feed.ts [--out <path>]`);
  process.exit(2);
}

function resolveOutPath(): string {
  const fallback = process.env.FLEET_FEED_OUT ?? join(PKG, "fleet-feed.json");
  const args = process.argv.slice(2);
  if (args.length === 0) return fallback;
  if (args.filter((a) => a === "--out").length > 1)
    usageError(`duplicate --out (got ${JSON.stringify(args)}); the documented form is a single --out <path>`);
  if (args[0] === "--out") {
    const value: string | undefined = args[1];
    if (value === undefined || value === "" || value.startsWith("--"))
      usageError(`--out requires a path value (got ${value === undefined ? "no value" : JSON.stringify(value)})`);
    if (args.length > 2) usageError(`unexpected argument ${JSON.stringify(args[2])}`);
    return value;
  }
  usageError(`unknown argument ${JSON.stringify(args[0])}`);
}

const outPath = resolveOutPath();
const quotaViewPath =
  process.env.QUOTA_VIEW_PATH ??
  join(
    HOME,
    "workspace/goals/stewardship-commission-recurring-review-and-dispatch/hidden_files/visibility/quota-view.md",
  );
const freeServicesTrackerPath =
  process.env.PM_AGG_FREE_SERVICES_TRACKER ??
  join(HOME, "hatch-personal/workspace/ai-subscriptions/free-services-to-check.md");
const orchestrationPath = process.env.ORCHESTRATION_PATH ?? join(HIDDEN, "orchestration.json");
const dnsRegisterPath =
  process.env.DNS_REGISTER_PATH ??
  join(HOME, "workspace/goals/stand-up-lab-cluster-talm-talos-cozystack-postgres-rustfs/files/dns-domain-register.md");
// Slice 2 of #902: register.json sits beside the register Markdown and
// is the authoritative source for the fields it carries.
const dnsRegisterJsonPath = process.env.DNS_REGISTER_JSON_PATH ?? join(dirname(dnsRegisterPath), "register.json");
const launchRecordsPath = process.env.CLAUDE_LAUNCH_RECORDS_PATH ?? join(HIDDEN, "claude-launch-records.jsonl");
const quotaRegistryPath = process.env.QUOTA_REGISTRY_PATH ?? join(HIDDEN, "provider-quota-registry.md");
const snapshotOutboxPath = process.env.USAGE_SNAPSHOT_OUTBOX_PATH ?? join(HIDDEN, "usage-snapshot-outbox.jsonl");
const chargeCandidatesPath = process.env.CHARGE_CANDIDATES_PATH ?? join(HIDDEN, "charges-candidates.json");
const chargesSurfacedStatePath = process.env.CHARGES_SURFACED_STATE_PATH ?? join(HIDDEN, "charges-surfaced-state.json");
const burnHistoryStatePath =
  process.env.BURN_HISTORY_STATE_PATH ?? join(dirname(chargesSurfacedStatePath), "burn-history-state.json");
const opencodeConfigPath = process.env.AA_OPENCODE_CONFIG_PATH ?? join(PKG, "..", "..", ".opencode", "opencode.jsonc");
const opencodeRuntimeConfigPath =
  process.env.AA_OPENCODE_RUNTIME_CONFIG_PATH ?? join(HOME, ".local", "state", "opencode", "opencode-runtime.jsonc");
const serverUrl = process.env.OPENCODE_SERVER_URL ?? process.env.SANDBOX_OPENCODE_URL ?? "http://127.0.0.1:4096";
/** Session source label for the scraped server: the collector's home host
 * decides — the laptop collector labels its local server
 * "opencode-laptop"; anything else keeps the historical default. */
const sessionSource: SessionSource =
  process.env.OPENCODE_SESSION_SOURCE === "opencode-laptop" ? "opencode-laptop" : "opencode-sandbox";
const passwordFile =
  process.env.OPENCODE_SERVER_PASSWORD_FILE ?? join(HOME, "tools/secrets.d/opencode-server-password");

/** Provider id (OpenCode config) -> quota pool id, for burn attribution. */
function poolFor(provider: string, model: string): string | null {
  switch (provider) {
    case "meta":
      return "meta";
    case "claude-code":
    case "_claude":
      return "anthropic:claude-pro";
    case "openai":
    case "_openai":
    case "_openai.main":
    case "codex":
      // Keep aligned with the pool IDs in ../src/quota-extract.ts; matching below is exact.
      // ("codex" is the pre-rename fork quota id, fork PR #2261; a row
      // still stamped with it files under the renamed pool.)
      return "_openai.main";
    case "_openai.alt":
      return "_openai.alt";
    case "cline-pass":
      return "cline-pass";
    case "opendesign":
      return "opendesign";
    case "deepseek":
      return "deepseek";
    case "antigravity-direct":
    case "antigravity-ls":
    case "antigravity-agy":
    case "google":
      return model.startsWith("gemini") ? "antigravity-direct.gemini" : "antigravity-direct.claude-gpt";
    case "_antigravity":
      return model.startsWith("gemini") ? "_antigravity.gemini" : "_antigravity.claude-gpt";
    default:
      // opencode-go is deliberately unattributed in slice 1: which
      // workspace (Personal vs Lab) a sandbox session burns is not
      // visible from the session list alone.
      return null;
  }
}

/**
 * Drain the §4 writers' outbox: records appended by stewardship-cycle
 * (scripts/append-usage-snapshots.ts) and the keeper's check-usage.
 * Rename-then-read so a writer appending mid-collection lands in the
 * fresh file for the next run — nothing is lost, and the laptop store
 * dedupes anything forwarded twice.
 */
async function drainSnapshotOutbox(): Promise<SnapshotRecord[]> {
  const drainPath = `${snapshotOutboxPath}.drain-${process.pid}`;
  try {
    await rename(snapshotOutboxPath, drainPath);
  } catch {
    return []; // no outbox yet — the normal state until a writer runs
  }
  try {
    const text = await readFile(drainPath, "utf8");
    const out: SnapshotRecord[] = [];
    for (const line of text.split("\n")) {
      if (!line.trim()) continue;
      try {
        const parsed: unknown = JSON.parse(line);
        if (isSnapshotRecord(parsed)) out.push(parsed);
      } catch {
        // one bad line never blocks the drain
      }
    }
    return out;
  } catch (err) {
    console.error(`collector: snapshot outbox drain failed (${err}); records stay for the next run`);
    return [];
  } finally {
    await unlink(drainPath).catch(() => {});
  }
}

/**
 * Resolve one Bitwarden Secrets Manager secret by key, without ever
 * logging its value: the bws CLI + the machine-account access-token
 * file under $HOME/.config/bitwarden — both already present under the
 * collector systemd unit's pinned HOME. Returns null when the route
 * yields nothing (bws missing, file unreadable, key absent).
 */
async function resolveBwsSecret(key: string): Promise<string | null> {
  const bwsBin = process.env.BWS_BIN ?? join(HOME, "tools/bin/bws");
  const accessTokenFile = process.env.BWS_ACCESS_TOKEN_FILE ?? join(HOME, ".config/bitwarden/access-token");
  try {
    await access(bwsBin);
    const accessToken = (await readFile(accessTokenFile, "utf8")).trim();
    if (!accessToken) return null;
    const run = (args: string[]): Promise<string> =>
      new Promise((resolve, reject) => {
        execFile(
          bwsBin,
          args,
          { env: { ...process.env, BWS_ACCESS_TOKEN: accessToken }, maxBuffer: 16 * 1024 * 1024, timeout: 30_000 },
          (err, stdout) => (err ? reject(err) : resolve(stdout)),
        );
      });
    const list = JSON.parse(await run(["secret", "list"])) as { id?: string; key?: string }[];
    const entry = list.find((s) => s.key === key);
    if (!entry?.id) return null;
    const got = JSON.parse(await run(["secret", "get", entry.id])) as { value?: string };
    return got.value && got.value.trim() ? got.value.trim() : null;
  } catch {
    return null; // bws unreachable/unreadable — fail soft, no details logged
  }
}

/**
 * Resolve the Projects-scope GitHub token without ever logging it.
 * Order: (1) GITHUB_PROJECT_TOKEN / GH_TOKEN already in the process
 * env (agent runs, manual runs, a future systemd EnvironmentFile);
 * (2) Bitwarden Secrets Manager via resolveBwsSecret.
 * Returns null when neither route yields a token (section omitted).
 */
async function resolveProjectToken(): Promise<string | null> {
  const fromEnv = process.env.GITHUB_PROJECT_TOKEN ?? process.env.GH_TOKEN;
  if (fromEnv && fromEnv.trim()) return fromEnv.trim();
  return resolveBwsSecret("github.project-token");
}

// --- AA cost explorer (Supabase ainventory) ---------------------------------
// One SQL statement returns the whole section payload as a single
// jsonb object (keys pre-renamed to the section's camelCase). The
// Test_Runs data rides as raw rows; src/aa.ts maps them by
// their own header row and computes pool/catalog coverage.

// The AA payload SQL is generated per selected batch by src/aa-batch-sql.ts
// (#3302): the v_rankings / v_threshold_metrics bodies are copied verbatim
// from ainventory with only their latest-batch CTE parameterized, so the
// default (latest) query returns exactly what it did before.

/**
 * The last-good AA section store (code#3332): kept beside the feed so a
 * transient Supabase failure degrades to the previous good payload
 * instead of blanking the explorer. Keyed by the import batch and the
 * opencode config fingerprint — the section is rebuilt when either
 * changes and reused untouched while neither does.
 */
const aaLastGoodPath = join(dirname(outPath), "aa-last-good.json");
// #1116: last-good attention entries, carried across runs whose server
// read fails (restart churn) so one bad read cannot zero the feed.
const attentionLastGoodPath = join(dirname(outPath), "attention-last-good.json");
/** Per-batch payload cache (#3302), beside the feed. */
const aaBatchDir = join(dirname(outPath), "aa-batches");
/** How many missing historical payloads one tick may build. The latest is
 * always fresh; history fills in over ticks so no single run storms the
 * Management API (#3332). */
const AA_HISTORICAL_BUILDS_PER_TICK = 2;

interface AaLastGood {
  batch: string;
  configFingerprint: string;
  /** sha256 of the batch-bound semantic data (aa-batches.ts). */
  fingerprint?: string;
  section: AaSection;
}

async function readAaLastGood(): Promise<AaLastGood | null> {
  try {
    const parsed = JSON.parse(await readFile(aaLastGoodPath, "utf8")) as Partial<AaLastGood>;
    if (typeof parsed.batch !== "string" || typeof parsed.configFingerprint !== "string") return null;
    if (!isAaSection(parsed.section) || parsed.section.status !== "ok") return null;
    return {
      batch: parsed.batch,
      configFingerprint: parsed.configFingerprint,
      fingerprint: typeof parsed.fingerprint === "string" ? parsed.fingerprint : undefined,
      section: parsed.section,
    };
  } catch {
    return null;
  }
}

async function writeAaLastGood(entry: AaLastGood): Promise<void> {
  try {
    const tmp = `${aaLastGoodPath}.tmp`;
    await writeFile(tmp, JSON.stringify(entry));
    await rename(tmp, aaLastGoodPath);
  } catch (err) {
    console.error(`collector: AA last-good store write failed (${err instanceof Error ? err.message : "error"})`);
  }
}

async function readAaBatchCache(batch: string): Promise<AaBatchCacheEntry | null> {
  try {
    const parsed: unknown = JSON.parse(await readFile(join(aaBatchDir, aaCacheFileName(batch)), "utf8"));
    if (!isAaBatchCacheEntry(parsed) || parsed.batch !== batch) return null;
    if (!isAaSection(parsed.section) || parsed.section.status !== "ok") return null;
    return parsed;
  } catch {
    return null;
  }
}

async function writeAaBatchCache(entry: AaBatchCacheEntry): Promise<boolean> {
  try {
    await mkdir(aaBatchDir, { recursive: true });
    const path = join(aaBatchDir, aaCacheFileName(entry.batch));
    const tmp = `${path}.tmp`;
    await writeFile(tmp, JSON.stringify(entry));
    await rename(tmp, path);
    return true;
  } catch (err) {
    console.error(`collector: AA batch cache write failed (${err instanceof Error ? err.message : "error"})`);
    return false;
  }
}

/** The coverage half of the AA section is computed from the opencode
 * config, so the reuse key includes the config's fingerprint. */
async function opencodeConfigFingerprint(): Promise<string> {
  try {
    const st = await stat(opencodeConfigPath);
    return `${st.mtimeMs}:${st.size}`;
  } catch {
    return "unreadable";
  }
}

async function supabaseQuery(token: string, ref: string, sql: string): Promise<unknown> {
  const res = await fetch(`https://api.supabase.com/v1/projects/${ref}/database/query`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ query: sql }),
    signal: AbortSignal.timeout(60_000),
  });
  if (!res.ok) throw new Error(`Supabase query failed (HTTP ${res.status})`);
  return res.json();
}

function aaRawFromRows(rows: unknown): AaRawPayload | null {
  if (!Array.isArray(rows) || typeof rows[0]?.aa !== "object" || rows[0].aa === null) return null;
  return rows[0].aa as AaRawPayload;
}

/** Fetch one batch payload (a uuid; null is the latest fallback). */
async function fetchAaPayload(token: string, ref: string, batch: string | null): Promise<AaRawPayload> {
  const raw = aaRawFromRows(await supabaseQuery(token, ref, aaPayloadSql(batch)));
  if (!raw) throw new Error("Supabase returned no AA payload");
  return raw;
}

/**
 * Build the AA feed section. Fail-soft like every other section: a
 * failed build degrades to the last good payload with a stale marker
 * (code#3332), or to the absent form when no good payload exists yet,
 * so the dashboard explains the gap. The management token is only ever
 * sent to api.supabase.com and is never logged or written to the feed.
 *
 * #3302: the index comes from ainventory.sources (complete); each batch
 * payload is cached under aa-batches/. The latest batch is refreshed
 * every tick, history fills in at a bounded rate, and the default
 * section is byte-compatible with the pre-#3302 latest payload.
 */
async function collectAaSection(generatedAt: string): Promise<AaFeedSection> {
  const lastGood = await readAaLastGood();
  const fingerprint = await opencodeConfigFingerprint();
  const degrade = (reason: string): AaFeedSection =>
    lastGood ? { ...lastGood.section, stale: true, staleReason: reason } : aaAbsent(reason, generatedAt);
  // Batch-independent config reads, done once for every assembly.
  let configText: string | null = null;
  try {
    configText = await readFile(opencodeConfigPath, "utf8");
  } catch {
    console.error("collector: opencode.jsonc unreadable; AA coverage omitted this run");
  }
  // #4544: the sync-models-owned provider catalogs live in the runtime
  // overlay; read it when present so AA coverage sees the effective config.
  let runtimeText: string | null = null;
  try {
    runtimeText = await readFile(opencodeRuntimeConfigPath, "utf8");
  } catch {
    runtimeText = null;
  }

  const cacheMeta = new Map<string, { fingerprint: string | null; cached: boolean }>();
  const assemble = async (raw: AaRawPayload): Promise<AaSection | null> => {
    const section = aaSectionFromSupabase(raw, configText, generatedAt, runtimeText);
    if (!section) return null;
    const contentFingerprint = aaBatchFingerprint(raw);
    const cached = await writeAaBatchCache({
      batch: section.batch,
      configFingerprint: fingerprint,
      fingerprint: contentFingerprint,
      pricesFingerprint: aaPricesFingerprint(section.prices ?? []),
      builtAt: generatedAt,
      section,
    });
    cacheMeta.set(section.batch, { fingerprint: contentFingerprint, cached });
    return section;
  };

  const token = await resolveBwsSecret("supabase.muse-dev2.access-token-muse-cli");
  if (!token) return degrade("no Supabase management token available (bws)");
  // Project ref: env override, else the Bitwarden supabase.muse-dev2.project-ref
  // secret (the same resolve-by-name discipline as the token), else the
  // muse-dev2 project — never the retired muse-dev project.
  const ref =
    process.env.SUPABASE_PROJECT_REF ??
    (await resolveBwsSecret("supabase.muse-dev2.project-ref")) ??
    "keihfchslgjetkmnmzra";

  // The workbook index is the complete sources register; ingest_batches is
  // optional metadata and a missing join row does not hide a batch.
  let index: AaBatchIndexRow[];
  try {
    const rows = (await supabaseQuery(token, ref, AA_BATCH_INDEX_SQL)) as { batches?: unknown }[];
    index = parseBatchIndex(Array.isArray(rows) ? rows[0]?.batches : null);
  } catch (err) {
    const msg = err instanceof Error ? err.message : "error";
    return degrade(msg.startsWith("Supabase query failed") ? msg : `Supabase unreachable (${msg})`);
  }
  if (index.length === 0) return degrade("no workbook batches in ainventory.sources");

  // The current-price overlay, hashed once per tick: it is the
  // validity key for every cached section's prices and price-derived
  // catalog (#3302 review). A price-only change — no new workbook,
  // no config change — must still rebuild the caches, because the
  // page presents these prices as current.
  let currentPricesFingerprint: string;
  try {
    const rows = (await supabaseQuery(token, ref, AA_PRICES_SQL)) as { prices?: unknown }[];
    currentPricesFingerprint = aaPricesFingerprint(Array.isArray(rows) ? rows[0]?.prices : null);
  } catch (err) {
    const msg = err instanceof Error ? err.message : "error";
    return degrade(msg.startsWith("Supabase query failed") ? msg : `Supabase unreachable (${msg})`);
  }

  // Read every batch cache once; a changed config — or a changed
  // current-price overlay — invalidates all of them.
  const cachedSections = new Map<string, AaSection>();
  for (const row of index) {
    const cached = await readAaBatchCache(row.importBatch);
    if (
      cached &&
      cached.configFingerprint === fingerprint &&
      (cached.pricesFingerprint ?? null) === currentPricesFingerprint
    ) {
      cacheMeta.set(row.importBatch, { fingerprint: cached.fingerprint, cached: true });
      cachedSections.set(row.importBatch, cached.section);
    } else {
      cacheMeta.set(row.importBatch, { fingerprint: cached?.fingerprint ?? null, cached: false });
    }
  }
  const latestBatch = index[0]?.importBatch ?? null;

  let latestSection: AaSection | null = null;
  if (latestBatch && cachedSections.has(latestBatch)) {
    latestSection = cachedSections.get(latestBatch) ?? null;
  } else if (
    latestBatch &&
    lastGood &&
    lastGood.batch === latestBatch &&
    lastGood.configFingerprint === fingerprint &&
    lastGood.fingerprint
  ) {
    // Reuse the last good latest section without re-running the full query.
    latestSection = lastGood.section;
    const cached = await writeAaBatchCache({
      batch: latestBatch,
      configFingerprint: fingerprint,
      fingerprint: lastGood.fingerprint,
      pricesFingerprint: aaPricesFingerprint(lastGood.section.prices ?? []),
      builtAt: latestSection.generatedAt,
      section: latestSection,
    });
    cacheMeta.set(latestBatch, { fingerprint: lastGood.fingerprint, cached });
  } else if (latestBatch) {
    try {
      latestSection = await assemble(await fetchAaPayload(token, ref, latestBatch));
      if (!latestSection) return degrade("Supabase payload failed its structural check");
    } catch (err) {
      const msg = err instanceof Error ? err.message : "error";
      return degrade(msg.startsWith("Supabase query failed") ? msg : `Supabase unreachable (${msg})`);
    }
  }
  if (!latestSection) return degrade("no AA payload for the latest workbook batch");

  await writeAaLastGood({
    batch: latestSection.batch,
    configFingerprint: fingerprint,
    fingerprint: cacheMeta.get(latestSection.batch)?.fingerprint ?? undefined,
    section: latestSection,
  });

  // Build a bounded number of missing historical payloads per tick. The
  // budget is consumed before each fetch, so a failing or unassemblable
  // batch still counts against the tick limit — an upstream outage can
  // never turn this loop into an unbounded run of 60 s queries.
  let attempts = 0;
  for (const row of index) {
    if (attempts >= AA_HISTORICAL_BUILDS_PER_TICK) break;
    if (row.importBatch === latestSection.batch) continue;
    if (cacheMeta.get(row.importBatch)?.cached) continue;
    attempts++;
    try {
      await assemble(await fetchAaPayload(token, ref, row.importBatch));
    } catch (err) {
      console.error(`collector: AA historical batch build failed (${err instanceof Error ? err.message : "error"})`);
    }
  }

  const batches: AaBatchSummary[] = summarizeBatches(index, cacheMeta);
  const withIndex: AaSection = { ...latestSection, batches };
  // #3302 review: the last-good payload must carry the selector index.
  // Re-writing it after the index exists means a later transient failure
  // degrades to a page that still offers every cached version, instead
  // of silently losing the picker until the next successful collection.
  await writeAaLastGood({
    batch: withIndex.batch,
    configFingerprint: fingerprint,
    fingerprint: cacheMeta.get(withIndex.batch)?.fingerprint ?? undefined,
    section: withIndex,
  });
  return withIndex;
}

async function main(): Promise<void> {
  const now = new Date();
  const sectionStatuses: NonNullable<FleetFeed["meta"]>["sections"] = {};
  const markSection = (key: string, status: FeedSectionStatus, error?: unknown) => {
    sectionStatuses[key] = feedSectionStatus(status, new Date().toISOString(), error);
  };
  const isMissingInput = (error: unknown): boolean =>
    typeof error === "object" && error !== null && "code" in error && error.code === "ENOENT";
  // #1116: true when this run's attention entries came from the
  // last-good file rather than a live server read.
  let attentionCarried = false;
  async function readAttentionLastGood(): Promise<SessionFeedEntry[] | null> {
    try {
      const parsed: unknown = JSON.parse(await readFile(attentionLastGoodPath, "utf8"));
      if (!parsed || typeof parsed !== "object") return null;
      const rec = parsed as { at?: unknown; entries?: unknown };
      if (typeof rec.at !== "string" || !Array.isArray(rec.entries) || rec.entries.length === 0) return null;
      return rec.entries as SessionFeedEntry[];
    } catch {
      return null;
    }
  }
  async function writeAttentionLastGood(entries: SessionFeedEntry[]): Promise<void> {
    try {
      await writeFile(attentionLastGoodPath, JSON.stringify({ at: new Date().toISOString(), entries }, null, 2));
    } catch {
      console.error("collector: attention last-good write failed; a failed read next run has nothing to carry");
    }
  }
  // Identity-registry records (#906 §3), accumulated across sources below.
  const registry: SessionRecord[] = [];

  // --- quota -------------------------------------------------------------
  let pools: QuotaPool[] = [];
  let quotaViewText = "";
  try {
    quotaViewText = await readFile(quotaViewPath, "utf8");
    // Pass-through (Drew, 2026-11-03): pool assembly applies NO
    // disabled_providers filter. The #1114 filter dropped every pool
    // whose direct config door was disabled — including live pools
    // burned through model routes and measured by OpenCode (meta, the
    // Antigravity splits, commandcode, anthropic:claude-pro, deepseek
    // at removal time). The board displays the OpenCode quota read;
    // the dated reads extracted here are only its fail-soft fallback.
    pools = extractQuotaPools(quotaViewText, now);
    markSection("quota", "ok");
  } catch (err) {
    console.error(`collector: quota view unreadable (${sanitizeCollectionError(err)}); emitting no pools`);
    markSection("quota", "error", err);
  }

  // --- usage & billing hub sections (design §2.3: ride the same feed) ---
  let saas: SaasService[] | undefined;
  let uninstrumentedServices: ReturnType<typeof extractUninstrumentedServices> | undefined;
  let payg: PaygLine[] | undefined;
  let charges: ChargeRow[] | undefined;
  let subscriptions: FleetFeed["subscriptions"];
  if (quotaViewText) {
    let registryText = "";
    try {
      registryText = await readFile(quotaRegistryPath, "utf8");
    } catch {
      console.error("collector: provider quota registry unreadable; hub PAYG/charges lose the registry reads");
    }
    saas = extractSaasServices(quotaViewText);
    payg = extractPaygLines(quotaViewText, registryText);
    charges = extractCharges(quotaViewText, registryText);
    subscriptions = extractSubscriptions(quotaViewText);
    for (const key of ["saas", "payg", "charges", "subscriptions"]) markSection(key, "ok");
  } else {
    for (const key of ["saas", "payg", "charges", "subscriptions"]) markSection(key, "skipped");
  }
  try {
    const trackerText = await readFile(freeServicesTrackerPath, "utf8");
    uninstrumentedServices = extractUninstrumentedServices(trackerText);
  } catch {
    console.error("collector: free-services tracker unreadable; quota registry omitted");
  }

  // --- charges auto-matching (#1057) -------------------------------------
  // The producer's candidates (Plaid + Gmail) run through the pure
  // matcher; auto rows merge over the hand-fed rows above, and
  // unmatched SaaS/AI charges surface exactly once via the persisted
  // surfaced-state. Fail-soft like every other section: no readable
  // candidates file = the hand-fed charges stand as they were.
  let chargesAttention: UnmatchedCharge[] | undefined;
  try {
    const candidates = parseChargeCandidates(JSON.parse(await readFile(chargeCandidatesPath, "utf8")));
    if (candidates.length) {
      const auto = matchCharges(candidates, chargeServiceCatalog(saas ?? [], pools));
      charges = mergeChargeRows(auto.rows, charges ?? []);
      let surfaced = new Set<string>();
      try {
        const state: unknown = JSON.parse(await readFile(chargesSurfacedStatePath, "utf8"));
        const keys = (state as { surfaced?: unknown }).surfaced;
        if (Array.isArray(keys)) surfaced = new Set(keys.filter((k): k is string => typeof k === "string"));
      } catch {
        /* no state yet: everything unmatched is new */
      }
      const { fresh } = filterNewUnmatched(auto.unmatchedSaas, surfaced);
      if (fresh.length) {
        chargesAttention = fresh;
      }
      console.log(
        `collector: charges auto-match — ${auto.rows.length} rows from ${candidates.length} candidates, ${fresh.length} new unmatched surfaced`,
      );
      markSection("chargesAttention", "ok");
    } else {
      markSection("chargesAttention", "skipped");
    }
  } catch (err) {
    console.error("collector: charge candidates unreadable; hub charges stay hand-fed only this run");
    markSection("chargesAttention", isMissingInput(err) ? "skipped" : "error", err);
  }

  // --- server attention + burn -------------------------------------------
  let attention: SessionFeedEntry[] = [];
  const activePoolTokens: PoolTokenObservation[] = [];
  try {
    const password = (await readFile(passwordFile, "utf8")).trim();
    const mapped = await mapServerSessions({ baseUrl: serverUrl, password }, now.getTime(), {
      source: sessionSource,
    });
    if (mapped) {
      attention = mapped.entries;
      if (attention.length) await writeAttentionLastGood(attention);
      for (const e of mapped.entries) {
        registry.push(recordFromSessionEntry(e, mapped.tokensBySession.get(e.sessionId)));
      }
      // Burn attribution: sessions the server holds active right now.
      const simple = mapped.entries.map((e) => ({
        id: e.sessionId,
        provider: e.provider,
        model: e.model,
      }));
      const burn = poolBurn(simple, mapped.tokensBySession, mapped.activeIds);
      for (const pool of pools) {
        const sessions: { session: string; model: string; tokens: number }[] = [];
        for (const [provider, list] of burn) {
          for (const s of list) {
            if (poolFor(provider, s.model) === pool.id) sessions.push(s);
          }
        }
        if (sessions.length) {
          pool.activeSessions = sessions;
          // #3933 live token source: this pool's measured token
          // total at this instant, for the burn-history rate.
          activePoolTokens.push({
            pool: pool.id,
            provider: pool.provider,
            activeTokens: sessions.reduce((sum, s) => sum + s.tokens, 0),
            // Per-session cumulative counters behind the total, so
            // the rate derivation can diff MATCHING sessions only
            // (PR #7354 Codex round 2) instead of the raw totals.
            sessionTokens: Object.fromEntries(sessions.map((s) => [s.session, s.tokens])),
            at: now.toISOString(),
          });
        }
      }
      markSection("attention", "ok");
    } else {
      const carried = await readAttentionLastGood();
      if (carried) {
        attention = carried;
        attentionCarried = true;
        console.error(
          "collector: OpenCode server unreachable; carrying " + carried.length + " last-good attention entries (#1116)",
        );
      } else {
        console.error("collector: OpenCode server unreachable; attention empty");
      }
      markSection("attention", "error", "OpenCode server unreachable");
    }
  } catch (err) {
    const carried = await readAttentionLastGood();
    if (carried) {
      attention = carried;
      attentionCarried = true;
      console.error(
        "collector: OpenCode server read failed (" +
          sanitizeCollectionError(err) +
          "); carrying " +
          carried.length +
          " last-good attention entries (#1116)",
      );
    } else {
      console.error("collector: OpenCode server read failed (" + sanitizeCollectionError(err) + "); attention empty");
    }
    markSection("attention", "error", err);
  }

  // --- Muse launch records (#906 §3) --------------------------------
  // Muse sessions have no server to scrape: they register by
  // appending one JSON line at launch (scripts/record-claude-launch.ts).
  // Absent file = no Muse sessions recorded yet — the normal state
  // until the helper is used.
  try {
    const jsonl = await readFile(launchRecordsPath, "utf8");
    registry.push(...parseLaunchRecords(jsonl));
  } catch {
    /* no launch records yet */
  }
  markSection("registry", registry.length ? "ok" : "skipped");

  // --- DNS register (#902's files; #906 §1 owns the view) ----------------
  // Slice 2: register.json is authoritative for the fields it carries;
  // the Markdown parse supplies the editorial columns and is the
  // FALLBACK when register.json is missing or malformed — the fallback
  // is logged, never silent (see dnsSectionFromRegisterJson in dns.ts).
  let dns: DnsSection | undefined;
  let dnsSourcePath = dnsRegisterPath;
  let mdText: string | null = null;
  let mdErr: unknown = null;
  try {
    mdText = await readFile(dnsRegisterPath, "utf8");
  } catch (err) {
    mdErr = err;
  }
  const mdEntries = mdText ? parseDnsRegister(mdText) : [];
  let jsonProblem: string | null = null;
  try {
    const jsonText = await readFile(dnsRegisterJsonPath, "utf8");
    dns =
      dnsSectionFromRegisterJson(
        jsonText,
        now.toISOString(),
        "register.json (#902 pipeline) via fleet feed collector",
        mdEntries,
      ) ?? undefined;
    if (!dns) jsonProblem = "register.json did not produce a valid DNS section";
  } catch (err) {
    jsonProblem = err instanceof Error ? err.message : String(err);
  }
  if (dns) {
    dnsSourcePath = dnsRegisterJsonPath;
    markSection("dns", "ok");
  } else {
    console.error(
      `collector: DNS register.json unusable (${jsonProblem ?? "unknown reason"}); FALLING BACK to the register Markdown parse`,
    );
    if (mdText) {
      dns =
        dnsSectionFromRegister(mdText, now.toISOString(), "register file (#902 pipeline) via fleet feed collector") ??
        undefined;
      if (!dns) {
        console.error("collector: DNS register parsed to zero entries; section omitted");
        markSection("dns", "error", "register did not produce a valid DNS section");
      } else {
        markSection("dns", "ok");
      }
    } else {
      console.error("collector: DNS register unreadable; section omitted");
      markSection("dns", isMissingInput(mdErr) ? "skipped" : "error", mdErr);
    }
  }

  // --- orchestration section (#899) --------------------------------------
  let orchestration: OrchestrationSection | undefined;
  try {
    const parsed: unknown = JSON.parse(await readFile(orchestrationPath, "utf8"));
    // Embed the normalized form (#1110): the feed carries the section
    // in the one canonical shape the aggregator serves, whatever
    // provenance form the publisher emitted.
    const normalized = normalizeOrchestrationSection(parsed);
    if (normalized) {
      orchestration = normalized;
      markSection("orchestration", "ok");
    } else {
      console.error(
        `collector: orchestration.json failed its structural check (${orchestrationSectionProblem(parsed)}); section omitted`,
      );
      markSection(
        "orchestration",
        "error",
        `orchestration section failed structural check: ${orchestrationSectionProblem(parsed)}`,
      );
    }
  } catch (err) {
    // No orchestration file yet (the job has not run) is the normal
    // pre-#899 state — omit the section silently.
    markSection("orchestration", isMissingInput(err) ? "skipped" : "error", err);
  }

  // --- GitHub Project pool (#910 stage 5) --------------------------------
  // Read-only GraphQL read of org Project #4. Fail-soft like every
  // other section: no token / read failure = section omitted, and the
  // aggregator's items() falls back to its previous chain.
  let projects: ProjectPoolItem[] | undefined;
  let projectAudit: FleetFeed["projectAudit"];
  try {
    const token = await resolveProjectToken();
    if (!token) {
      console.error("collector: no GitHub Project token available (env or bws); projects section omitted");
      markSection("projects", "skipped");
    } else {
      const snapshot = await fetchProjectSnapshot(token, fetch, now.toISOString());
      projects = snapshot.items;
      projectAudit = snapshot.audit;
      if (!projects.length) {
        // #3979: Project #4 is never legitimately empty, so a zero-item
        // read is a broken read (lost scope, renamed project, shape
        // drift) — mark it error like the DNS zero-entries case instead
        // of reporting ok for a section that was omitted.
        console.error("collector: GitHub Project read returned zero items; projects section omitted");
        projects = undefined;
        markSection("projects", "error", "GitHub Project read returned zero items");
      } else {
        markSection("projects", "ok");
      }
    }
  } catch (err) {
    console.error(`collector: GitHub Project read failed (${sanitizeCollectionError(err)}); projects section omitted`);
    projects = undefined;
    markSection("projects", "error", err);
  }

  // --- Coverage census (#1220) -------------------------------------------
  // Per-repo open-issue + gate-label counts via the Search API, using
  // the same token as the Project read. Fail-soft: a repo whose counts
  // fail is omitted, and a total failure omits the section — the
  // aggregator panel then renders backlog counts as unavailable.
  let coverage: RepoCensus[] | undefined;
  try {
    const token = await resolveProjectToken();
    if (!token) {
      console.error("collector: no GitHub token available; coverage census omitted");
      markSection("coverage", "skipped");
    } else {
      const count = async (q: string): Promise<number> => {
        const res = await fetch(`https://api.github.com/search/issues?per_page=1&q=${encodeURIComponent(q)}`, {
          headers: {
            authorization: `Bearer ${token}`,
            accept: "application/vnd.github+json",
            "user-agent": "pm-aggregator-collector",
          },
        });
        if (!res.ok) throw new Error(`search ${res.status}`);
        const body = (await res.json()) as { total_count?: number };
        if (!Number.isInteger(body.total_count)) throw new Error("search: no total_count");
        return body.total_count as number;
      };
      const rows: RepoCensus[] = [];
      const failedRepos = new Set<string>();
      for (const repo of REPOS) {
        let staleOpenIssues: number | null = null;
        let staleIssuesError: string | null = null;
        let staleIssuesCollectedAt: string;
        try {
          staleOpenIssues = await count(staleIssueSearchQuery(repo, new Date()));
          staleIssuesCollectedAt = new Date().toISOString();
        } catch (err) {
          failedRepos.add(repo);
          staleIssuesCollectedAt = new Date().toISOString();
          staleIssuesError = sanitizeCollectionError(err);
          console.error(`collector: stale-issue census failed for ${repo} (${staleIssuesError})`);
        }
        let openIssues: number | null = null;
        let gateNow: number | null = null;
        let gateNext: number | null = null;
        try {
          const base = `repo:${repo} is:issue is:open`;
          openIssues = await count(base);
          gateNow = await count(`${base} label:now`);
          gateNext = await count(`${base} label:next`);
        } catch (err) {
          failedRepos.add(repo);
          console.error(`collector: coverage census failed for ${repo} (${sanitizeCollectionError(err)}); backlog counts unavailable`);
        }
        rows.push({ repo, openIssues, gateNow, gateNext, staleOpenIssues, staleIssuesCollectedAt, staleIssuesError });
      }
      if (rows.length) coverage = rows;
      if (failedRepos.size === REPOS.length && REPOS.length > 0) {
        markSection("coverage", "error", "all repository census reads failed");
      } else if (failedRepos.size > 0) {
        markSection("coverage", "error", `${failedRepos.size} repository census reads failed`);
      } else {
        markSection("coverage", "ok");
      }
    }
  } catch (err) {
    console.error(`collector: coverage census failed (${sanitizeCollectionError(err)}); section omitted`);
    coverage = undefined;
    markSection("coverage", "error", err);
  }

  // --- AA cost explorer (Supabase ainventory) ------------------------------
  // Always attempted; the section carries its own failure (absent
  // form) so the dashboard can say why instead of showing stale data.
  const collectedAa = await collectAaSection(now.toISOString());
  const aa =
    collectedAa.status === "absent"
      ? { ...collectedAa, reason: sanitizeCollectionError(collectedAa.reason) }
      : collectedAa;
  if (aa.status === "absent") {
    console.error(`collector: AA section absent (${sanitizeCollectionError(aa.reason)})`);
    markSection("aa", "error", aa.reason);
  } else {
    markSection("aa", "ok");
  }

  // --- usage snapshot records (hub §4 writers) ---------------------------
  // Writer (a): records derived from this run's own hub sections. Each
  // carries its read's own date, so an unchanged read re-derives the
  // identical record and the laptop store dedupes it — only genuinely
  // new observations grow the history. Writers (b)/(c) arrive via the
  // outbox drain and ride the same field.
  const snapshotRecords: SnapshotRecord[] = [];
  if (saas) snapshotRecords.push(...recordsFromSaasServices(saas, "fleet-feed-collector"));
  if (payg) snapshotRecords.push(...recordsFromPaygLines(payg, "fleet-feed-collector"));
  if (dns) {
    try {
      const registerStat = await stat(dnsSourcePath);
      snapshotRecords.push(...recordsFromDns(dns, registerStat.mtime.toISOString(), "fleet-feed-collector"));
    } catch {
      console.error("collector: DNS register stat failed; dns snapshot records omitted this run");
    }
  }
  snapshotRecords.push(...(await drainSnapshotOutbox()));
  const dedupedSnapshots: SnapshotRecord[] = [];
  {
    const seen = new Set<string>();
    for (const r of snapshotRecords) {
      const k = snapshotKey(r);
      if (!seen.has(k)) {
        seen.add(k);
        dedupedSnapshots.push(r);
      }
    }
  }
  markSection("snapshots", dedupedSnapshots.length ? "ok" : "skipped");

  // #1116 item 3: this collector runs from the pm-feed checkout of
  // this repo; measure its drift vs origin/main so a stale checkout
  // (collector running old code) is visible in the health view
  // instead of lying quietly. Fail-soft: any git problem becomes
  // state "unknown" with the reason, never a failed collection.
  const checkout = await computeCheckoutDrift(join(PKG, "..", ".."));

  // --- historical burn + measured rate (#3933) -------------------------
  // Persist this run's pool-window observations beside the
  // charges-surfaced state, then derive completed reset-period
  // segments and measured per-pool rates from the series
  // (src/burn-history.ts). Fail-soft like every other section: no
  // readable state = this run's observations only, which honestly
  // yields no segments and no rates yet.
  let burnSegments: BurnSegment[] = [];
  try {
    let priorObservations: PoolWindowObservation[] = [];
    let priorPoolTokens: PoolTokenObservation[] = [];
    try {
      const raw: unknown = JSON.parse(await readFile(burnHistoryStatePath, "utf8"));
      if (Array.isArray(raw)) {
        priorObservations = raw as PoolWindowObservation[];
      } else if (raw && typeof raw === "object") {
        const state = raw as { windows?: unknown; poolTokens?: unknown };
        if (Array.isArray(state.windows)) priorObservations = state.windows as PoolWindowObservation[];
        if (Array.isArray(state.poolTokens)) priorPoolTokens = state.poolTokens as PoolTokenObservation[];
      }
    } catch {
      /* no state yet: history starts with this run */
    }
    const mergedObservations = mergeObservations(
      priorObservations,
      observationsFromPools(pools, now.toISOString()),
      now.getTime(),
    );
    const mergedPoolTokens = mergePoolTokenObservations(priorPoolTokens, activePoolTokens, now.getTime());
    try {
      await writeFile(
        burnHistoryStatePath,
        JSON.stringify({ windows: mergedObservations, poolTokens: mergedPoolTokens }, null, 2),
      );
    } catch {
      console.error("collector: burn-history state write failed; history restarts next run");
    }
    const derived = deriveBurnHistory(mergedObservations);
    burnSegments = derived.segments;
    // Measured rates: tokens-window pairs where a source provides
    // them; otherwise the live session-attribution totals (#3933).
    const sessionRates = deriveMeasuredRatesFromPoolTokens(mergedPoolTokens, now.getTime());
    let ratesSet = 0;
    for (const pool of pools) {
      const rate = derived.burnTokensPerMin.get(pool.id) ?? sessionRates.get(pool.id);
      if (rate !== undefined) {
        pool.burnTokensPerMin = rate;
        ratesSet += 1;
      }
    }
    markSection("burnHistory", burnSegments.length || ratesSet ? "ok" : "skipped");
  } catch (err) {
    console.error("collector: burn-history derivation failed; burn stays empty this run");
    markSection("burnHistory", "error", err);
  }

  // #8568: Drew's answers to needs-Drew questions, from the
  // aggregator's needs-drew store (same default path logic as
  // src/config.ts). Fail-soft: an absent store = no answers yet.
  let answers: NonNullable<FleetFeed["answers"]> = [];
  try {
    const answered = await listAnswers(defaultNeedsDrewFile());
    answers = answered.map((item) => ({
      ledgerId: item.ledgerId,
      ...(item.answer?.optionId !== undefined ? { optionId: item.answer.optionId } : {}),
      ...(item.answer?.freeText !== undefined ? { freeText: item.answer.freeText } : {}),
      answeredAt: item.answer?.at ?? "",
      via: item.answer?.via ?? "api",
    }));
  } catch {
    answers = [];
  }

  const feed: FleetFeed = assembleCollectorFeed({
    generatedAt: now.toISOString(),
    answers,
    checkout,
    meta: { sections: sectionStatuses },
    quota: { pools, burn: burnSegments, choices: computeChoices(pools, now.getTime()) },
    attention,
    ...(orchestration ? { orchestration } : {}),
    ...(dns ? { dns } : {}),
    ...(registry.length ? { registry } : {}),
    ...(saas ? { saas } : {}),
    ...(uninstrumentedServices ? { uninstrumentedServices } : {}),
    ...(payg ? { payg } : {}),
    ...(charges ? { charges } : {}),
    ...(chargesAttention ? { chargesAttention } : {}),
    ...(subscriptions ? { subscriptions } : {}),
    ...(dedupedSnapshots.length ? { snapshots: dedupedSnapshots } : {}),
    ...(projects ? { projects } : {}),
    ...(projectAudit ? { projectAudit } : {}),
    ...(coverage ? { coverage } : {}),
    aa,
  });
  const freshnessUrl = process.env.PM_AGG_FRESHNESS_URL ?? "http://127.0.0.1:4871/publish";
  await publishFleetFeedWithInvalidation(outPath, JSON.stringify(feed, null, 2), feed.generatedAt, async (input) => {
    const response = await fetch(freshnessUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      signal: AbortSignal.timeout(5_000),
    });
    if (!response.ok) throw new Error(`freshness publisher returned ${response.status}`);
  });
  const withReads = pools.filter((p) => p.windows.length > 0).length;
  console.log(
    `collector: wrote ${outPath} — ${pools.length} pools (${withReads} with reads), ${attention.length} sessions (${sessionSource}${attentionCarried ? ", carried last-good" : ""})` +
      (orchestration ? `, orchestration ${orchestration.runId}` : ", no orchestration section") +
      (dns ? `, dns ${dns.entries.length} domains` : ", no dns section") +
      (saas
        ? `, hub ${saas.length} services / ${payg?.length ?? 0} payg / ${charges?.length ?? 0} charges`
        : ", no hub sections") +
      (dedupedSnapshots.length ? `, snapshots ${dedupedSnapshots.length} records` : "") +
      (projectAudit ? `, projects ${projectAudit.totalItems} total / ${projects?.length ?? 0} resolvable` : ", no projects section") +
      (coverage ? `, coverage ${coverage.length} repos` : ", no coverage section") +
      (aa.status === "ok"
        ? `, aa ${aa.rankings?.length ?? 0} rankings / ${aa.configs?.length ?? 0} configs / ${aa.testRuns?.length ?? 0} test runs`
        : ", aa absent") +
      `, registry ${registry.length} records`,
  );
}

if (import.meta.main) await main();
