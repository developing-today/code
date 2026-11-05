#!/usr/bin/env bun
// Quota gatherer (Stage 2, #1151; single-source rewire #6127): the PM
// stack's 15-minute quota tick. Deployed on the LAPTOP (systemd user
// timer, units in pm-stack/systemd/, installed by install.sh) beside
// the aggregator and the snapshot store.
//
// THE SINGLE SOURCE (Drew's order, 2026-11-05, #6125): every quota
// reading in this tick comes from ONE producer — the opencode fork's
// quota subcommand, `opencode quota --json`, whose per-provider quota
// functions live in the providers that own the credentials
// (developing-today-forks/opencode#1718). The gatherer invokes the
// subcommand; it never fetches a provider itself. What the tick does
// around that single read:
//
//   1. The OpenDesign session keeper (src/opendesign-renewal.ts,
//      Drew's 2026-11-05 provider browser-auth auto-renewal rule):
//      OpenDesign's quota function rides on a Better Auth session in
//      a managed credential store (the one credential in the system
//      that is a browser session rather than a token). Before the
//      subcommand runs each tick, the keeper reads the stored
//      session (authenticated reads slide its expiry), records the
//      horizon in opendesign-renewal.json beside quota-current.json,
//      re-captures the cookie from the standing laptop Helium over
//      CDP when the stored session fails or its horizon runs low
//      (validating the candidate before it is stored), and raises an
//      alert in that state file while days remain when renewal
//      cannot complete. The keeper maintains a CREDENTIAL; it is not
//      a quota source. Fail-soft: a keeper error never fails the tick.
//      (Its credential-store accessors live in the OpenChamber lib
//      the store was born in — store plumbing, not a reading path.)
//   1b. The Meta session keeper (src/meta-renewal.ts, same rule):
//      Meta's quota functions ride the laptop Helium profile's
//      dev.meta.ai session — no stored credential exists to
//      maintain — so the keeper watches the meta-quota-export probe
//      document each tick, re-arms the exporter units if they were
//      renamed away (the 2026-11-01 failure), takes a fresh probe,
//      and records the verdict in meta-renewal.json with an alert
//      from the first bad tick when the session is truly logged
//      out. Fail-soft like the OpenDesign keeper.
//   2. The subcommand, twice: the configured listing
//      (`opencode quota --json`) and the explicit ask
//      (`opencode quota --json meta command-code`) — providers whose
//      routing doors are disabled in the opencode config but whose
//      pools are live are always fetched when named, the same
//      discipline the gatherer's out-of-band asks kept before the
//      rewire. An explicit-ask result replaces a listing result for
//      the same provider + credential. The subcommand exits non-zero
//      when a requested provider produced no successful result; that
//      is a per-provider failure, not a tick failure — stdout still
//      carries the results, and only an unparseable/empty stdout
//      fails the tick (loudly; the previous quota-current.json
//      stands, #1268).
//
// Outputs:
//   - quota-current.json — the latest-state contract (model.ts
//     QuotaCurrentFile) in the deployed aggregator dir, beside
//     fleet-feed.json. Its `source` is "opencode-quota"
//     (GATHERER_SOURCE): readings are the subcommand's, and the file
//     says so.
//   - quota-gatherer-heartbeat.json — the tick's health record
//     (also embedded in quota-current.json).
//   - §4 snapshot records for windows whose value moved since the
//     last appended record, appended DIRECTLY to the aggregator's
//     snapshot store (SnapshotStore.append dedupes). Setting
//     USAGE_SNAPSHOT_OUTBOX_PATH switches the sink to the writers'
//     outbox JSONL instead (the sandbox collector's drain path).
//   - Supabase agent.quota_records rows (muse-dev2), one per pool
//     entry per tick, source "opencode-quota" — Drew's rule that all
//     quota info lands in Supabase on a regular basis in addition to
//     the dashboard. (The previous writer of pool rows, source
//     "pm-pool", died 2026-10-29; this sink replaces it.) Fail-soft:
//     a sink failure is logged and never fails the tick —
//     quota-current.json is already written by then.
//
// Fail-soft contract: if the subcommand cannot be run or its output
// cannot be parsed, the tick writes a heartbeat carrying the error
// and exits non-zero WITHOUT touching quota-current.json. A single
// provider failing degrades only its own rows; the tick still
// succeeds. A provider whose run errors keeps its last-known ok
// entries from the previous tick's quota-current.json (#1268) —
// retained verbatim, so their original measuredAt makes the hub
// classify them stale instead of the fresh error rows replacing the
// last good data.
//
// Usage: bun run scripts/gather-quota.ts
// Env: QUOTA_SUBCOMMAND (full command override, e.g. a fork-built
//      binary path + args, shell-split on spaces; default:
//      "opencode quota --json" / the explicit ask appends provider
//      ids), QUOTA_SUBCOMMAND_TIMEOUT_MS (240000 — the whole-exec
//      cap; the subcommand self-limits per provider),
//      QUOTA_CURRENT_PATH, QUOTA_HEARTBEAT_PATH,
//      QUOTA_GATHERER_STATE_PATH, QUOTA_SNAPSHOTS_DIR (./snapshots),
//      USAGE_SNAPSHOT_OUTBOX_PATH,
//      QUOTA_GATHER_CONCURRENCY (retained for interface stability;
//      the subcommand owns fetch concurrency now),
//      OPENCHAMBER_LIB_DIR (ONLY the OpenDesign keeper's credential
//      store accessors resolve from here; no quota reading does),
//      SUPABASE_QUOTA_SINK=0 disables the Supabase sink (default on;
//      token + project ref resolved by name via the resolveBwsSecret
//      plumbing: supabase.muse-dev2.access-token-muse-cli /
//      supabase.muse-dev2.project-ref).
import { execFile } from "node:child_process";
import { access, appendFile, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaGatherHeartbeat, SnapshotRecord } from "../src/model.ts";
import {
  GATHERER_SOURCE,
  entriesWithPoolSplits,
  entryFromResult,
  entryHasMeasuredWindow,
  lastKnownEntries,
  markRetained,
  recordsFromEntries,
  type QuotaProviderResult,
} from "../src/quota-gather-map.ts";
import { SnapshotStore, isSnapshotRecord, snapshotKey } from "../src/snapshots.ts";
import { safeProviderReason } from "../src/credential-lifecycle.ts";
import { renewOpenDesignSession, type OpenDesignCredentialLike } from "../src/opendesign-renewal.ts";
import { renewMetaSession } from "../src/meta-renewal.ts";
import { resolveLibDir as resolveOpenChamberLib, type ResolvedOpenChamberLib } from "../src/openchamber-lib.ts";

