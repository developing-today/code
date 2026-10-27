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
//      The projection replicates the server's: rows ordered
//      (active, time_created, id), the last row per integration is
//      the selected one. Environment-variable keys are not store
//      rows and never appear here — stored credentials win, which
//      is the order OpenCode itself uses.
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
// rows; the tick still succeeds.
//
// Usage: bun run scripts/gather-quota.ts
// Env: OPENCHAMBER_LIB_DIR (default: first of ~/git/devtoday-
//      openchamber, ~/git/openchamber-fork containing the module),
//      OPENCODE_DB_PATH (~/.local/share/opencode/opencode.db),
//      QUOTA_CURRENT_PATH, QUOTA_HEARTBEAT_PATH,
//      QUOTA_GATHERER_STATE_PATH, QUOTA_SNAPSHOTS_DIR (./snapshots),
//      USAGE_SNAPSHOT_OUTBOX_PATH, META_QUOTA_EXPORT_PATH,
//      QUOTA_GATHER_TIMEOUT_MS (15000).
import { Database } from "bun:sqlite";
import { existsSync } from "node:fs";
import { appendFile, mkdir, readFile, rename, stat, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaGatherHeartbeat, SnapshotRecord } from "../src/model.ts";
import {
  GATHERER_SOURCE,
  entryFromResult,
  entryFromFileExport,
  recordsFromEntries,
  type FileExportDoc,
  type QuotaProviderResult,
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
}

/**
 * A credential source ({ list }) backed by OpenCode's SQLite store,
 * read-only. Answers the entry shape the server's credential.list
 * returns: { id, integrationID, label, active, value }.
 */
function directCredentialSource(path: string) {
  return {
    list: async () => {
      const db = new Database(path, { readonly: true });
      try {
        const rows = db
          .query(
            "SELECT id, integration_id, label, value FROM credential ORDER BY active ASC, time_created ASC, id ASC",
          )
          .all() as { id: string; integration_id: string | null; label: string; value: string }[];
        const selected = new Map<string, string>();
        const parsed: { id: string; integrationID: string; label: string; value: unknown }[] = [];
        for (const row of rows) {
          if (!row.integration_id) continue;
          let value: unknown;
          try {
            value = JSON.parse(row.value);
          } catch {
            continue; // the server drops undecodable rows the same way
          }
          parsed.push({ id: row.id, integrationID: row.integration_id, label: row.label, value });
          selected.set(row.integration_id, row.id);
        }
        return parsed.map((p) => ({ ...p, active: selected.get(p.integrationID) === p.id }));
      } finally {
        db.close();
      }
    },
    // No managed-launch environment to project keys from; stored
    // credentials are the whole story for this reader.
    listEnvironmentKeys: async () => ({}),
  };
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

interface ProviderRun {
  id: string;
  state: "ok" | "error";
  ms: number;
  entries: QuotaCurrentEntry[];
  error?: string;
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
    auth.configureOpenCodeCredentials(directCredentialSource(dbPath));
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

  const runs = await Promise.all(providerIds.map((id) => runProvider(quota, id, tickAt)));
  for (const fx of FILE_EXPORTS) runs.push(await runFileExport(fx));
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
      `quota-gatherer: ${run.id} ${run.state} ${run.ms}ms rows=${run.entries.length}${run.error ? ` error=${run.error.slice(0, 140)}` : ""}`,
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
