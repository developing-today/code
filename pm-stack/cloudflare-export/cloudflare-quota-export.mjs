#!/usr/bin/env bun
/**
 * cloudflare-quota-export — Cloudflare live collector, delivered as the
 * quota-gatherer's file-export adapter for developing-today/code#1048
 * (epic #1047; cadence per #1151: the gatherer's 15-minute tick, NOT a
 * standalone hourly job).
 *
 * Runs ON Drew's laptop under Bun (Node's fetch/https egress times out
 * on this laptop — observed 2026-10-29; Bun and curl connect fine).
 * The day-to-day API token is fetched by the wrapper
 * (cloudflare-quota-export.sh) from Bitwarden Secrets Manager by NAME
 * (`cloudflare.api-token-day-to-day`) into this process's environment
 * only — the value never lands in a file, log, or payload.
 *
 * Every run writes the export document atomically to
 *   <out-dir>/latest.json     (default ~/atlas/cloudflare-export/)
 * and appends the same document to <out-dir>/history.jsonl.
 *
 * Success document (the file-export format the #1151 gatherer uses;
 * source label fixed as "file-export/cloudflare"):
 *   { service: "cloudflare", source: "file-export/cloudflare",
 *     status: "ok", measuredAt: <ISO>, cycleStart: <ISO>,
 *     accountId: "...", billing: {...},
 *     metrics: [ { id, used, unit, source } ] }
 * Metric ids match the aggregator's saas-extract cloudflare card, so
 * the aggregator can prefer these live values over tracker prose.
 *
 * Billing: the subscriptions/billing endpoints return 403 with the
 * day-to-day token (probed live 2026-10-29, #1048) — the issue's own
 * sanctioned fallback settles billing as invoice-only, recorded in the
 * document's `billing` block with the probe evidence. Creating a new
 * token is Drew's account action and out of scope here.
 *
 * Failure documents carry status + attemptedAt + detail and NO
 * metrics/values — a failed tick can never be mistaken for a reading:
 *   no-token       CLOUDFLARE_API_TOKEN not in the environment. Exit 2.
 *   token-scope    the API answered 401/403 — the token lacks the
 *                  analytics scopes; `missingScopes` names them. Exit 3.
 *   unparseable    an endpoint answered an unrecognised shape. Exit 6.
 *   fetch-failed   network/other error. Exit 7.
 */
import { appendFile, mkdir, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

export const SOURCE = "file-export/cloudflare";
/**
 * Per-request deadline for Cloudflare API calls. The systemd unit
 * (cloudflare-quota-export.service) kills the exporter at
 * TimeoutStartSec=120, and Bun's default fetch idle timeout is five
 * minutes - without an explicit bound, a stalled request is killed
 * together with the process before main() can write its failure
 * document, leaving a stale latest.json inside the aggregator's
 * freshness window. 30s leaves room for the run's sequential calls
 * plus the failure write.
 */
export const API_REQUEST_TIMEOUT_MS = 30_000;
export const API_BASE = "https://api.cloudflare.com/client/v4";
/** The analytics scopes the day-to-day token needs for this collector. */
export const ANALYTICS_SCOPES = [
  "Account Analytics Read (GraphQL analytics: workersInvocationsAdaptive, kvOperationsAdaptiveGroups, d1AnalyticsAdaptiveGroups, queueMessageOperationsAdaptiveGroups, r2OperationsAdaptiveGroups, r2StorageAdaptiveGroups, aiInferenceAdaptiveGroups)",
  "Workers Scripts Read / KV Storage Read / D1 Read / Queues Read / R2 Read (REST inventory + bucket enumeration)",
];

export class ExportError extends Error {
  constructor(status, exitCode, detail, extra = {}) {
    super(detail);
    this.status = status;
    this.exitCode = exitCode;
    this.extra = extra;
  }
}

const round2 = (n) => Math.round(n * 100) / 100;

/** Start of the current cycle: calendar month, UTC. The paid monthly
 * reset is the subscription date, which is unknown (quota-view Part
 * (b)); calendar-month UTC is the documented approximation and the
 * document carries cycleStart so consumers can see the basis. */
export function cycleStartUtc(now = new Date()) {
  return new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1));
}

const PLAIN_DECIMAL = /^[0-9]+(?:\.[0-9]+)?$/;
const DECIMAL_COUNT = /^[0-9]+$/;

