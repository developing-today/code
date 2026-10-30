#!/usr/bin/env node
/**
 * supabase-quota-export — Supabase usage exporter (developing-today/code#1050,
 * follow-up to triage task 2313). The Supabase half of the #1151 file-export
 * family, in the #1048 Cloudflare adapter shape: a producer that runs on the
 * laptop tick and writes the document the aggregator prefers over quota-view
 * prose.
 *
 * Every run writes atomically to
 *   <out-dir>/latest.json     (default ~/atlas/supabase-export/)
 * and appends the same document to <out-dir>/history.jsonl.
 *
 * Success document (file-export format the #1151 gatherer ingests):
 *   { service: "supabase", source: "file-export/supabase",
 *     status: "ok" | "partial",
 *     measuredAt: <ISO>, project: "muse-dev", ref: <project ref>,
 *     metrics: [ { id: "db-size", used: 1.15, limit: 0.5, unit: "gb",
 *                  source: "management-api:/database/query pg_database_size",
 *                  flag?: "approaching-paid-trigger", detail? } ],
 *     billing: { plan: "free", billedUsd: 0, measured: true, source: ... },
 *     scope: [ { endpoint, path, status, note? } ] }
 *
 * Failure documents carry status + attemptedAt + detail and NO metrics or
 * billing values — a failed tick can never be mistaken for a reading:
 *   no-token        the Bitwarden secret did not resolve. Exit 3.
 *   scope-denied    GET /projects/:ref answered 401/403. Exit 4.
 *   fetch-failed    network/other error. Exit 5.
 *   unparseable     an endpoint answered a shape this parser does not know.
 *                   Exit 6.
 *
 * The management token is resolved by NAME from Bitwarden Secrets Manager at
 * runtime and sent only to api.supabase.com. Its value is never read into a
 * document, log line, fixture, or argv.
 *
 * Scope probe (recorded endpoint-by-endpoint in the `.scope` array of every
 * successful document; task #2985 re-ran it against the live token):
 *   GET /projects                                  200  (auth works)
 *   GET /projects/:ref                             200
 *   GET /projects/:ref/database/query              201  (db size SQL)
 *   GET /projects/:ref/analytics/endpoints/usage.api-counts          200
 *   GET /projects/:ref/analytics/endpoints/usage.api-requests-count  200
 *   GET /projects/:ref/billing/addons              200  (measured $0 addons)
 *   GET /projects/:ref/storage/buckets             200
 *   GET /organizations/:slug                       403  scope gap: org-scope
 *   GET /organizations/:slug/{billing,usage}       403/404  scope gap
 *   GET /projects/:ref/{usage,database/size}       404  no such endpoint
 *   egres bytes        no endpoint exists (request counts only) — documented gap
 */
