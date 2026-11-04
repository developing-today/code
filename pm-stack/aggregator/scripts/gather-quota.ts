#!/usr/bin/env bun
// Quota gatherer (Stage 2, #1151): the PM stack's 15-minute quota
// tick. Deployed on the LAPTOP (systemd user timer, units in
// pm-stack/systemd/, installed by install.sh) beside the aggregator
// and the snapshot store. Two input families:
//
//   1. The OpenChamber quota module, imported in-process from the
//      laptop's OpenChamber checkout, fetching every configured
//      provider per credential. Credentials come from ASKING THE
//      OPENCODE SERVER (Drew's ruling, 2026-11-03: the quota ask
//      terminates at the OpenCode server — never at a standalone
//      proxy program and never at a direct database read when the
//      server can answer): the module's own openCodeCredentialSource
//      against GET /api/credential on the standing fork server
//      (service-prod.json under ~/.local/state/opencode/ names its
//      URL and password; the password is read at tick time and never
//      logged) — the quota CLI's primary path (#2490). The DIRECT
//      read of OpenCode's credential store (the `credential` table
//      of opencode.db, opened read-only, the module's
//      openCodeLocalCredentialSource) remains ONLY as the
//      transport-failure fallback, the CLI's exact combined-source
//      shape: a stopped server switches the tick's reads to the
//      local store instead of zeroing the tick (#1251), and when
//      even that fails the tick fails loudly and the previous
//      quota-current.json stands, with #1268 retaining last-known
//      rows per provider. A server that ANSWERS and refuses is
//      authoritative — no fallback. The fallback's projection (rows
//      ordered (active, time_created, id), the last row per
//      integration selected, unreadable rows surfaced rather than
//      silently dropped) is the one every surface gets.
//      Environment-variable keys are not store rows and never appear
//      here — stored credentials win, which is the order OpenCode
//      itself uses.
//      One leg does NOT terminate at the server: OpenDesign. Its
//      provider reads a managed session credential (a Better Auth
//      session cookie + workspace id, in OpenChamber's own quota
//      credential store) because the billing usage API it answers
//      from accepts only that session — the credential the OpenCode
//      server holds for `opendesign` is the AMR inference API key,
//      which that API refuses (verified 403 as Bearer and as session
//      cookie, 2026-11-03), and the server exposes no quota/usage
//      surface at all (its route table has credential/session/etc.
//      groups, no quota group). The gap is filed as code#5060.
//   3. The Turso org usage API (#1049): usage + subscription over
//      the verified route, token from Bitwarden by name.
//   2. File exports (the Phase-1 convention, #1301): producers the
//      module cannot reach write latest.json documents on the laptop;
//      the gatherer ingests them in the same tick. A document older
//      than 30 minutes means the producer is unreachable — an error
//      entry, never zeros. The v1 producer (Meta's dev.meta.ai
//      console scrape) was retired 2026-11-03: Drew's ruling is that
//      quota is gathered by asking OpenCode — family 5 below is its
//      replacement, and the producer list is empty today.
//   4. The native ClinePass token adapter (#1159, src/clinepass-
//      adapter.ts on the #1153 framework): fills the ClinePass row
//      when the OpenChamber module did not already produce a fresh
//      one this tick, and only when CLINEPASS_CREDENTIAL_REF names
//      a credential reference on this host. An expired token is
//      session-expired / re-login with the last-known row retained
//      (#1268) — never a 0% row.
//   5. The Meta module ask (2026-11-03): the provider listing
//      (family 1) filters providers whose session door is disabled
//      in the opencode config, and `meta` is disabled as a direct
//      door — sessions reach the pool through model routes — so the
//      listing never names it. The gatherer asks the module for meta
//      explicitly (runMetaModule): the read runs through OpenCode's
//      own credential machinery, which is what gathering quota by
//      asking OpenCode means for a route-only pool.
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
//      OPENCODE_URL (default: the service-prod.json URL, else
//      http://127.0.0.1:13420), OPENCODE_AUTH_FILE
//      (~/.local/state/opencode/service-prod.json — the standing
//      server's URL + password, read at tick time, never logged),
//      OPENCODE_SERVER_PASSWORD (overrides the file's password),
//      QUOTA_CURRENT_PATH, QUOTA_HEARTBEAT_PATH,
//      QUOTA_GATHERER_STATE_PATH, QUOTA_SNAPSHOTS_DIR (./snapshots),
//      USAGE_SNAPSHOT_OUTBOX_PATH,
//      QUOTA_GATHER_TIMEOUT_MS (15000),
//      QUOTA_GATHER_CONCURRENCY (6), TURSO_API_TOKEN (else the
//      Bitwarden secret turso.platform-api-token-muse-cli, resolved
//      by name via ~/tools/bws-env conventions — BWS_BIN and
//      BWS_ACCESS_TOKEN_FILE override its plumbing),
//      TURSO_ORG_SLUG (dezren39), TURSO_API_BASE
//      (https://api.turso.tech),
//      CLINEPASS_CREDENTIAL_REF (bitwarden:<name> or file:<path> —
//      a CredentialRef for the native ClinePass adapter (#1159),
//      never a credential value; unset = not configured on this
//      host, live verification gated on #1155).
import { existsSync } from "node:fs";
import { execFile } from "node:child_process";
import { access, appendFile, mkdir, readFile, rename, stat, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaGatherHeartbeat, SnapshotRecord } from "../src/model.ts";
import {
  GATHERER_SOURCE,
  entriesWithPoolSplits,
  entryFromResult,
  entryFromFileExport,
  entryFromTurso,
  lastKnownEntries,
  mapWithConcurrency,
  markRetained,
  recordsFromEntries,
  type FileExportDoc,
  type QuotaProviderResult,
  type TursoSubscriptionDoc,
  type TursoUsageDoc,
} from "../src/quota-gather-map.ts";
import { SnapshotStore, isSnapshotRecord, snapshotKey } from "../src/snapshots.ts";
import {
  CLINEPASS_PROVIDER_ID,
  clinePassErrorEntry,
  credentialRefFromConfig,
  gatherClinePass,
} from "../src/clinepass-adapter.ts";
import type { CredentialRef } from "../src/token-adapter.ts";
import { safeProviderReason } from "../src/credential-lifecycle.ts";

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