const num = (v, field, { integerString = false } = {}) => {
  const validString = typeof v === "string" &&
    (integerString ? DECIMAL_COUNT.test(v) : PLAIN_DECIMAL.test(v));
  const n = typeof v === "number" ? v : validString ? Number(v) : NaN;
  if (!Number.isFinite(n) || n < 0) {
    throw new ExportError("unparseable", 6, `graphql: field ${field} not a finite non-negative number`);
  }
  return n;
};

/** R2 operation classification per Cloudflare's published pricing
 * classes. Deletes and aborts are free (neither class). An action type
 * in neither list is reported via `unclassified` — never guessed. */
export const R2_CLASS_A = new Set([
  "ListBuckets", "ListObjects", "ListMultipartUploads", "ListParts",
  "CreateBucket", "PutBucket", "PutObject", "CopyObject",
  "CreateMultipartUpload", "CompleteMultipartUpload", "UploadPart",
  "UploadPartCopy", "PutBucketEncryption", "PutBucketCors",
  "LifecycleStorageTierTransition",
  "PutBucketLifecycleConfiguration", "PutBucketLock",
]);
export const R2_CLASS_B = new Set([
  "HeadBucket", "HeadObject", "GetObject", "GetBucketLocation",
  "UsageSummary", "GetBucketEncryption", "GetBucketCors",
  "GetBucketLifecycleConfiguration", "GetBucketVersioning",
  "GetBucketLock",
]);
export const R2_FREE = new Set([
  "DeleteObject", "DeleteBucket", "AbortMultipartUpload",
  "DeleteBucketEncryption", "DeleteBucketCors",
  "DeleteBucketLifecycleConfiguration",
]);

/** Response statuses Cloudflare does not bill for R2 (pricing FAQ:
 * HTTP 401 and authentication/permission-related HTTP 403 are excluded
 * from billing). R2 403s are authorization failures, so both codes are
 * dropped here; every other failure (404, 409, 5xx, ...) stays counted,
 * because those remain billable. Groups without a responseStatusCode
 * dimension (older payloads) are counted as before. */
export const R2_NONBILLABLE_STATUSES = new Set([401, 403]);

export function classifyR2(groups) {
  // Fail closed: a missing/null groups field is shape drift, not "zero
  // operations". Only a present, empty array means no R2 requests.
  if (!Array.isArray(groups)) {
    throw new ExportError("unparseable", 6, "graphql: r2OperationsAdaptiveGroups missing");
  }
  let classA = 0;
  let classB = 0;
  let nonbillable = 0;
  const unclassified = [];
  for (const g of groups) {
    const action = g?.dimensions?.actionType;
    const requests = num(g?.sum?.requests, `r2.${String(action)}.requests`, { integerString: true });
    const status = g?.dimensions?.responseStatusCode;
    if (status != null && R2_NONBILLABLE_STATUSES.has(Number(status))) {
      nonbillable += requests;
      continue;
    }
    if (R2_CLASS_A.has(action)) classA += requests;
    else if (R2_CLASS_B.has(action)) classB += requests;
    else if (R2_FREE.has(action)) continue;
    else unclassified.push(action);
  }
  return { classA, classB, unclassified, nonbillable };
}

/** Fold KV action types onto the card's two metrics: reads, and
 * writes/deletes/lists combined (the card's own grouping). */
export function foldKv(groups) {
  // Same fail-closed rule as firstSum/classifyR2: a missing KV groups
  // field must not be published as a successful zero reading.
  if (!Array.isArray(groups)) {
    throw new ExportError("unparseable", 6, "graphql: kvOperationsAdaptiveGroups missing");
  }
  let reads = 0;
  let writes = 0;
  for (const g of groups) {
    const requests = num(g?.sum?.requests, `kv.${String(g?.dimensions?.actionType)}.requests`, { integerString: true });
    if (g?.dimensions?.actionType === "read") reads += requests;
    else writes += requests; // write, delete, list — and any future write-side action
  }
  return { reads, writes };
}

const firstSum = (groups, field, label, { integerString = true } = {}) => {
  if (!Array.isArray(groups)) {
    throw new ExportError("unparseable", 6, `graphql: ${label} groups missing`);
  }
  let total = 0;
  for (const g of groups) total += num(g?.sum?.[field], `${label}.${field}`, { integerString });
  return total;
};

/** Parse one combined GraphQL analytics payload into export metrics.
 * Fail-closed: any recognised-shape violation throws; GraphQL-level
 * errors are handled by the caller (they arrive out-of-band). */