import { execFile } from "node:child_process";
import { appendFile, mkdir, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

export const SOURCE = "file-export/supabase";
export const DEFAULT_PROJECT_REF = "nltgxorqwxhgfzseiirt"; // muse-dev (#1050)
export const DEFAULT_SECRET_NAME = "supabase.access-token-muse-cli";
export const API_BASE = "https://api.supabase.com/v1";

/** Supabase Free-plan allowances (supabase.com/pricing, recorded 2026-10-29:
 * 500 MB database, 1 GB file storage, 5 GB egress). There is no API read of
 * the numbers themselves; the API measures usage against them. */
export const FREE_LIMITS = {
  "db-size": 0.5,
  egress: 5,
  storage: 1,
};

/** Metric ids at >= this share of their Free limit carry the paid trigger. */
export const PAID_TRIGGER_RATIO = 0.8;

const GB = (bytes) => Math.round((bytes / 1e9) * 1000) / 1000;

export class ExportError extends Error {
  constructor(status, exitCode, detail) {
    super(detail);
    this.status = status;
    this.exitCode = exitCode;
  }
}

/** Non-secret shape check for the SQL row the db-size query returns. */
export function parseDbSize(rows) {
  const bytes = rows?.[0]?.bytes;
  const n = typeof bytes === "string" ? Number(bytes) : bytes;
  if (typeof n !== "number" || !Number.isFinite(n) || n < 0) {
    throw new ExportError("unparseable", 6, "database/query: pg_database_size row missing/invalid");
  }
  return n;
}

/** Total requests from usage.api-requests-count ({ result: [{ count }] }). */
export function parseRequestCount(payload) {
  const count = payload?.result?.[0]?.count;
  const n = typeof count === "string" ? Number(count) : count;
  return typeof n === "number" && Number.isFinite(n) && n >= 0 ? n : null;
}

/** Measured $0 (or more) from billing/addons: selected addons priced fixed
 * monthly amounts; usage-priced addons are unknown while none are selected. */
export function billedFromAddons(payload) {
  const selected = payload?.selected_addons;
  if (!Array.isArray(selected)) {
    throw new ExportError("unparseable", 6, "billing/addons: selected_addons missing");
  }
  let usd = 0;
  for (const addon of selected) {
    const amount = addon?.price?.amount;
    if (typeof amount === "number" && Number.isFinite(amount)) usd += amount;
  }
  return Math.round(usd * 100) / 100;
}

export function metric(id, label, used, limit, unit, source, detail) {
  const m = { id, label, used, limit, unit, source };
  if (detail) m.detail = detail;
  if (used != null && limit != null && limit > 0 && used / limit >= PAID_TRIGGER_RATIO) {
    m.flag = "approaching-paid-trigger";
  }
  return m;
}

/** Build the three tracked metrics from whatever the API answered. Missing
 * reads stay null (never zero) and carry the reason in `detail`. */
export function buildMetrics({ dbBytes, bucketCount, requestTotal }) {
  const dbUsed = dbBytes == null ? null : GB(dbBytes);
  const storageUsed = bucketCount === 0 ? 0 : null; // no buckets = no stored bytes (a non-empty bucket inventory exposes no byte sizes)
  return [
    metric(
      "db-size",
      "Database size",
      dbUsed,
      FREE_LIMITS["db-size"],
      "gb",
      "management-api:/database/query pg_database_size",
      dbBytes == null ? "SQL read unavailable this tick" : `${dbBytes} bytes`,
    ),
    metric(
      "egress",
      "Egress (cycle)",
      null,
      FREE_LIMITS.egress,
      "gb",
      "none",
      `No Management API endpoint reports egress bytes (usage.egress / usage.unified-egress 404); ${
        requestTotal == null ? "request count also unavailable" : `${requestTotal} API requests observed (usage.api-requests-count)`
      }.`,
    ),
    metric(
      "storage",
      "File storage",
      storageUsed,
      FREE_LIMITS.storage,
      "gb",
      "management-api:/storage/buckets",
      bucketCount == null
        ? "storage/buckets unavailable this tick"
        : bucketCount === 0
          ? "0 buckets on the project — no stored objects"
          : `${bucketCount} buckets; per-bucket byte sizes are not exposed by this endpoint`,
    ),
  ];
}

/** Probe a path and return { status, json? , text? } without throwing on HTTP. */
async function probe(fetchImpl, token, path) {
  const res = await fetchImpl(`${API_BASE}${path}`, {
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "application/json",
    },
  });
  let body = null;
  try {
    body = await res.text();
  } catch {
    body = null;
  }
  let json = null;
  try {
    json = body ? JSON.parse(body) : null;
  } catch {
    json = null;
  }
  return { status: res.status, json, body };
}

async function api(fetchImpl, token, path, init) {
  const res = await fetchImpl(`${API_BASE}${path}`, {
    ...init,
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "application/json",
      ...(init?.headers ?? {}),
    },
  });
  const text = await res.text();
  return { status: res.status, text };
}