const PKG = join(dirname(fileURLToPath(import.meta.url)), "..");
const HOME = homedir();
const HIDDEN = join(HOME, "workspace/goals/keep-the-four-repos-healthy/hidden_files");

/** Candidate checkouts for the OpenDesign keeper's managed credential
 * store. All are validated before use; quota readings still come only
 * from the opencode subcommand. */
function openChamberLibCandidates(): string[] {
  return [
    process.env.OPENCHAMBER_LIB_DIR,
    join(HOME, "git/devtoday-openchamber/packages/web/server/lib"),
    join(HOME, "git/openchamber-fork/packages/web/server/lib"),
  ].filter((d): d is string => typeof d === "string" && d !== "");
}
const currentPath = process.env.QUOTA_CURRENT_PATH ?? join(PKG, "quota-current.json");
const heartbeatPath = process.env.QUOTA_HEARTBEAT_PATH ?? join(PKG, "quota-gatherer-heartbeat.json");
const statePath = process.env.QUOTA_GATHERER_STATE_PATH ?? join(PKG, "quota-gatherer-state.json");
const snapshotsDir = process.env.QUOTA_SNAPSHOTS_DIR ?? join(PKG, "snapshots");
const outboxPath = process.env.USAGE_SNAPSHOT_OUTBOX_PATH ?? null;
const subcommandTimeoutMs = Number(process.env.QUOTA_SUBCOMMAND_TIMEOUT_MS ?? 240_000);
const OBSERVED_BY = "quota-gatherer";