export function parseGraphqlUsage(account, { now = new Date(), cycleStart } = {}) {
  const src = "cloudflare-graphql-analytics";
  const metrics = [];
  const push = (id, used, unit, extra = {}) =>
    metrics.push({ id, used, unit, source: src, ...extra });

  const workers = account?.workersInvocationsAdaptive;
  push("workers-requests", firstSum(workers, "requests", "workers"), "count");
  push("workers-cpu-ms", Math.round(firstSum(workers, "cpuTimeUs", "workers") / 1000), "ms");

  const kv = foldKv(account?.kvOperationsAdaptiveGroups);
  push("kv-reads", kv.reads, "count");
  push("kv-writes", kv.writes, "count");

  push("d1-rows-read", firstSum(account?.d1AnalyticsAdaptiveGroups, "rowsRead", "d1"), "rows");
  push("d1-rows-written", firstSum(account?.d1AnalyticsAdaptiveGroups, "rowsWritten", "d1"), "rows");

  push("queues-ops", firstSum(account?.queueMessageOperationsAdaptiveGroups, "billableOperations", "queues"), "ops");

  const r2 = classifyR2(account?.r2OperationsAdaptiveGroups);
  push("r2-class-a", r2.classA, "ops");
  push("r2-class-b", r2.classB, "ops");

  const storageGroups = account?.r2StorageAdaptiveGroups;
  if (!Array.isArray(storageGroups)) {
    throw new ExportError("unparseable", 6, "graphql: r2StorageAdaptiveGroups missing");
  }
  // An empty array is legitimate: the storage dataset holds one
  // observation per bucket, so an account with no bucket observations
  // reports zero storage - it must not fail the whole export and
  // discard the other services' readings.
  // payloadSize is per bucket: each group is one bucket's peak over the
  // window, so the account total is the SUM of the per-bucket peaks —
  // an account-wide max would report the largest single bucket only.
  let payloadBytes = 0;
  for (const g of storageGroups) {
    payloadBytes += num(g?.max?.payloadSize, "r2storage.payloadSize");
  }
  const saturated = storageGroups.length >= R2_STORAGE_GROUP_LIMIT;
  push("r2-storage", round2(payloadBytes / 1e9), "gb", {
    // Explicit marker (not just prose) so consumers can qualify derived
    // percentages as "at least N%" instead of understating them.
    ...(saturated ? { lowerBound: true } : {}),
    detail:
      (storageGroups.length === 0
        ? "no R2 bucket storage observations in the cycle window (r2StorageAdaptiveGroups empty); reporting zero "
        : "") +
      `sum of per-bucket max payloadSize ${payloadBytes} bytes across ${storageGroups.length} bucket group(s) in the cycle window (r2StorageAdaptiveGroups)` +
      (saturated
        ? ` — group count hit the ${R2_STORAGE_GROUP_LIMIT} query limit, so buckets beyond it are excluded and this total is a LOWER BOUND`
        : ""),
  });

  // Neurons are a fractional measurement (fixture total 4114515.31), not a request count: plain-decimal strings stay valid here.
  const neuronsTotal = firstSum(account?.aiInferenceAdaptiveGroups, "totalNeurons", "ai", { integerString: false });
  const daysElapsed = Math.max(1, (now - cycleStart) / 86_400_000);
  push("workers-ai-neurons", Math.round(neuronsTotal / daysElapsed), "neurons", {
    detail: `per-day average over the cycle to date; cycle total ${Math.round(neuronsTotal)} neurons`,
  });

  return { metrics, r2Unclassified: r2.unclassified };
}

/** Classify the billing probe pair (billing/profile + subscriptions).
 * Both 403 (the known day-to-day-token result) settles invoice-only —
 * the issue's sanctioned fallback. A 200 means billing became readable
 * and the document says so instead of assuming. */
export function classifyBilling(probes) {
  const list = probes.map((p) => ({ endpoint: p.endpoint, status: p.status }));
  if (list.every((p) => p.status === 403)) {
    return {
      status: "invoice-only",
      probes: list,
      detail:
        "subscriptions/billing endpoints returned 403 with the day-to-day token (probed 2026-10-29 for #1048); billed state is the monthly invoice only — the metrics in this document are not-yet-billed current-cycle consumption",
    };
  }
  if (list.every((p) => p.status === 200)) {
    return { status: "readable", probes: list, detail: "billing endpoints answered 200" };
  }
  return {
    status: "unknown",
    probes: list,
    detail: "billing probes disagreed or errored; treating billed state as unknown, not invoice-only",
  };
}

export function successDocument({ measuredAt, cycleStart, accountId, billing, parsed, r2Buckets }) {
  return {
    service: "cloudflare",
    source: SOURCE,
    status: "ok",
    measuredAt,
    cycleStart: cycleStart.toISOString(),
    accountId,
    billing,
    r2Buckets,
    ...(parsed.r2Unclassified?.length ? { r2UnclassifiedActions: parsed.r2Unclassified } : {}),
    metrics: parsed.metrics,
  };
}