export async function run({ token, ref = DEFAULT_PROJECT_REF, fetchImpl = fetch } = {}) {
  if (!token) throw new ExportError("no-token", 3, "Bitwarden secret did not resolve");
  const call = (path, init) => api(fetchImpl, token, path, init);

  const project = await call(`/projects/${ref}`);
  if (project.status === 401 || project.status === 403) {
    throw new ExportError("scope-denied", 4, `GET /projects/${ref}: HTTP ${project.status}`);
  }
  if (project.status !== 200) {
    throw new ExportError("fetch-failed", 5, `GET /projects/${ref}: HTTP ${project.status}`);
  }
  let projectBody;
  try {
    projectBody = JSON.parse(project.text);
  } catch {
    throw new ExportError("unparseable", 6, "GET /projects/:ref returned non-JSON");
  }

  const scope = [
    { endpoint: "projects", path: "/projects", status: project.status },
    { endpoint: "project", path: `/projects/${ref}`, status: project.status },
  ];

  // Database size — the one real measure of the Free 500 MB wedge.
  let dbBytes = null;
  const size = await call(`/projects/${ref}/database/query`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ query: "select pg_database_size(current_database()) as bytes" }),
  });
  scope.push({ endpoint: "db-size SQL", path: `/projects/${ref}/database/query`, status: size.status });
  if (size.status === 200 || size.status === 201) {
    try {
      dbBytes = parseDbSize(JSON.parse(size.text));
    } catch (err) {
      scope.push({ endpoint: "db-size SQL", path: `/projects/${ref}/database/query`, status: size.status, note: err.message });
    }
  }

  // File storage — bucket inventory (a project with no buckets stores nothing).
  let bucketCount = null;
  const buckets = await call(`/projects/${ref}/storage/buckets`);
  scope.push({ endpoint: "storage/buckets", path: `/projects/${ref}/storage/buckets`, status: buckets.status });
  if (buckets.status === 200) {
    try {
      const parsed = JSON.parse(buckets.text);
      if (Array.isArray(parsed)) bucketCount = parsed.length;
    } catch {
      /* leave null — the metric says so */
    }
  }

  // Egress: there is no bytes endpoint. Record the gap precisely and keep the
  // request count as context, then the available usage endpoints for the record.
  const requests = await probe(fetchImpl, token, `/projects/${ref}/analytics/endpoints/usage.api-requests-count`);
  scope.push({
    endpoint: "usage.api-requests-count",
    path: `/projects/${ref}/analytics/endpoints/usage.api-requests-count`,
    status: requests.status,
    note: "request counts only — no egress bytes",
  });
  const requestTotal = requests.status === 200 ? parseRequestCount(requests.json) : null;
  for (const path of [
    `/projects/${ref}/analytics/endpoints/usage.egress`,
    `/projects/${ref}/analytics/endpoints/usage.unified-egress`,
    `/projects/${ref}/usage`,
    `/projects/${ref}/database/size`,
  ]) {
    const r = await probe(fetchImpl, token, path);
    scope.push({ endpoint: path.split("/").pop(), path, status: r.status, note: r.status === 404 ? "no such endpoint" : undefined });
  }

  // Billing: addons are the only readable $ state; the org-scope plan read is
  // Forbidden with this token, which is recorded, not guessed around.
  let billing = null;
  const addons = await call(`/projects/${ref}/billing/addons`);
  scope.push({ endpoint: "billing/addons", path: `/projects/${ref}/billing/addons`, status: addons.status });
  if (addons.status === 200) {
    try {
      const usd = billedFromAddons(JSON.parse(addons.text));
      billing = {
        plan: "free",
        billedUsd: usd,
        measured: true,
        source: "management-api:/billing/addons (no selected paid addons)",
      };
    } catch {
      billing = null;
    }
  }
  const slug = projectBody?.organization_slug;
  if (slug) {
    const org = await probe(fetchImpl, token, `/organizations/${slug}`);
    scope.push({
      endpoint: "organizations/:slug",
      path: `/organizations/${slug}`,
      status: org.status,
      note: org.status === 403 ? "scope gap: token is project-scoped; plan/billing org reads need an org token (Drew's account action)" : undefined,
    });
  }

  const metrics = buildMetrics({ dbBytes, bucketCount, requestTotal });
  const measured = metrics.some((m) => m.used != null);
  if (!measured) {
    throw new ExportError("unparseable", 6, "no metric could be measured this tick (scope: see probe)");
  }
  const gaps = scope.filter((s) => s.status >= 400);
  return {
    service: "supabase",
    source: SOURCE,
    status: gaps.length ? "partial" : "ok",
    measuredAt: new Date().toISOString(),
    project: projectBody?.name ?? "unknown",
    ref,
    metrics,
    billing,
    scope,
  };
}

