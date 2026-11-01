#!/usr/bin/env bun
// Quota gatherer (Stage 2, #1151): the PM stack's 15-minute quota
// tick. Deployed on the LAPTOP (systemd user timer, units in
// pm-stack/systemd/, installed by install.sh) beside the aggregator
// and the snapshot store. Two input families:
//
//   1. The OpenChamber quota module, imported in-process from the
//      laptop's OpenChamber checkout, fetching every configured
//      provider per credential. Credentials come from a DIRECT read
//      of OpenCode's own credential store (the `credential` table of
//      opencode.db, opened read-only) instead of the OpenCode server
//      API, so a stopped server cannot zero the tick (the CLI's
//      wholesale "Transport: fetch failed" failure mode, #1251).
//      The read is the OpenChamber module's own
//      openCodeLocalCredentialSource — the same shared implementation
//      the quota CLI falls back to (#2490 phase 3) — so its
//      projection (rows ordered (active, time_created, id), the last
//      row per integration selected, unreadable rows surfaced rather
//      than silently dropped) is the one every surface gets.
//      Environment-variable keys are not store rows and never appear
//      here — stored credentials win, which is the order OpenCode
//      itself uses.
//   3. The Turso org usage API (#1049): usage + subscription over
//      the verified route, token from Bitwarden by name.
//   2. File exports (the Meta Phase-1 convention, #1301): producers
//      the module cannot reach write latest.json documents on the
//      laptop (v1: Meta, ~/atlas/meta-quota-export/latest.json);
//      the gatherer ingests them in the same tick. A document older
//      than 30 minutes means the producer is unreachable — an error
//      entry, never zeros.
//
// Outputs:
//   - quota-current.json — the latest-state contract (model.ts
//     QuotaCurrentFile) in the deployed aggregator dir, beside
//     fleet-feed.json. (Aggregator pickup of this file is the
//     remaining hop; see the README's quota-gatherer section.)
//   - quota-gatherer-heartbeat.json — the tick's health record
//     (also embedded in quota-current.json).
//   - §4 snapshot records for windows whose value moved since the
//     last appended record, appended DIRECTLY to the aggregator's
//     snapshot store (SnapshotStore.append dedupes). Setting
//     USAGE_SNAPSHOT_OUTBOX_PATH switches the sink to the writers'
//     outbox JSONL instead (the sandbox collector's drain path).
//
// Fail-soft contract: if the module import fails or the credential
// store cannot be read, the tick writes a heartbeat carrying the
// error and exits non-zero WITHOUT touching quota-current.json. A
// single provider failing or exceeding its cap degrades only its own
// rows; the tick still succeeds. A provider whose run errors keeps
// its last-known ok entries from the previous tick's
// quota-current.json (#1268) — retained verbatim, so their original
// measuredAt makes the hub classify them stale instead of the fresh
// error rows replacing the last good data.
//
// Usage: bun run scripts/gather-quota.ts
// Env: OPENCHAMBER_LIB_DIR (default: first of ~/git/devtoday-
//      openchamber, ~/git/openchamber-fork containing the module),
//      OPENCODE_DB_PATH (~/.local/share/opencode/opencode.db),
//      QUOTA_CURRENT_PATH, QUOTA_HEARTBEAT_PATH,
//      QUOTA_GATHERER_STATE_PATH, QUOTA_SNAPSHOTS_DIR (./snapshots),
//      USAGE_SNAPSHOT_OUTBOX_PATH, META_QUOTA_EXPORT_PATH,
//      QUOTA_GATHER_TIMEOUT_MS (15000),
//      QUOTA_GATHER_CONCURRENCY (6), TURSO_API_TOKEN (else the
//      Bitwarden secret turso.platform-api-token-muse-cli, resolved
//      by name via ~/tools/bws-env conventions — BWS_BIN and
//      BWS_ACCESS_TOKEN_FILE override its plumbing),
//      TURSO_ORG_SLUG (dezren39), TURSO_API_BASE
//      (https://api.turso.tech).
import { existsSync } from "node:fs";
import { execFile } from "node:child_process";
import { access, appendFile, mkdir, readFile, rename, stat, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaGatherHeartbeat, SnapshotRecord } from "../src/model.ts";
import {
  GATHERER_SOURCE,
  entryFromResult,
  entryFromFileExport,
  entryFromTurso,
  lastKnownEntries,
  mapWithConcurrency,
  recordsFromEntries,
  type FileExportDoc,
  type QuotaProviderResult,
  type TursoSubscriptionDoc,
  type TursoUsageDoc,
} from "../src/quota-gather-map.ts";
import { SnapshotStore, isSnapshotRecord, snapshotKey } from "../src/snapshots.ts";