export function failureDocument({ attemptedAt, status, detail, extra = {} }) {
  return {
    service: "cloudflare",
    source: SOURCE,
    status,
    measuredAt: null,
    attemptedAt,
    detail,
    ...extra,
  };
}

// ---------- live API (only used by the main run) ----------

async function apiJson(path, token, init = {}) {
  let res;
  try {
    res = await fetch(`${API_BASE}${path}`, {
      ...init,
      signal: AbortSignal.timeout(API_REQUEST_TIMEOUT_MS),
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    });
  } catch (e) {
    throw new ExportError("fetch-failed", 7, `${path}: ${e.message}`);
  }
  let body = null;
  try {
    body = await res.json();
  } catch {
    /* status-only handling below */
  }
  return { status: res.status, body };
}

function scopeError(what, status) {
  return new ExportError(
    "token-scope",
    3,
    `${what}: HTTP ${status} — the day-to-day token lacks a scope this collector needs`,
    { missingScopes: ANALYTICS_SCOPES },
  );
}

// One group per bucket: the group limit must cover every bucket on the
// account or the storage total silently drops the rest. 10000 is the
// GraphQL Analytics API's maximum limit; parseGraphqlUsage flags the
// total as a lower bound if a result ever saturates it.
export const R2_STORAGE_GROUP_LIMIT = 10000;

const GRAPHQL_QUERY = `query($a: string!, $s: Time!, $e: Time!) {
  viewer {
    accounts(filter: { accountTag: $a }) {
      workersInvocationsAdaptive(limit: 100, filter: { datetime_geq: $s, datetime_leq: $e }) { sum { requests cpuTimeUs } }
      kvOperationsAdaptiveGroups(limit: 100, filter: { datetime_geq: $s, datetime_leq: $e }) { dimensions { actionType } sum { requests } }
      d1AnalyticsAdaptiveGroups(limit: 100, filter: { datetime_geq: $s, datetime_leq: $e }) { sum { rowsRead rowsWritten } }
      queueMessageOperationsAdaptiveGroups(limit: 100, filter: { datetime_geq: $s, datetime_leq: $e }) { sum { billableOperations } }
      r2OperationsAdaptiveGroups(limit: 1000, filter: { datetime_geq: $s, datetime_leq: $e }) { dimensions { actionType responseStatusCode } sum { requests } }
      r2StorageAdaptiveGroups(limit: 10000, filter: { datetime_geq: $s, datetime_leq: $e }) { dimensions { bucketName } max { payloadSize } }
      aiInferenceAdaptiveGroups(limit: 100, filter: { datetime_geq: $s, datetime_leq: $e }) { sum { totalNeurons } }
    }
  }
}`;