/**
 * File-export producers ingested each tick (the #1301 convention).
 * The v1 producer — Meta, ~/atlas/meta-quota-export/latest.json, a
 * dev.meta.ai console scrape — was retired 2026-11-03: quota is
 * gathered by asking OpenCode (Drew's ruling), and the module's own
 * Meta provider replaced the scrape (see runMetaModule). The list
 * stays, empty, for the next producer the module cannot reach;
 * entryFromFileExport keeps the convention's mapping tested.
 */
const FILE_EXPORTS: { key: string; path: string; staleMs: number }[] = [];

interface QuotaModule {
  listConfiguredQuotaProviders: (opts?: { includeDisabled?: boolean }) => Promise<string[]>;
  fetchQuotaForProviderInstances: (providerId: string) => Promise<QuotaProviderResult[]>;
}
interface AuthModule {
  configureOpenCodeCredentials: (source: unknown) => void;
  openCodeCredentialSource: (opts: {
    buildOpenCodeUrl: (requestPath?: string) => string;
    getOpenCodeAuthHeaders: () => Record<string, string>;
  }) => CredentialSourceLike;
  // The shared direct credential-store read from lib/opencode/auth.js
  // (#2490 phase 3): same projection and failure semantics as the CLI
  // fallback — it throws when the store is missing or holds rows but
  // none are readable, and that throw reaches main()'s failTick via
  // listConfiguredQuotaProviders, so a bad store fails the tick loudly
  // instead of producing an empty "not configured" tick. It has no
  // listEnvironmentKeys: stored credentials are the whole story for
  // this reader, as before. Since 2026-11-03 it is wired only BEHIND
  // the server source above (transport-failure fallback), never as
  // the primary read.
  openCodeLocalCredentialSource: (opts: { dbPath: string }) => CredentialSourceLike;
}