const PKG = join(dirname(fileURLToPath(import.meta.url)), "..");
const HOME = homedir();
const HIDDEN = join(HOME, "workspace/goals/keep-the-four-repos-healthy/hidden_files");

function resolveLibDir(): string {
  const candidates = [
    process.env.OPENCHAMBER_LIB_DIR,
    join(HOME, "git/devtoday-openchamber/packages/web/server/lib"),
    join(HOME, "git/openchamber-fork/packages/web/server/lib"),
  ].filter((d): d is string => typeof d === "string" && d !== "");
  for (const dir of candidates) {
    if (existsSync(join(dir, "quota/index.js"))) return dir;
  }
  return candidates[0];
}
const libDir = resolveLibDir();
const dbPath = process.env.OPENCODE_DB_PATH ?? join(HOME, ".local/share/opencode/opencode.db");
const currentPath = process.env.QUOTA_CURRENT_PATH ?? join(PKG, "quota-current.json");
const heartbeatPath = process.env.QUOTA_HEARTBEAT_PATH ?? join(PKG, "quota-gatherer-heartbeat.json");
const statePath = process.env.QUOTA_GATHERER_STATE_PATH ?? join(PKG, "quota-gatherer-state.json");
const snapshotsDir = process.env.QUOTA_SNAPSHOTS_DIR ?? join(PKG, "snapshots");
const outboxPath = process.env.USAGE_SNAPSHOT_OUTBOX_PATH ?? null;
const timeoutMs = Number(process.env.QUOTA_GATHER_TIMEOUT_MS ?? 15_000);
const gatherConcurrency = Number(process.env.QUOTA_GATHER_CONCURRENCY ?? 6);
const OBSERVED_BY = "quota-gatherer";

/** File-export producers ingested each tick (the #1301 convention). */
const FILE_EXPORTS = [
  {
    key: "meta",
    path: process.env.META_QUOTA_EXPORT_PATH ?? join(HOME, "atlas/meta-quota-export/latest.json"),
    staleMs: 30 * 60_000,
  },
];

interface QuotaModule {
  listConfiguredQuotaProviders: (opts?: { includeDisabled?: boolean }) => Promise<string[]>;
  fetchQuotaForProviderInstances: (providerId: string) => Promise<QuotaProviderResult[]>;
}
interface AuthModule {
  configureOpenCodeCredentials: (source: unknown) => void;
  // The shared direct credential-store read from lib/opencode/auth.js
  // (#2490 phase 3): same projection and failure semantics as the CLI
  // fallback — it throws when the store is missing or holds rows but
  // none are readable, and that throw reaches main()'s failTick via
  // listConfiguredQuotaProviders, so a bad store fails the tick loudly
  // instead of producing an empty "not configured" tick. It has no
  // listEnvironmentKeys: stored credentials are the whole story for
  // this reader, as before.
  openCodeLocalCredentialSource: (opts: { dbPath: string }) => unknown;
}

async function writeAtomic(path: string, text: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const tmp = `${path}.tmp-${process.pid}`;
  await writeFile(tmp, text, "utf8");
  await rename(tmp, path);
}

async function writeHeartbeat(hb: QuotaGatherHeartbeat): Promise<void> {
  await writeAtomic(heartbeatPath, JSON.stringify(hb, null, 2) + "\n");
}

async function failTick(tickAt: string, started: number, error: string): Promise<number> {
  const hb: QuotaGatherHeartbeat = {
    tickAt,
    ok: false,
    durationMs: Date.now() - started,
    perProvider: {},
    error,
  };
  await writeHeartbeat(hb).catch(() => {});
  console.error(`quota-gatherer: tick failed before providers ran: ${error}`);
  return 1;
}