export async function run({ token, accountId, now = new Date() } = {}) {
  if (!token) {
    throw new ExportError(
      "no-token",
      2,
      "CLOUDFLARE_API_TOKEN is not set — run via cloudflare-quota-export.sh, which fetches the Bitwarden secret named cloudflare.api-token-day-to-day into the environment",
    );
  }
  let acct = accountId ?? process.env.CLOUDFLARE_ACCOUNT_ID ?? null;
  if (!acct) {
    // Ask for two so a multi-account token is visible: picking result[0]
    // silently would export an arbitrary account's usage.
    const accounts = await apiJson("/accounts?per_page=2", token);
    if (accounts.status === 401 || accounts.status === 403) throw scopeError("GET /accounts", accounts.status);
    const visible = accounts.body?.result;
    if (!Array.isArray(visible)) {
      throw new ExportError("unparseable", 6, "GET /accounts: result is not a list");
    }
    if (visible.length !== 1) {
      throw new ExportError(
        "unparseable",
        6,
        `GET /accounts: token sees ${visible.length} account(s); set CLOUDFLARE_ACCOUNT_ID to choose one`,
      );
    }
    acct = visible[0]?.id;
    if (!acct) throw new ExportError("unparseable", 6, "GET /accounts: no account in result");
  }

  // Billing probe first: its result is documentation, never a blocker
  // for the usage metrics (invoice-only is a settled state, #1048).
  const probePaths = [`/accounts/${acct}/billing/profile`, `/accounts/${acct}/subscriptions`];
  const probes = [];
  for (const p of probePaths) {
    // Billing is informational and nonblocking (#1048): a transport
    // failure on an optional probe records billing as unknown
    // (status 0) and usage collection continues — it must not abort
    // run() and replace all usage metrics with a failure document.
    try {
      const r = await apiJson(p, token);
      probes.push({ endpoint: p.replace(acct, "{account}"), status: r.status });
    } catch {
      probes.push({ endpoint: p.replace(acct, "{account}"), status: 0 });
    }
  }
  const billing = classifyBilling(probes);

  // R2 bucket enumeration directly (names only) — the storage figure
  // itself comes from the analytics dataset, not the tracker prose.
  const buckets = await apiJson(`/accounts/${acct}/r2/buckets?per_page=100`, token);
  if (buckets.status === 401 || buckets.status === 403) throw scopeError("GET r2/buckets", buckets.status);
  const r2Buckets = (buckets.body?.result?.buckets ?? []).map((b) => b.name).filter(Boolean);

  const start = cycleStartUtc(now);
  const gql = await apiJson("/graphql", token, {
    method: "POST",
    body: JSON.stringify({
      query: GRAPHQL_QUERY,
      variables: { a: acct, s: start.toISOString(), e: now.toISOString() },
    }),
  });
  if (gql.status === 401 || gql.status === 403) throw scopeError("POST /graphql", gql.status);
  if (gql.status !== 200) throw new ExportError("fetch-failed", 7, `POST /graphql: HTTP ${gql.status}`);
  // GraphQL errors: an array of {message: string} is the documented shape,
  // and a single object with a string message is the same shape unwrapped -
  // normalise it to the array so it classifies identically (#2608). Any
  // other drift (errors as a string, null/non-object entries, a missing or
  // non-string message) is a shape this parser does not know: fail closed
  // as unparseable instead of throwing a bare TypeError that main() would
  // misclassify as fetch-failed, or silently ignoring the error.
  const rawErrors = gql.body?.errors ?? [];
  const gqlErrors = Array.isArray(rawErrors) ? rawErrors : [rawErrors];
  for (const e of gqlErrors) {
    if (!e || typeof e !== "object" || typeof e.message !== "string") {
      throw new ExportError(
        "unparseable",
        6,
        "POST /graphql: malformed errors shape (expected an array of {message: string})",
      );
    }
  }
  if (gqlErrors.length) {
    const msg = gqlErrors.map((e) => e.message.split("\n")[0]).join(" ; ");
    if (/auth|permission|scope/i.test(msg)) throw scopeError("POST /graphql", 403);
    throw new ExportError("unparseable", 6, `POST /graphql: ${msg}`);
  }
  const account = gql.body?.data?.viewer?.accounts?.[0];
  if (!account) throw new ExportError("unparseable", 6, "POST /graphql: no account node in data");
  const parsed = parseGraphqlUsage(account, { now, cycleStart: start });
  return successDocument({
    measuredAt: now.toISOString(),
    cycleStart: start,
    accountId: acct,
    billing,
    parsed,
    r2Buckets,
  });
}

async function writeOutputs(outDir, doc) {
  await mkdir(outDir, { recursive: true });
  const latest = join(outDir, "latest.json");
  const tmp = join(outDir, `.latest.json.tmp-${process.pid}`);
  await writeFile(tmp, `${JSON.stringify(doc, null, 2)}\n`, "utf8");
  await rename(tmp, latest);
  await appendFile(join(outDir, "history.jsonl"), `${JSON.stringify(doc)}\n`, "utf8");
}

async function main() {
  const args = process.argv.slice(2);
  const opt = (name, fallback) => {
    const i = args.indexOf(name);
    return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
  };
  const outDir = opt("--out-dir", join(homedir(), "atlas", "cloudflare-export"));
  try {
    const doc = await run({
      token: process.env.CLOUDFLARE_API_TOKEN,
      accountId: opt("--account-id", undefined),
    });
    await writeOutputs(outDir, doc);
    console.log(JSON.stringify({ status: doc.status, measuredAt: doc.measuredAt, metrics: doc.metrics.length, outDir }));
  } catch (e) {
    const err =
      e instanceof ExportError ? e : new ExportError("fetch-failed", 7, `unexpected: ${e.message ?? String(e)}`);
    const doc = failureDocument({
      attemptedAt: new Date().toISOString(),
      status: err.status,
      detail: err.message,
      extra: err.extra,
    });
    try {
      await writeOutputs(outDir, doc);
    } catch (writeErr) {
      console.error(`cloudflare-quota-export: could not write outputs: ${writeErr.message}`);
    }
    console.error(`cloudflare-quota-export: ${err.status}: ${err.message}`);
    process.exit(err.exitCode);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