interface CredentialSourceLike {
  list: () => Promise<unknown>;
  create?: (input: unknown) => Promise<unknown>;
  remove?: (credentialID: string) => Promise<unknown>;
  listEnvironmentKeys?: () => Promise<Record<string, string>>;
  usesLocalStore?: () => boolean;
  getLaunchEnvironment?: () => unknown;
}

interface ClaudeCredentialSource {
  loadClaudeCredential: () => Promise<{
    expiresAt?: number | null;
    source?: "keychain" | "credentials-file" | "opencode-auth" | "env";
  } | null>;
}

/** Read only Claude expiry metadata; credential values stay local and are discarded. */
async function claudeExpiryMetadata(): Promise<{
  expiresAt?: string;
  source?: "claude-code" | "opencode" | "environment";
} | null> {
  try {
    const auth = (await import(join(libDir, "quota/providers/claude/auth.js"))) as unknown as ClaudeCredentialSource;
    const credential = await auth.loadClaudeCredential();
    if (!credential) return null;
    const metadata: { expiresAt?: string; source?: "claude-code" | "opencode" | "environment" } = {};
    if (typeof credential.expiresAt === "number" && Number.isFinite(credential.expiresAt)) {
      const expiry = new Date(credential.expiresAt);
      if (Number.isFinite(expiry.getTime())) metadata.expiresAt = expiry.toISOString();
    }
    if (credential.source === "keychain" || credential.source === "credentials-file") metadata.source = "claude-code";
    else if (credential.source === "opencode-auth") metadata.source = "opencode";
    else if (credential.source === "env") metadata.source = "environment";
    return metadata;
  } catch {
    return null;
  }
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
    error: safeProviderReason(error),
  };
  await writeHeartbeat(hb).catch(() => {});
  console.error(`quota-gatherer: tick failed before providers ran: ${safeProviderReason(error)}`);
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
        error: safeProviderReason(err instanceof Error ? err.message : String(err)),
      }),
    );
  }
}

/**
 * Resolve a ClinePass CredentialRef to its value at tick time
 * (#1159): Bitwarden by name (the resolveBwsSecret plumbing), or a
 * file whose owner-only mode is enforced, not assumed. The value is
 * handed to the adapter's fetch and never logged or stored.
 */
async function resolveClinePassCredentialValue(ref: CredentialRef): Promise<string | null> {
  if (ref.kind === "bitwarden") return resolveBwsSecret(ref.name);
  try {
    const st = await stat(ref.path);
    if ((st.mode & 0o077) !== 0) return null;
    const v = (await readFile(ref.path, "utf8")).trim();
    return v || null;
  } catch {
    return null;
  }
}

/**
 * The native ClinePass adapter run (#1159). Null when no credential
 * reference is configured on this host — the adapter is registered
 * but dormant until #1155 names the provenance. A misconfigured
 * reference is an error row naming the defect, never a value.
 * Previous-value retention is the tick's #1268 pass below, so the
 * adapter is collected with no previous here.
 */