async function withTimeout<T>(p: Promise<T>, ms: number, label: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} timed out after ${ms}ms`)), ms);
  });
  try {
    return await Promise.race([p, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

/** Bitwarden secret NAME for the Turso platform token (#1049).
 * The value is resolved at tick time and never logged or stored. */
const TURSO_SECRET_NAME = "turso.platform-api-token-muse-cli";

/**
 * Resolve one Bitwarden Secrets Manager secret by key, without ever
 * logging its value (the collect-fleet-feed.ts pattern). Plumbing:
 * bws at BWS_BIN, ~/tools/bin/bws, ~/.local/bin/bws, or PATH; the
 * machine-account access token at BWS_ACCESS_TOKEN_FILE, else
 * ~/.config/bws/access-token (laptop) or
 * ~/.config/bitwarden/access-token (sandbox). Null when the route
 * yields nothing.
 */
async function resolveBwsSecret(key: string): Promise<string | null> {
  const bwsCandidates = [process.env.BWS_BIN, join(HOME, "tools/bin/bws"), join(HOME, ".local/bin/bws"), "bws"].filter(
    (d): d is string => typeof d === "string" && d !== "",
  );
  let bwsBin: string | null = null;
  for (const cand of bwsCandidates) {
    if (!cand.includes("/")) {
      bwsBin = cand;
      break;
    }
    try {
      await access(cand);
      bwsBin = cand;
      break;
    } catch {
      /* next */
    }
  }
  if (!bwsBin) return null;
  const tokenFiles = [
    process.env.BWS_ACCESS_TOKEN_FILE,
    join(HOME, ".config/bws/access-token"),
    join(HOME, ".config/bitwarden/access-token"),
  ].filter((d): d is string => typeof d === "string" && d !== "");
  let accessToken: string | null = null;
  for (const f of tokenFiles) {
    try {
      const v = (await readFile(f, "utf8")).trim();
      if (v) {
        accessToken = v;
        break;
      }
    } catch {
      /* next */
    }
  }
  if (!accessToken) return null;
  const run = (args: string[]): Promise<string> =>
    new Promise((resolve, reject) => {
      execFile(
        bwsBin as string,
        args,
        {
          env: { ...process.env, BWS_ACCESS_TOKEN: accessToken as string },
          maxBuffer: 16 * 1024 * 1024,
          timeout: 30_000,
        },
        (err, stdout) => (err ? reject(err) : resolve(stdout)),
      );
    });
  try {
    const list = JSON.parse(await run(["secret", "list"])) as { id?: string; key?: string }[];
    const found = list.find((s) => s.key === key);
    if (!found?.id) return null;
    const got = JSON.parse(await run(["secret", "get", found.id])) as { value?: string };
    return got.value && got.value.trim() ? got.value.trim() : null;
  } catch {
    return null; // bws unreachable/unreadable — fail soft, no details logged
  }
}

/**
 * Turso org usage (#1049): usage + subscription reads over the
 * verified org route, mapped by entryFromTurso. Any failure (no
 * token, HTTP error, timeout) degrades to this run's error entry —
 * per-provider fail-soft like every other source; never zeros.
 */
async function runTurso(tickIso: string): Promise<ProviderRun> {
  const t0 = Date.now();
  const done = (entry: QuotaCurrentEntry): ProviderRun => ({
    id: "turso",
    state: entry.state === "ok" ? "ok" : "error",
    ms: Date.now() - t0,
    entries: [entry],
    ...(entry.error ? { error: entry.error } : {}),
  });
  const token = process.env.TURSO_API_TOKEN?.trim() || (await resolveBwsSecret(TURSO_SECRET_NAME));
  if (!token) {
    return done(
      entryFromTurso(null, null, {
        measuredAt: tickIso,
        error: `turso token unavailable (Bitwarden secret ${TURSO_SECRET_NAME})`,
      }),
    );
  }
  const base = process.env.TURSO_API_BASE ?? "https://api.turso.tech";
  const slug = process.env.TURSO_ORG_SLUG ?? "dezren39";
  try {
    const get = async (path: string): Promise<unknown> => {
      // Fetch AND body parse inside the timeout, with an abort
      // signal: a body that stalls after prompt headers must not
      // hang the tick (Codex review, PR #2366).
      return withTimeout(
        (async () => {
          const res = await fetch(`${base}/v1/organizations/${slug}${path}`, {
            headers: { Authorization: `Bearer ${token}` },
            signal: AbortSignal.timeout(timeoutMs),
          });
          if (!res.ok) throw new Error(`turso ${path} HTTP ${res.status}`);
          return res.json();
        })(),
        timeoutMs,
        `turso ${path}`,
      );
    };
    const usage = (await get("/usage")) as TursoUsageDoc;
    const subscription = (await get("/subscription")) as TursoSubscriptionDoc;
    return done(entryFromTurso(usage, subscription, { measuredAt: tickIso }));
  } catch (err) {
    return done(
      entryFromTurso(null, null, {
        measuredAt: tickIso,
        error: err instanceof Error ? err.message : String(err),
      }),
    );
  }
}

interface ProviderRun {
  id: string;
  state: "ok" | "error";
  ms: number;
  entries: QuotaCurrentEntry[];
  error?: string;
  /** #1268: entries are last-known retentions, not fresh reads. */
  retained?: boolean;
}

async function runProvider(quota: QuotaModule, id: string, tickIso: string): Promise<ProviderRun> {
  const t0 = Date.now();
  try {
    const results = await withTimeout(quota.fetchQuotaForProviderInstances(id), timeoutMs, id);
    const entries = results.map((r) => entryFromResult(r, tickIso));
    // A provider is ok when at least one credential row answered;
    // a failing row (e.g. Codex's token-sharing login, which OpenAI
    // refuses by design) keeps its error on its own entry.
    const anyOk = entries.some((e) => e.state === "ok");
    const firstBad = entries.find((e) => e.state !== "ok");
    return {
      id,
      state: anyOk ? "ok" : "error",
      ms: Date.now() - t0,
      entries,
      ...(!anyOk && firstBad?.error ? { error: firstBad.error } : {}),
    };
  } catch (err) {
    return {
      id,
      state: "error",
      ms: Date.now() - t0,
      entries: [],
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

/** Ingest one file-export document into a quota-current entry. */
async function runFileExport(fx: (typeof FILE_EXPORTS)[number]): Promise<ProviderRun> {
  const t0 = Date.now();
  let entry: QuotaCurrentEntry;
  try {
    const st = await stat(fx.path);
    let doc: FileExportDoc | null = null;
    try {
      doc = JSON.parse(await readFile(fx.path, "utf8")) as FileExportDoc;
    } catch {
      doc = null;
    }
    entry = entryFromFileExport(doc, {
      fileMtimeIso: st.mtime.toISOString(),
      stale: Date.now() - st.mtimeMs > fx.staleMs,
    });
  } catch {
    entry = entryFromFileExport(null, { fileMtimeIso: null, stale: false });
  }
  return {
    id: fx.key,
    state: entry.state === "ok" ? "ok" : "error",
    ms: Date.now() - t0,
    entries: [entry],
    ...(entry.error ? { error: entry.error } : {}),
  };
}

/** The previous tick's quota-current entries, for #1268 last-known
 *  retention. Any read/parse failure means no retention this tick. */
async function readPreviousEntries(): Promise<QuotaCurrentEntry[]> {
  try {
    const parsed: unknown = JSON.parse(await readFile(currentPath, "utf8"));
    if (typeof parsed === "object" && parsed !== null && Array.isArray((parsed as QuotaCurrentFile).providers)) {
      return (parsed as QuotaCurrentFile).providers;
    }
  } catch {
    // no previous tick yet, or an unreadable file — nothing to retain
  }
  return [];
}

async function readState(): Promise<Map<string, string>> {
  try {
    const parsed: unknown = JSON.parse(await readFile(statePath, "utf8"));
    if (typeof parsed === "object" && parsed !== null) {
      return new Map(Object.entries(parsed as Record<string, string>));
    }
  } catch {
    // no state yet — every current value is a first observation
  }
  return new Map();
}

async function main(): Promise<number> {
  const started = Date.now();
  const tickAt = new Date(started).toISOString();

  let quota: QuotaModule;
  try {
    const auth = (await import(join(libDir, "opencode/auth.js"))) as unknown as AuthModule;
    auth.configureOpenCodeCredentials(auth.openCodeLocalCredentialSource({ dbPath }));
    quota = (await import(join(libDir, "quota/index.js"))) as unknown as QuotaModule;
  } catch (err) {
    return failTick(tickAt, started, `quota module import failed: ${err instanceof Error ? err.message : String(err)}`);
  }

  let providerIds: string[];
  try {
    providerIds = await quota.listConfiguredQuotaProviders();
  } catch (err) {
    return failTick(
      tickAt,
      started,
      `credential store read failed: ${err instanceof Error ? err.message : String(err)}`,
    );
  }

  // #1268: bounded provider concurrency (default 6, matching the
  // OpenChamber CLI) instead of one Promise.all over the catalogue.
  const runs = await mapWithConcurrency(providerIds, gatherConcurrency, (id) => runProvider(quota, id, tickAt));
  for (const fx of FILE_EXPORTS) runs.push(await runFileExport(fx));
  runs.push(await runTurso(tickAt));

  // #1268: an errored run with no ok row of its own retains the
  // provider's last-known ok entries from the previous tick instead
  // of replacing them with fresh error rows (or with nothing, on a
  // whole-fetch timeout). The heartbeat below still reports the run
  // as an error, with its reason and the retained row count.
  const prevEntries = await readPreviousEntries();
  if (prevEntries.length) {
    for (const run of runs) {
      if (run.state !== "error" || run.entries.some((e) => e.state === "ok")) continue;
      const retained = lastKnownEntries(prevEntries, run.id);
      if (retained.length) {
        run.entries = retained;
        run.retained = true;
      }
    }
  }
  const entries = runs.flatMap((r) => r.entries);

  const perProvider: QuotaGatherHeartbeat["perProvider"] = {};
  for (const run of runs) {
    perProvider[run.id] = {
      state: run.state,
      ms: run.ms,
      rows: run.entries.length,
      ...(run.error ? { error: run.error } : {}),
    };
    console.log(
      `quota-gatherer: ${run.id} ${run.state} ${run.ms}ms rows=${run.entries.length}${run.retained ? " retained-last-known" : ""}${run.error ? ` error=${run.error.slice(0, 140)}` : ""}`,
    );
  }

  const heartbeat: QuotaGatherHeartbeat = {
    tickAt,
    ok: true,
    durationMs: Date.now() - started,
    perProvider,
  };
  const file: QuotaCurrentFile = {
    version: 1,
    generatedAt: tickAt,
    source: GATHERER_SOURCE,
    heartbeat,
    providers: entries,
  };
  await writeAtomic(currentPath, JSON.stringify(file, null, 2) + "\n");
  await writeHeartbeat(heartbeat);

  // Snapshot records: append only what moved, to the aggregator's
  // store directly (or the writers' outbox when explicitly routed).
  const lastAppended = await readState();
  const { records, appended } = recordsFromEntries(entries, lastAppended, OBSERVED_BY);
  const valid = records.filter(isSnapshotRecord);
  const seen = new Set<string>();
  const fresh = valid.filter((r: SnapshotRecord) => {
    const k = snapshotKey(r);
    if (seen.has(k)) return false;
    seen.add(k);
    return true;
  });
  if (fresh.length) {
    if (outboxPath) {
      await mkdir(dirname(outboxPath), { recursive: true });
      await appendFile(outboxPath, fresh.map((r) => JSON.stringify(r)).join("\n") + "\n", "utf8");
    } else {
      await new SnapshotStore(snapshotsDir).append(fresh);
    }
    for (const [k, v] of appended) lastAppended.set(k, v);
    await writeAtomic(statePath, JSON.stringify(Object.fromEntries(lastAppended), null, 2) + "\n").catch((err) =>
      console.error(`quota-gatherer: state write failed (${err}); next tick may re-append`),
    );
  }
  const okCount = runs.filter((r) => r.state === "ok").length;
  console.log(
    `quota-gatherer: tick ${tickAt} providers=${runs.length} ok=${okCount} records=+${fresh.length} durationMs=${heartbeat.durationMs} -> ${currentPath}`,
  );
  return 0;
}

process.exit(await main());