/** Providers always asked explicitly, by name, every tick: their
 * routing doors are disabled in the opencode config (definitions
 * kept, flipped off), so a listing can omit them even though their
 * credentials and plans are live pools. A named ask always fetches.
 * (Meta: route-only pool, #6079. Command Code: disabled door, live
 * plan — its read is the PLAN read; see keepCommandCodePlanWindows.
 * DeepSeek: named ask resolves it but the subcommand's listing
 * currently omits it — opencode#1729; remove it here when the
 * listing and the named ask agree on the configured set.
 * Mistral: its console budget read is CDP-backed and exceeds the
 * subcommand's 30s per-provider cap under the full listing's
 * concurrency (opencode#1729); the small explicit ask gives it
 * room, and its result replaces the listing's timed-out one.
 * Remove it here when the cap is raised.) */
const EXPLICIT_ASK_PROVIDERS = ["meta", "command-code", "deepseek", "mistral"];

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
 * The OpenDesign session keeper run. Runs before the subcommand so a
 * credential renewed this tick is the one the tick's OpenDesign read
 * uses. The validate step performs a quota fetch with the candidate
 * credential purely to prove the credential works before it is
 * stored — a credential check, not a quota reading (no value from it
 * reaches any output). Fail-soft: any error is logged and the tick
 * proceeds with the stored credential as-is.
 */