async function runClinePass(tickIso: string): Promise<ProviderRun | null> {
  const t0 = Date.now();
  let ref: CredentialRef | null;
  try {
    ref = credentialRefFromConfig(process.env.CLINEPASS_CREDENTIAL_REF);
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    const entry = clinePassErrorEntry(safeProviderReason(detail), tickIso);
    entry.credentialWatch = true;
    return {
      id: CLINEPASS_PROVIDER_ID,
      state: "error",
      ms: Date.now() - t0,
      entries: [entry],
      error: safeProviderReason(detail),
    };
  }
  if (!ref) return null;
  const outcome = await gatherClinePass({
    now: new Date(tickIso),
    previous: null,
    credentialRef: ref,
    resolveCredentialValue: resolveClinePassCredentialValue,
  });
  const entry =
    outcome.value ?? clinePassErrorEntry(outcome.detail ?? `cline-pass adapter: ${outcome.status}`, tickIso);
  entry.credentialWatch = true;
  return {
    id: CLINEPASS_PROVIDER_ID,
    state: outcome.status === "ok" ? "ok" : "error",
    ms: Date.now() - t0,
    entries: [entry],
    ...(outcome.status !== "ok" ? { error: outcome.detail ?? outcome.status } : {}),
  };
}

/**
 * The Meta module ask (2026-11-03, Drew's ask-OpenCode ruling): the
 * quota module's provider listing filters out providers whose
 * session door is disabled in the opencode config, and `meta` is
 * disabled as a direct door (sessions reach the pool through model
 * routes), so the listing never names it and the plain tick never
 * asked — the board's Meta row went dark even though OpenCode holds
 * the pool's credential and the module's Meta provider reads the
 * plan's own usage endpoint with it. The gatherer asks explicitly,
 * the same out-of-band registration pattern as the native ClinePass
 * adapter. No outer timeout wraps the ask: the provider self-times
 * out per request (15s), and its account-token fallback is a second
 * request — clipping the pair at the generic 15s cap could turn a
 * slow success into a false error row.
 */