export function failureDocument({ attemptedAt, status, detail }) {
  return {
    service: "supabase",
    source: SOURCE,
    status,
    measuredAt: null,
    attemptedAt,
    detail,
  };
}

export async function writeOutputs(outDir, doc) {
  await mkdir(outDir, { recursive: true });
  const tmp = join(outDir, `.latest.json.tmp-${process.pid}`);
  await writeFile(tmp, `${JSON.stringify(doc, null, 2)}\n`, "utf8");
  await rename(tmp, join(outDir, "latest.json"));
  await appendFile(join(outDir, "history.jsonl"), `${JSON.stringify(doc)}\n`, "utf8");
}

/** Resolve the management token by its Bitwarden NAME — value stays in memory. */
export async function resolveToken({ secretName = process.env.SUPABASE_TOKEN_SECRET_NAME ?? DEFAULT_SECRET_NAME, bwsBin = process.env.BWS_BIN ?? "bws" } = {}) {
  if (process.env.SUPABASE_MANAGEMENT_TOKEN) return process.env.SUPABASE_MANAGEMENT_TOKEN;
  const accessToken = process.env.BWS_ACCESS_TOKEN;
  const env = { ...process.env, ...(accessToken ? { BWS_ACCESS_TOKEN: accessToken } : {}) };
  const runBws = (args) =>
    new Promise((resolve, reject) => {
      execFile(bwsBin, args, { env, maxBuffer: 16 << 20, timeout: 30_000 }, (err, stdout) =>
        err ? reject(new Error("bws call failed")) : resolve(stdout),
      );
    });
  try {
    const list = JSON.parse(await runBws(["secret", "list"]));
    const entry = Array.isArray(list) ? list.find((s) => s.key === secretName) : null;
    if (!entry?.id) return null;
    const got = JSON.parse(await runBws(["secret", "get", entry.id]));
    return typeof got?.value === "string" && got.value.trim() ? got.value.trim() : null;
  } catch {
    return null; // never log why: the failure detail may carry the secret's neighbourhood
  }
}

async function main() {
  const args = process.argv.slice(2);
  const opt = (name, fallback) => {
    const i = args.indexOf(name);
    return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
  };
  const outDir = opt("--out-dir", join(homedir(), "atlas", "supabase-export"));
  const ref = opt("--project-ref", process.env.SUPABASE_PROJECT_REF ?? DEFAULT_PROJECT_REF);
  try {
    const token = await resolveToken();
    const doc = await run({ token, ref });
    await writeOutputs(outDir, doc);
    const flags = doc.metrics.filter((m) => m.flag).map((m) => m.id);
    console.log(
      JSON.stringify({ status: doc.status, measuredAt: doc.measuredAt, metrics: doc.metrics.map((m) => m.id), triggers: flags, outDir }),
    );
  } catch (e) {
    const err = e instanceof ExportError ? e : new ExportError("fetch-failed", 5, `unexpected: ${e.message ?? String(e)}`);
    const doc = failureDocument({ attemptedAt: new Date().toISOString(), status: err.status, detail: err.message });
    try {
      await writeOutputs(outDir, doc);
    } catch (writeErr) {
      console.error(`supabase-quota-export: could not write outputs: ${writeErr.message}`);
    }
    console.error(`supabase-quota-export: ${err.status}: ${err.message}`);
    process.exit(err.exitCode);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