async function runOpenDesignRenewal(tickAt: string, lib: ResolvedOpenChamberLib): Promise<void> {
  if (process.env.OPENDESIGN_RENEWAL_DISABLED === "1") return;
  try {
    const { credentials, provider } = lib;
    const state = await renewOpenDesignSession({
      readCredential: () => credentials.readManagedCredential("opendesign"),
      writeCredential: (value) => {
        credentials.writeManagedCredential("opendesign", value);
      },
      validateCredential: async (credential) => {
        await provider.fetchOpenDesignQuota(credential);
      },
      cdpUrl: process.env.OPENDESIGN_CDP_URL ?? "http://127.0.0.1:19222",
      statePath: process.env.OPENDESIGN_RENEWAL_STATE_PATH ?? join(dirname(currentPath), "opendesign-renewal.json"),
      forceCapture: process.env.OPENDESIGN_RENEWAL_FORCE_CAPTURE === "1",
      now: () => new Date(tickAt),
      log: (line) => console.log(`quota-gatherer: ${line}`),
    });
    if (state.status === "attention") {
      console.log(`quota-gatherer: opendesign-renewal ATTENTION ${state.alert ?? ""}`);
    }
  } catch (err) {
    console.log(
      `quota-gatherer: opendesign-renewal failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
    );
  }
}

/**
 * The Meta session keeper run (src/meta-renewal.ts, Drew's
 * 2026-11-05 auto-renewal rule). Meta's quota functions ride the
 * laptop Helium profile's dev.meta.ai session — no stored credential
 * exists for a keeper to maintain, so this keeper watches the
 * meta-quota-export probe document instead: fresh + ok = session
 * valid; expired or silent = re-arm the exporter units if they were
 * renamed away, take a fresh probe, and if the session is truly
 * logged out raise an alert in meta-renewal.json from the first bad
 * tick (re-login is a human step — Meta emails a code). Fail-soft,
 * same contract as the OpenDesign keeper.
 */
async function runMetaRenewal(tickAt: string): Promise<void> {
  if (process.env.META_RENEWAL_DISABLED === "1") return;
  try {
    const state = await renewMetaSession({
      statePath: process.env.META_RENEWAL_STATE_PATH ?? join(dirname(currentPath), "meta-renewal.json"),
      ...(process.env.META_EXPORTER_LATEST_PATH ? { latestPath: process.env.META_EXPORTER_LATEST_PATH } : {}),
      now: () => new Date(tickAt),
      log: (line) => console.log(`quota-gatherer: ${line}`),
    });
    if (state.status === "attention") {
      console.log(`quota-gatherer: meta-renewal ATTENTION ${state.alert ?? ""}`);
    }
  } catch (err) {
    console.log(
      `quota-gatherer: meta-renewal failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
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

/**
 * Build a run from mapped entries, with the ok-but-null downgrade.
 * A run whose entries are all "ok" yet carry NO measured window
 * (every pctOfLimit null — Meta's active-but-no-subs_usage shape
 * since 2026-11-03) is not a successful read: previously it passed
 * as ok, so #1268 retention (which only fires on error runs) never
 * engaged and quota-current.json published fresh null rows over the
 * last good percentages. Downgrade it to an error run carrying the
 * mapper's no-data reason, flip its entries to error so retention
 * can replace them with last-known rows, and let the heartbeat say
 * why. The entries' failureClass / lastSuccessAt (when the provider
 * provides them) ride along in the reason.
 */
function finalizeRun(id: string, entries: QuotaCurrentEntry[], t0: number): ProviderRun {
  const anyOk = entries.some((e) => e.state === "ok");
  const anyMeasured = entries.some((e) => entryHasMeasuredWindow(e));
  if (anyOk && !anyMeasured) {
    const reason =
      entries.find((e) => e.error)?.error ??
      "no-data: provider reported ok but no window carried a percentage";
    for (const e of entries) {
      if (e.state === "ok") e.state = "error";
      if (!e.error) e.error = reason;
    }
    return { id, state: "error", ms: Date.now() - t0, entries, error: reason };
  }
  const firstBad = entries.find((e) => e.state !== "ok");
  return {
    id,
    state: anyOk ? "ok" : "error",
    ms: Date.now() - t0,
    entries,
    ...(!anyOk && firstBad?.error ? { error: firstBad.error } : {}),
  };
}

/** Command Code plan windows (Drew's rule, 2026-11-01: Command Code
 *  reads use PLAN, not BALANCE). The provider also returns balance
 *  windows (purchased_credits / free_credits: label-only, no
 *  percentage); those are balances, never quota, so they are stripped
 *  from the entry's windows — the raw payload stays on entry.usage —
 *  and no consumer or snapshot writer can mistake a balance for a
 *  plan reading or read the plan windows as "empty" because the
 *  first window is a balance. This is a mapping rule over the single
 *  source's output, not a second read. */
const COMMAND_CODE_PLAN_WINDOWS = new Set(["5h", "weekly", "monthly"]);
function keepCommandCodePlanWindows(run: ProviderRun): ProviderRun {
  for (const entry of run.entries) {
    entry.windows = entry.windows.filter((w) => COMMAND_CODE_PLAN_WINDOWS.has(w.name));
  }
  return run;
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

/** The subcommand invocation: QUOTA_SUBCOMMAND overrides the whole
 *  command (interim fork builds, tests); the default is the deployed
 *  opencode binary's quota subcommand. Explicit provider ids are
 *  appended as positional args. */
function subcommandArgv(extraArgs: string[]): string[] {
  const override = process.env.QUOTA_SUBCOMMAND?.trim();
  const base = override ? override.split(/\s+/) : ["opencode", "quota", "--json"];
  return [...base, ...extraArgs];
}

/** Run the subcommand once. A non-zero exit WITH stdout is a
 *  per-provider failure signal, not an exec failure — the results on
 *  stdout stand. An exec failure (spawn error, timeout, no stdout)
 *  rejects. */
function execSubcommand(argv: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(
      argv[0],
      argv.slice(1),
      {
        maxBuffer: 64 * 1024 * 1024,
        timeout: subcommandTimeoutMs,
        env: {
          ...process.env,
          // The standing runtime config is the CLI's fast,
          // deterministic config resolution. Interactive shells on
          // this host carry OPENCODE_CONFIG_PATH; the systemd unit
          // does not, and without it the CLI's default discovery
          // takes minutes (project scan) — the first production
          // ticks died on exactly that (opencode#1729). Respect an
          // explicitly set value; otherwise name the standing file.
          OPENCODE_CONFIG_PATH:
            process.env.OPENCODE_CONFIG_PATH ?? join(HOME, ".local/state/opencode/opencode-runtime.jsonc"),
        },
        // Neutral cwd (the state dir): quota is a machine-level read,
        // not a project read. From the aggregator's repo cwd the CLI
        // does project discovery over the code repo's worktrees —
        // minutes of CPU, and the first production tick died on it
        // (240s exec cap, truncated stdout, #6127).
        cwd: dirname(currentPath),
      },
      (err, stdout) => {
        if (err && !stdout) {
          reject(err);
          return;
        }
        resolve(stdout ?? "");
      },
    );
  });
}

/** Parse one subcommand run's stdout into provider results. The
 *  contract (opencode#1718): --json emits an array of result objects,
 *  one per credential instance; an object keyed by providerId is
 *  also accepted. Anything else is a tick-failing defect — a silent
 *  empty tick is exactly what the single-source rule exists to
 *  prevent. */
function parseSubcommandResults(stdout: string, label: string): QuotaProviderResult[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(stdout);
  } catch {
    throw new Error(`opencode quota (${label}): unparseable output (${stdout.length} bytes)`);
  }
  const arr = Array.isArray(parsed)
    ? parsed
    : typeof parsed === "object" && parsed !== null
      ? Object.values(parsed as Record<string, unknown>)
      : null;
  if (!arr) throw new Error(`opencode quota (${label}): output is not a result set`);
  const results = arr.filter(
    (r): r is QuotaProviderResult =>
      typeof r === "object" && r !== null && typeof (r as QuotaProviderResult).providerId === "string",
  );
  if (results.length === 0) throw new Error(`opencode quota (${label}): no provider results`);
  return results;
}

/**
 * THE tick's only quota read: the subcommand's configured listing,
 * plus the explicit ask for the route-disabled live pools
 * (EXPLICIT_ASK_PROVIDERS). Explicit results replace listing results
 * for the same provider + credential. Results are grouped into one
 * run per provider id, mapped through the unchanged quota-gather-map
 * contract.
 */
async function collectRuns(tickAt: string): Promise<ProviderRun[]> {
  const listingT0 = Date.now();
  const listingStdout = await execSubcommand(subcommandArgv([]));
  const listing = parseSubcommandResults(listingStdout, "listing");
  const listingMs = Date.now() - listingT0;

  const askT0 = Date.now();
  const askStdout = await execSubcommand(subcommandArgv(EXPLICIT_ASK_PROVIDERS));
  const asked = parseSubcommandResults(askStdout, "explicit ask");
  const askMs = Date.now() - askT0;

  // Per-provider group choice, the pre-rewire discipline generalized:
  // a listing group with an ok row wins (the ask is not consulted for
  // it); otherwise a fresher ask group with an ok row replaces it;
  // otherwise the ask group (fresher error) stands over the listing's.
  // The two invocations key credentials differently (the listing
  // enumerates stored credentials; a named ask may answer with a
  // credential-less result), so merging per-credential would publish
  // the same pool twice — once as each invocation saw it.
  const groupByProvider = (results: QuotaProviderResult[], ms: number) => {
    const groups = new Map<string, { results: QuotaProviderResult[]; ms: number }>();
    for (const result of results) {
      const group = groups.get(result.providerId) ?? { results: [], ms };
      group.results.push(result);
      groups.set(result.providerId, group);
    }
    return groups;
  };
  const listingGroups = groupByProvider(listing, listingMs);
  const askGroups = groupByProvider(asked, askMs);
  const byProvider = new Map<string, { results: QuotaProviderResult[]; ms: number }>();
  for (const [id, group] of listingGroups) byProvider.set(id, group);
  for (const [id, askGroup] of askGroups) {
    const listed = byProvider.get(id);
    if (!listed) {
      byProvider.set(id, askGroup);
      continue;
    }
    const listedOk = listed.results.some((r) => r.ok);
    const askOk = askGroup.results.some((r) => r.ok);
    if (!listedOk) byProvider.set(id, askGroup);
    else if (!askOk) {
      console.log(`quota-gatherer: ${id} ok via listing — explicit ask's error row dropped`);
    }
    // Both ok: the listing group stands — it is the canonical,
    // credential-keyed enumeration; the ask exists to rescue a
    // missing or errored listing group, not to override a good one.
  }

  const runs: ProviderRun[] = [];
  for (const [id, group] of byProvider) {
    const t0 = Date.now() - group.ms;
    const entries = group.results.map((r) => entryFromResult(r, tickAt));
    let run = finalizeRun(id, entries, t0);
    if (id === "command-code") run = keepCommandCodePlanWindows(run);
    runs.push(run);
  }
  return runs;
}

const SUPABASE_TOKEN_NAME = "supabase.muse-dev2.access-token-muse-cli";
const SUPABASE_REF_NAME = "supabase.muse-dev2.project-ref";

function sqlString(v: string): string {
  return `'${v.replace(/'/g, "''")}'`;
}

/**
 * The Supabase sink: one agent.quota_records row per pool entry that
 * carries a reading (at least one window), in the shape the retired
 * "pm-pool" writer used (payload = {id, ts, plan, state, source,
 * windows[{name, percent, resetsAt}], provider}), with source =
 * GATHERER_SOURCE naming the single source. Runs AFTER
 * quota-current.json is written; fail-soft by contract (a sink
 * failure is logged, never fails the tick, never zeroes a row).
 */
async function writeSupabaseRecords(entries: QuotaCurrentEntry[], tickAt: string): Promise<void> {
  if (process.env.SUPABASE_QUOTA_SINK === "0") return;
  try {
    const token = await resolveBwsSecret(SUPABASE_TOKEN_NAME);
    const ref = await resolveBwsSecret(SUPABASE_REF_NAME);
    if (!token || !ref) {
      console.log("quota-gatherer: supabase sink skipped — token/ref unresolved by name");
      return;
    }
    const readable = entries.filter((e) => e.windows.length > 0);
    if (!readable.length) {
      console.log("quota-gatherer: supabase sink — no entries with windows this tick");
      return;
    }
    const values = readable.map((e) => {
      const weekly = e.windows.find((w) => w.name === "weekly");
      const payload = {
        id: e.serviceId,
        ts: e.measuredAt,
        plan: e.planLabel ?? null,
        state: e.state,
        source: GATHERER_SOURCE,
        windows: e.windows.map((w) => ({ name: w.name, percent: w.pctOfLimit, resetsAt: w.resetsAt })),
        provider: e.providerName,
      };
      const id = `${e.serviceId}:${tickAt}`;
      return `(${sqlString(id)}, ${sqlString(GATHERER_SOURCE)}, ${sqlString(e.measuredAt)}, ${
        e.planLabel ? sqlString(e.planLabel) : "NULL"
      }, ${weekly?.pctOfLimit ?? "NULL"}, ${weekly?.resetsAt ? sqlString(weekly.resetsAt) : "NULL"}, ${sqlString(
        JSON.stringify(payload),
      )}::jsonb)`;
    });
    const query =
      "insert into agent.quota_records (id, source, ts, plan, weekly_percent, resets, payload) values " +
      values.join(", ") +
      " on conflict (source, id, ts) do nothing;";
    const res = await fetch(`https://api.supabase.com/v1/projects/${ref}/database/query`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ query }),
    });
    if (!res.ok) {
      console.log(`quota-gatherer: supabase sink failed-soft: HTTP ${res.status}`);
      return;
    }
    console.log(`quota-gatherer: supabase sink wrote ${readable.length} quota_records rows (source=${GATHERER_SOURCE})`);
  } catch (err) {
    console.log(
      `quota-gatherer: supabase sink failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
    );
  }
}

async function main(): Promise<number> {
  const started = Date.now();
  const tickAt = new Date(started).toISOString();

  let openChamberLib: ResolvedOpenChamberLib;
  try {
    openChamberLib = await resolveOpenChamberLib(openChamberLibCandidates());
  } catch (err) {
    return failTick(
      tickAt,
      started,
      `OpenChamber lib resolution failed: ${err instanceof Error ? err.message : String(err)}`,
    );
  }

  // The OpenDesign keeper runs first: a session renewed this tick is
  // the one the subcommand's OpenDesign read uses.
  await runOpenDesignRenewal(tickAt, openChamberLib);
  await runMetaRenewal(tickAt);

  // The single source: every reading this tick comes from the
  // opencode quota subcommand. A failure to run it, or output that
  // cannot be parsed, fails the tick loudly and the previous
  // quota-current.json stands.
  let runs: ProviderRun[];
  try {
    runs = await collectRuns(tickAt);
  } catch (err) {
    return failTick(
      tickAt,
      started,
      `opencode quota subcommand failed: ${err instanceof Error ? err.message : String(err)}`,
    );
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

  // Supabase: the same readings land in agent.quota_records on the
  // same tick (Drew's regular-basis rule). Fail-soft, after the file.
  await writeSupabaseRecords(entries, tickAt);

  const okCount = runs.filter((r) => r.state === "ok").length;
  console.log(
    `quota-gatherer: tick ${tickAt} providers=${runs.length} ok=${okCount} records=+${fresh.length} durationMs=${heartbeat.durationMs} -> ${currentPath}`,
  );
  return 0;
}

process.exit(await main());