async function runMetaModule(quota: QuotaModule, tickIso: string): Promise<ProviderRun> {
  const t0 = Date.now();
  try {
    const results = await quota.fetchQuotaForProviderInstances("meta");
    const entries = results.map((r) => entryFromResult(r, tickIso));
    const anyOk = entries.some((e) => e.state === "ok");
    const firstBad = entries.find((e) => e.state !== "ok");
    return {
      id: "meta",
      state: anyOk ? "ok" : "error",
      ms: Date.now() - t0,
      entries,
      ...(!anyOk && firstBad?.error ? { error: firstBad.error } : {}),
    };
  } catch (err) {
    return {
      id: "meta",
      state: "error",
      ms: Date.now() - t0,
      entries: [],
      error: safeProviderReason(err instanceof Error ? err.message : String(err)),
    };
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
      error: safeProviderReason(err instanceof Error ? err.message : String(err)),
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

/**
 * The standing OpenCode fork server's coordinates (Drew's ruling,
 * 2026-11-03: the quota ask terminates at the OpenCode server). The
 * service file is the fleet's standing record — the dispatcher's
 * OPENCODE_AUTH_FILE convention (pm-stack/dispatch/src/config.ts) —
 * naming the server's URL and its password. OPENCODE_URL /
 * OPENCODE_SERVER_PASSWORD override it. The password is held in this
 * closure only: never logged, never written, never placed in a URL.
 * A missing/unreadable file is not fatal: the default loopback URL is
 * used with no auth header, and the local-store fallback behind the
 * server source covers a server that cannot be reached at all.
 */
async function resolveOpenCodeServer(): Promise<{ baseUrl: string; authHeaders: () => Record<string, string> }> {
  let url = process.env.OPENCODE_URL?.trim() || "";
  let password = process.env.OPENCODE_SERVER_PASSWORD?.trim() || "";
  if (!url || !password) {
    try {
      const filePath = process.env.OPENCODE_AUTH_FILE ?? join(HOME, ".local", "state", "opencode", "service-prod.json");
      const svc = JSON.parse(await readFile(filePath, "utf8")) as { url?: unknown; password?: unknown };
      if (!url && typeof svc.url === "string" && svc.url.trim()) url = svc.url.trim();
      if (!password && typeof svc.password === "string") password = svc.password;
    } catch {
      // no service file — the defaults below stand
    }
  }
  const baseUrl = (url || "http://127.0.0.1:13420").replace(/\/+$/, "");
  return {
    baseUrl,
    authHeaders: () =>
      password ? { Authorization: `Basic ${Buffer.from(`opencode:${password}`).toString("base64")}` } : {},
  };
}

const TRANSPORT_ERROR_CODES = new Set([
  "ECONNREFUSED",
  "ECONNRESET",
  "ECONNABORTED",
  "ENOTFOUND",
  "EAI_AGAIN",
  "ETIMEDOUT",
  "EPIPE",
]);

/**
 * Whether a credential read failed because the OpenCode server could
 * not be reached at all, as opposed to the server answering and
 * refusing. Ported from the quota CLI's commands-quota.js
 * isTransportFailure (#2490): the OpenCode client marks its own
 * transport failures with `reason: 'Transport'` (an HTTP answer is a
 * different reason and stays authoritative); the walk down the cause
 * chain covers the fetch and socket errors underneath.
 */
function isTransportFailure(error: unknown): boolean {
  let current: unknown = error;
  for (let depth = 0; current && depth < 8; depth += 1) {
    const rec = current as { reason?: unknown; code?: unknown; message?: unknown; cause?: unknown };
    if (rec.reason === "Transport") return true;
    if (typeof rec.code === "string" && TRANSPORT_ERROR_CODES.has(rec.code)) return true;
    const message = typeof rec.message === "string" ? rec.message : "";
    if (message.startsWith("Transport:") || message === "fetch failed") return true;
    current = rec.cause;
  }
  return false;
}

/**
 * The server-first credential source wired into the quota module:
 * the OpenCode server answers (GET /api/credential via the module's
 * openCodeCredentialSource); the direct local-store read stands
 * behind it ONLY for transport failures — the quota CLI's
 * createFallbackCredentialSource shape (#2490), kept here so the
 * gatherer does not import CLI plumbing. The first transport failure
 * switches every later read this tick to the local store (a server
 * that is down does not come back mid-tick) and is logged once, so a
 * tick's provenance is never ambiguous. This is the #1251 resilience
 * contract: server-down degrades to the store read, and if the store
 * also fails the error propagates to failTick — the tick fails
 * loudly and last-known rows stand (#1268), never zeros.
 */
function createServerFirstCredentialSource(
  primary: CredentialSourceLike,
  fallback: CredentialSourceLike,
): CredentialSourceLike {
  let fellBack = false;
  const source: CredentialSourceLike = {
    usesLocalStore: () => primary.usesLocalStore?.() ?? false,
    getLaunchEnvironment: () => primary.getLaunchEnvironment?.() ?? null,
    list: async () => {
      if (fellBack) return fallback.list();
      try {
        return await primary.list();
      } catch (error) {
        if (!isTransportFailure(error)) throw error;
        fellBack = true;
        console.log(
          "quota-gatherer: OpenCode server unreachable — credentials from the local OpenCode store this tick (transport fallback)",
        );
        try {
          return await fallback.list();
        } catch (fallbackError) {
          const reason = error instanceof Error ? error.message : "OpenCode server unreachable";
          const storeReason =
            fallbackError instanceof Error ? fallbackError.message : "local credential store unreadable";
          throw new Error(`${reason}; the local credential store could not be read either (${storeReason})`);
        }
      }
    },
  };
  if (primary.create) source.create = (input) => primary.create!(input);
  if (primary.remove) source.remove = (credentialID) => primary.remove!(credentialID);
  if (primary.listEnvironmentKeys) source.listEnvironmentKeys = () => primary.listEnvironmentKeys!();
  return source;
}

async function main(): Promise<number> {
  const started = Date.now();
  const tickAt = new Date(started).toISOString();

  let quota: QuotaModule;
  try {
    const auth = (await import(join(libDir, "opencode/auth.js"))) as unknown as AuthModule;
    // Ask the OpenCode server first (GET /api/credential on the
    // standing fork server); the direct store read is only the
    // transport-failure fallback behind it. See
    // createServerFirstCredentialSource and the header (family 1).
    const server = await resolveOpenCodeServer();
    const serverSource = auth.openCodeCredentialSource({
      buildOpenCodeUrl: (requestPath?: string) => `${server.baseUrl}${requestPath ?? ""}`,
      getOpenCodeAuthHeaders: server.authHeaders,
    });
    auth.configureOpenCodeCredentials(
      createServerFirstCredentialSource(serverSource, auth.openCodeLocalCredentialSource({ dbPath })),
    );
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

  // #1159: register the native ClinePass adapter. Exactly one
  // cline-pass run survives: a fresh module row wins and the native
  // adapter is never even started (no duplicate credential resolve
  // or dashboard fetch that tick); otherwise the native run
  // replaces the module's errored run so #1268 retention below
  // applies to one run only.
  const clineIdx = runs.findIndex((r) => r.id === CLINEPASS_PROVIDER_ID);
  if (clineIdx !== -1 && runs[clineIdx].entries.some((e) => e.state === "ok")) {
    console.log("quota-gatherer: cline-pass ok via openchamber module - native adapter run skipped");
  } else {
    const clineRun = await runClinePass(tickAt);
    if (clineRun) {
      if (clineIdx === -1) runs.push(clineRun);
      else runs[clineIdx] = clineRun;
    }
  }

  // 2026-11-03: the Meta module ask (see runMetaModule). Exactly one
  // meta run survives, mirroring the ClinePass discipline above: a
  // fresh listed run (the door re-enabled someday) wins and the ask
  // is skipped; otherwise the ask replaces the listed run, or is
  // appended when the listing never named meta.
  const metaIdx = runs.findIndex((r) => r.id === "meta");
  if (metaIdx !== -1 && runs[metaIdx].entries.some((e) => e.state === "ok")) {
    console.log("quota-gatherer: meta ok via provider listing - explicit module ask skipped");
  } else {
    const metaRun = await runMetaModule(quota, tickAt);
    if (metaIdx === -1) runs.push(metaRun);
    else runs[metaIdx] = metaRun;
  }

  // The Claude auth source exposes an expiry timestamp. Persist only that
  // metadata and its source class, never the credential object or values.
  const claudeRun = runs.find((run) => run.id === "claude");
  if (claudeRun?.entries.length) {
    const metadata = await claudeExpiryMetadata();
    if (metadata) {
      for (const entry of claudeRun.entries) {
        if (metadata.expiresAt) entry.credentialExpiresAt = metadata.expiresAt;
        if (metadata.source) entry.credentialSource = metadata.source;
      }
    }
  }

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
        run.entries = markRetained(retained, tickAt);
        run.retained = true;
      }
    }
  }
  // #1331: file the split pool rows (Go per-workspace, Antigravity
  // per-pool) alongside the combined rows, so the overlay maps them
  // onto the four split hub pools and the snapshot writer continues
  // the pool-level series.
  const entries = entriesWithPoolSplits(runs.flatMap((r) => r.entries));
  for (const entry of entries) {
    if (entry.error) entry.error = safeProviderReason(entry.error);
  }

  const perProvider: QuotaGatherHeartbeat["perProvider"] = {};
  for (const run of runs) {
    perProvider[run.id] = {
      state: run.state,
      ms: run.ms,
      rows: run.entries.length,
      ...(run.error ? { error: safeProviderReason(run.error) } : {}),
    };
    console.log(
      `quota-gatherer: ${run.id} ${run.state} ${run.ms}ms rows=${run.entries.length}${run.retained ? " retained-last-known" : ""}${run.error ? ` error=${safeProviderReason(run.error).slice(0, 140)}` : ""}`,
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
