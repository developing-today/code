#!/usr/bin/env bun
// Quota gatherer (Stage 2, #1151; single-source rewire #6127): the PM
// stack's 15-minute quota tick. Deployed on the LAPTOP (systemd user
// timer, units in pm-stack/systemd/, installed by install.sh) beside
// the aggregator and the snapshot store.
//
// THE SINGLE SOURCE (Drew's order, 2026-11-05, #6125): quota readings
// come from OpenCode-owned provider code. The configured listing and
// ordinary asks use `opencode quota --json`; OpenDesign prefers the
// server's provider-only quota endpoint when the deployed server and
// its credential support it. The CLI result remains the compatibility
// fallback during rollout. The gatherer never fetches a provider API
// itself. What the tick does around those OpenCode reads:
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
//   1b. The Meta session keeper (src/meta-renewal.ts, same rule,
//      Drew's Meta answer meta-auth-keeper-6120): the provider owns
//      the credential — the keeper captures the dev.meta.ai session
//      from the laptop Helium profile over CDP, validates it against
//      the portal API before storing it in the ONE provider-owned
//      location (meta-credential.json beside the state file), and
//      renews it on cadence and on auth failure. A renewal failure
//      records the alert in meta-renewal.json from the first bad
//      tick AND dispatches a Web Push. The meta-quota-export probe
//      document is read only for last-good continuity; the keeper
//      no longer arms or manages the exporter units. Fail-soft like
//      the OpenDesign keeper.
//   2. The subcommand, twice: the configured listing
//      (`opencode quota --json`) and the explicit ask
//      (`opencode quota --json meta command-code`) — providers whose
//      routing doors are disabled in the opencode config but whose
//      pools are live are always fetched when named, the same
//      discipline the gatherer's out-of-band asks kept before the
//      rewire. An explicit-ask result replaces a listing result for
//      the same provider + credential. When available, the OpenDesign
//      provider-only server answer replaces its CLI result; unavailable
//      auth, route or credential leaves the CLI result as fallback. The
//      subcommand exits non-zero
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
//   - Turso agent_quota_records rows (fleet-platform), one per pool
//     entry per tick, source "opencode-quota" — Drew's rule that all
//     quota info lands in the quota ledger on a regular basis in
//     addition to the dashboard. (The previous writer of pool rows,
//     source "pm-pool", died 2026-10-29; this sink replaces it. The
//     sink targeted Supabase agent.quota_records until the 2026-11-06
//     Wave 3 cutover, code#5690.) Fail-soft: a sink failure is logged
//     and never fails the tick — quota-current.json is already written
//     by then.
//   - pool-state.json in the claimant state dir (POOL_STATE_PATH) —
//     the babysitter's per-pool wall state (#1338), projected from
//     this tick's entries by src/pool-state.ts. Fail-soft on the
//     same terms as the Turso sink.
//   - quota-forecast.json beside quota-current.json — the Stage-3
//     forecast product (#12695, design D1/D2): the quota-analytics
//     pace projection per pool/window, projected from this tick's
//     entries + the snapshot store's history by
//     src/quota-forecast.ts, plus one Turso agent_quota_records
//     ledger row per pool/window (source "quota-forecast") beside
//     the reading rows. Fail-soft on the same terms: a forecast
//     failure never fails the tick and the previous forecast
//     stands and ages.
//   - quota-shadow-placement.json beside quota-current.json, plus
//     one appended line per tick in
//     quota-shadow-placement-history.jsonl beside it — the Stage-3
//     shadow placement record (#12697, design D3): the
//     forecast-informed ordering logged beside the live ordering
//     as a per-decision diff record, projected by
//     src/quota-shadow-placement.ts from this tick's entries + the
//     forecast product. SHADOW ONLY: no live path reads it; the
//     live ordering is computed from the measured entries alone.
//     Fail-soft on the same terms as the forecast.
//   - quota-anomaly-state.json + quota-anomaly-history.jsonl beside
//     quota-current.json — the Stage-3 anomaly detectors (#12699,
//     design D6): burn spike, adapter schema drift, vendor plan
//     change, run by src/quota-anomaly.ts over this tick's entries
//     and records + the store's history. The state file holds the
//     recorded baselines (parsed shapes, limits, window sets,
//     fired keys); each NEW finding is appended to the history
//     JSONL as an evidence-baseline diff record and posted to the
//     alerts ledger (AlertsStore.postIncident, source
//     "quota-anomaly"; no alerts store = the history record still
//     stands). Fail-soft on the same terms: a detector failure
//     never fails the tick, and a lost/corrupt state file simply
//     re-baselines (no alert storm).
//   - quota-forecast-alert-state.json beside quota-current.json —
//     the C4.5 forecast push alerts (#13261, design §1/§8): the
//     tick reads the C4 prediction log (quota-predictions.jsonl),
//     evaluates the newest record per subject via
//     src/quota-forecast-alerts.ts, and posts each NEW alertable
//     prediction to the alerts ledger (AlertsStore.postIncident,
//     source "quota-prediction", key = the record id). The state
//     holds the fired record ids so repeat ticks do not re-alert.
//     Fail-soft like the anomaly detectors.
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
//      QUOTA_CURRENT_PATH, QUOTA_HEARTBEAT_PATH, QUOTA_FORECAST_PATH
//      (default: quota-forecast.json beside QUOTA_CURRENT_PATH),
//      QUOTA_SHADOW_PLACEMENT_PATH / QUOTA_SHADOW_PLACEMENT_HISTORY_PATH
//      (defaults: quota-shadow-placement.json /
//      quota-shadow-placement-history.jsonl beside QUOTA_CURRENT_PATH),
//      QUOTA_ANOMALY_STATE_PATH / QUOTA_ANOMALY_HISTORY_PATH
//      (defaults beside QUOTA_CURRENT_PATH),
//      QUOTA_GATHERER_STATE_PATH, QUOTA_SNAPSHOTS_DIR (./snapshots),
//      USAGE_SNAPSHOT_OUTBOX_PATH,
//      QUOTA_GATHER_CONCURRENCY (retained for interface stability;
//      the subcommand owns fetch concurrency now),
//      OPENCHAMBER_LIB_DIR (ONLY the OpenDesign keeper's credential
//      store accessors resolve from here; no quota reading does),
//      OPENCHAMBER_LIB_INSTALL_CMD (override for the lib checkout's
//      dependency install step, shell-split like QUOTA_SUBCOMMAND;
//      default is bun install at the checkout root, #8759),
//      TURSO_QUOTA_SINK=0 disables the Turso sink (default on; token
//      resolved by name via the resolveBwsSecret plumbing:
//      turso.fleet-platform-rw-token).
import { execFile } from "node:child_process";
import { access, appendFile, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaGatherHeartbeat, SnapshotRecord } from "../src/model.ts";
import {
  GATHERER_SOURCE,
  CLAUDE_CROSS_TICK_STATE_KEY,
  CLAUDE_RETRY_BASE_MS,
  CLAUDE_RETRY_MAX_ATTEMPTS,
  claudeCooldownMs,
  claudeFailureKind,
  claudeRetryDelayMs,
  isClaudeRateLimitFailure,
  parseClaudeCrossTick,
  serializeClaudeCrossTick,
  type ClaudeCrossTickMemory,
  canonicalProviderId,
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
import { fleetPlatformUrl } from "../src/decision-events-sync.ts";
import { renewOpenDesignSession, type OpenDesignCredentialLike } from "../src/opendesign-renewal.ts";
import { fetchOpenCodeProviderQuota } from "../src/opencode-provider-quota.ts";
import { ensureLibDependencies } from "../src/lib-deps.ts";
import { loadOpenChamberLib, resolveLibDir as resolveOpenChamberLib } from "../src/openchamber-lib.ts";
import { readProviderAuthAlerts } from "../src/provider-auth-alerts.ts";
import { projectPoolState } from "../src/pool-state.ts";
import { buildForecastLedgerRows, buildQuotaForecast, type QuotaForecastFile } from "../src/quota-forecast.ts";
import { buildDowntimeLedgerRows, deriveDowntimeWindowRecords } from "../src/quota-downtime.ts";
import {
  buildResetDurationLedgerRows,
  deriveResetDurationRecords,
  resetDurationSubjectKey,
  resetObservationsFor,
} from "../src/quota-reset-duration.ts";
import { buildDayTypeShadowRecords, dayTypeShadowSubjectKey } from "../src/quota-daytype-adjustment.ts";
import { buildShadowPlacement } from "../src/quota-shadow-placement.ts";
import {
  anomalyIncidentInput,
  emptyAnomalyState,
  isAnomalyStateFile,
  runAnomalyTick,
  type AnomalyStateFile,
} from "../src/quota-anomaly.ts";
import {
  emptyForecastAlertState,
  evaluateForecastAlerts,
  forecastAlertIncidentInput,
  isForecastAlertState,
  parseForecastAlertLog,
} from "../src/quota-forecast-alerts.ts";
import { sectionPoolIdsFromConfig } from "../src/aa.ts";
import { renewMetaSession } from "../src/meta-renewal.ts";
import { classEnabled, getAlertsStore } from "../src/alerts.ts";
import { sendPush, type PushTarget } from "../src/push.ts";
import { requireHome } from "../src/home.ts";

const PKG = join(dirname(fileURLToPath(import.meta.url)), "..");
const HOME = requireHome("the gather-quota paths");
const HIDDEN = join(HOME, "workspace/goals/keep-the-four-repos-healthy/hidden_files");

/** Candidate checkouts for the OpenDesign keeper's managed credential
 * store (the OpenChamber lib). Store plumbing for ONE credential the
 * keeper maintains, never a quota reading path: all readings come from
 * the subcommand (see the header). Each candidate must be a clean main
 * checkout (src/openchamber-lib.ts), so a parked or stale checkout is
 * rejected rather than imported. */
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
// #1338: the claimant babysitter's pool-state.json, written into the
// claimant state dir (claimant.py STATE_DIR) on every tick so the
// walled verdict runs on live quota. Fail-soft like the Turso write:
// a failed pool-state write warns and never fails the tick.
const poolStatePath = process.env.POOL_STATE_PATH ?? join(HOME, ".local/state/opencode-claimant/pool-state.json");
// #12695: the Stage-3 forecast product, beside quota-current.json
// (design D1: same directory, same fail-soft posture).
const forecastPath = process.env.QUOTA_FORECAST_PATH ?? join(dirname(currentPath), "quota-forecast.json");
// #13257: the C4 prediction log (design §1), beside quota-current.json
// like the forecast product — append-only, one JSONL record per
// prediction, monthly rotation like the snapshot store.
const predictionsPath = process.env.QUOTA_PREDICTIONS_PATH ?? join(dirname(currentPath), "quota-predictions.jsonl");
// #12697: the Stage-3 shadow placement record (design D3), beside
// quota-current.json like the forecast: the latest diff record plus
// the append-only history the weekly review (S7) scores.
const shadowPlacementPath =
  process.env.QUOTA_SHADOW_PLACEMENT_PATH ?? join(dirname(currentPath), "quota-shadow-placement.json");
const shadowPlacementHistoryPath =
  process.env.QUOTA_SHADOW_PLACEMENT_HISTORY_PATH ?? join(dirname(currentPath), "quota-shadow-placement-history.jsonl");
// #13259: the C4.3 day-type shadow records (design §5 earn-in),
// beside quota-current.json like the other shadow products —
// append-only JSONL, deliberately NOT the C4 prediction log:
// shadow records are not the published forecast.
const daytypeShadowPath =
  process.env.QUOTA_DAYTYPE_SHADOW_PATH ?? join(dirname(currentPath), "quota-daytype-shadow.jsonl");
// #12699: the Stage-3 anomaly detectors' state + history, beside
// quota-current.json in the quota-watchdog shape (design D6).
const anomalyStatePath = process.env.QUOTA_ANOMALY_STATE_PATH ?? join(dirname(currentPath), "quota-anomaly-state.json");
const anomalyHistoryPath =
  process.env.QUOTA_ANOMALY_HISTORY_PATH ?? join(dirname(currentPath), "quota-anomaly-history.jsonl");
// #13261 (C4.5): the forecast-alert tick state, beside the anomaly
// state — the fired prediction record ids that suppress repeat
// ticks (record-keyed dedupe, design §8).
const forecastAlertStatePath =
  process.env.QUOTA_FORECAST_ALERT_STATE_PATH ?? join(dirname(currentPath), "quota-forecast-alert-state.json");
/** Strict millisecond parse for boot-time env (#2835 env-number contract):
 * digits only, 1..2^31-1, else fail naming the variable and the value
 * instead of coercing NaN/negative/huge values into a fetch deadline. */
function strictEnvMillis(name: string, raw: string | undefined, fallback: number): number {
  if (raw === undefined || raw === "") return fallback;
  if (/^[0-9]+$/.test(raw)) {
    const value = Number(raw);
    if (value >= 1 && value <= 2147483647) return value;
  }
  throw new Error(`${name}="${raw}" is not a valid millisecond count (expected an integer 1-2147483647)`);
}

const subcommandTimeoutMs = strictEnvMillis(
  "QUOTA_SUBCOMMAND_TIMEOUT_MS",
  process.env.QUOTA_SUBCOMMAND_TIMEOUT_MS,
  240_000,
);
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
const EXPLICIT_ASK_PROVIDERS = ["meta", "_commandcode", "deepseek", "mistral"];

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
async function runOpenDesignRenewal(tickAt: string): Promise<void> {
  if (process.env.OPENDESIGN_RENEWAL_DISABLED === "1") return;
  try {
    // Resolve first: a parked, dirty or non-main checkout is rejected
    // here, before anything from it is installed or imported. The keeper
    // then fails soft and the subcommand readings still run.
    const lib = await resolveOpenChamberLib(openChamberLibCandidates());
    // #8759: the lib checkout is a plain clone; if its declared
    // dependencies were never installed, every import below fails
    // with "Cannot find package ..." and Bun caches that failure
    // for the process, so install BEFORE the first import
    // (src/lib-deps.ts preflight), never as an import retry.
    const installCmd = process.env.OPENCHAMBER_LIB_INSTALL_CMD?.split(/\s+/).filter(Boolean) ?? [];
    const install =
      installCmd.length > 0
        ? async (root: string): Promise<void> => {
            await new Promise<void>((resolve, reject) => {
              execFile(
                installCmd[0],
                installCmd.slice(1),
                { cwd: root, timeout: 240_000, maxBuffer: 16 * 1024 * 1024 },
                (err) => {
                  if (err) reject(new Error("OPENCHAMBER_LIB_INSTALL_CMD failed in the OpenChamber lib checkout"));
                  else resolve();
                },
              );
            });
          }
        : undefined;
    await ensureLibDependencies({
      libDir: lib.dir,
      log: (line) => console.log("quota-gatherer: " + line),
      ...(install ? { install } : {}),
    });
    const { credentials, provider } = await loadOpenChamberLib(lib);
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
 * 2026-11-05 auto-renewal rule + his 2026-11-13 Meta answer,
 * meta-auth-keeper-6120). The keeper owns the Meta credential: CDP
 * capture from the laptop Helium profile, validate-before-store into
 * meta-credential.json, renewal on cadence and on auth failure. On a
 * new alert episode it also dispatches a push (notifyMetaAuthPush).
 * Fail-soft, same contract as the OpenDesign keeper.
 */

/**
 * The push half of Drew's Meta auth answer (#6120): a NEW meta-renewal
 * alert episode dispatches a Web Push (class "incident", the
 * breakthrough class) to every stored subscription, with the same
 * sends-ledger bookkeeping the digest delivery uses. No alerts store
 * (Turso store unavailable) = no delivery surface; the state-file alert
 * still stands and the outcome string records the skip.
 */
async function notifyMetaAuthPush(notice: { title: string; body: string }): Promise<string> {
  const store = await getAlertsStore();
  if (!store) return "no-store";
  const subs = await store.listSubscriptions();
  const payload = {
    class: "incident",
    objectRef: "provider-auth/meta",
    url: "/quota",
    title: notice.title,
    body: notice.body,
  };
  let sent = 0;
  for (const sub of subs) {
    if (!classEnabled(sub, "incident")) continue;
    const target: PushTarget = { endpoint: sub.endpoint, p256dh: sub.p256dh, auth: sub.auth };
    const outcome = await sendPush(target, payload);
    if (outcome.ok) {
      sent += 1;
      await store.logSend({ class: "incident", objectRef: payload.objectRef, outcome: "sent" });
      await store.noteSubscriptionAlive(sub.endpoint);
    } else if (outcome.gone) {
      await store.logSend({ class: "incident", objectRef: payload.objectRef, outcome: "gone" });
      await store.noteSubscriptionGone(sub.endpoint);
    } else {
      await store.logSend({ class: "incident", objectRef: payload.objectRef, outcome: "error" });
    }
  }
  return `sent:${sent}/${subs.length}`;
}
async function runMetaRenewal(tickAt: string): Promise<void> {
  if (process.env.META_RENEWAL_DISABLED === "1") return;
  try {
    const state = await renewMetaSession({
      statePath: process.env.META_RENEWAL_STATE_PATH ?? join(dirname(currentPath), "meta-renewal.json"),
      ...(process.env.META_EXPORTER_LATEST_PATH ? { latestPath: process.env.META_EXPORTER_LATEST_PATH } : {}),
      ...(process.env.META_CREDENTIAL_PATH ? { credentialPath: process.env.META_CREDENTIAL_PATH } : {}),
      notifyPush: notifyMetaAuthPush,
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
  state: "ok" | "error" | "lapsed";
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
      entries.find((e) => e.error)?.error ?? "no-data: provider reported ok but no window carried a percentage";
    for (const e of entries) {
      if (e.state === "ok") e.state = "error";
      if (!e.error) e.error = reason;
    }
    return { id, state: "error", ms: Date.now() - t0, entries, error: reason };
  }
  const firstFailure = entries.find((e) => e.state !== "ok" && e.state !== "lapsed");
  // A run whose only non-ok rows are planned lapses is terminal, not
  // failed: reporting it as "error" would trip #1268 retention and
  // replace the lapsed rows with last-known ok readings, hiding the
  // lapse the hub must show.
  const onlyLapsed = entries.length > 0 && entries.every((e) => e.state === "lapsed");
  return {
    id,
    state: anyOk ? "ok" : onlyLapsed ? "lapsed" : "error",
    ms: Date.now() - t0,
    entries,
    // A partially-failed run still surfaces its first failure: a
    // folded provider (_openai answers one row per OpenAI credential)
    // must not let one credential's ok row hide a sibling credential's
    // error from the heartbeat — 2026-11-10 the _openai run read "ok"
    // with no error while the main and self-contained credentials were
    // both failing token_expired at the provider.
    ...(firstFailure?.error
      ? { error: firstFailure.error }
      : onlyLapsed
        ? { error: entries.find((e) => e.state === "lapsed")?.error }
        : {}),
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

/** #8725: within-tick Claude retry base delay override (tests set
 * it near zero; production uses CLAUDE_RETRY_BASE_MS). */
function claudeRetryBaseMs(): number {
  const raw = Number(process.env.QUOTA_CLAUDE_RETRY_BASE_MS);
  return Number.isFinite(raw) && raw > 0 ? raw : CLAUDE_RETRY_BASE_MS;
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
 * Prefer the provider-only OpenCode server surface for OpenDesign when
 * the deployed server has it and holds the session-cookie credential.
 * Missing auth, an older server, or an unbound credential returns null
 * so the existing quota subcommand remains the rollout fallback.
 */
async function fetchServerOpenDesignQuota(): Promise<QuotaProviderResult | null> {
  const authPath = process.env.OPENCODE_AUTH_FILE ?? join(HOME, ".local/state/opencode/service-prod.json");
  let auth: { username?: unknown; password?: unknown };
  try {
    auth = JSON.parse(await readFile(authPath, "utf8")) as { username?: unknown; password?: unknown };
  } catch {
    return null;
  }
  if (typeof auth.password !== "string" || auth.password.length === 0) return null;
  return fetchOpenCodeProviderQuota({
    baseUrl: process.env.OPENCODE_URL ?? "http://127.0.0.1:13420",
    auth: {
      username: typeof auth.username === "string" && auth.username.length > 0 ? auth.username : "opencode",
      password: auth.password,
    },
    providerId: "opendesign",
  });
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

  const serverT0 = Date.now();
  const serverOpenDesign = await fetchServerOpenDesignQuota().catch(() => null);
  const serverMs = Date.now() - serverT0;

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
  // Once present, OpenCode's provider-only result outranks both CLI
  // reads: the server uses its bound session-cookie credential itself.
  // A missing route/auth/credential returns null above and leaves the
  // established subcommand result as the compatibility fallback.
  if (serverOpenDesign) {
    byProvider.set("opendesign", { results: [serverOpenDesign], ms: serverMs });
    console.log("quota-gatherer: opendesign quota answered by OpenCode provider API");
  }

  const runs: ProviderRun[] = [];
  for (const [id, group] of byProvider) {
    const t0 = Date.now() - group.ms;
    const entries = group.results.map((r) => entryFromResult(r, tickAt));
    // The folded OpenAI door still answers under the fork's pre-rename
    // spelling ("codex"); its entries already file under the canonical
    // door id via entryFromResult, but the run kept the raw spelling —
    // so the heartbeat's perProvider leg for the OpenAI pool was keyed
    // "codex" while every consumer (quota-current's legErrorFor,
    // matched on the entry's providerId) looks up "_openai". Rename
    // the run to the canonical id, the same mapping quota-gather-map
    // applies to the entries. Only this door's run id changes: the
    // heartbeat contract for the other doors is the fork spelling
    // (e.g. "meta"), and retention canonicalizes both sides already,
    // so nothing else moves.
    const runId = canonicalProviderId(id) === "_openai" ? "_openai" : id;
    let run = finalizeRun(runId, entries, t0);
    if (id === "command-code" || id === "_commandcode") run = keepCommandCodePlanWindows(run);
    runs.push(run);
  }
  // #8725: within-tick retry of the Claude leg. A 429 from the
  // Anthropic usage endpoint used to fail the leg for the whole
  // tick, leaving a stale row that downstream readers misread as a
  // credential failure. The leg now re-asks the subcommand for
  // _claude inside the same tick - bounded retries with jittered
  // backoff honoring Retry-After - and only a persistent 429 falls
  // through to the cross-tick retention in main, with the failure
  // kind recorded there. Other providers are untouched.
  const claudeIdx = runs.findIndex((r) => r.id === "_claude");
  if (claudeIdx >= 0) {
    const failureOf = (r: ProviderRun): string => r.error ?? r.entries.find((e) => e.error)?.error ?? "";
    let run = runs[claudeIdx];
    let attempt = 0;
    while (
      attempt < CLAUDE_RETRY_MAX_ATTEMPTS &&
      !run.entries.some((e) => e.state === "ok" && entryHasMeasuredWindow(e)) &&
      isClaudeRateLimitFailure(failureOf(run))
    ) {
      const delayMs = claudeRetryDelayMs(attempt, failureOf(run), Math.random, claudeRetryBaseMs());
      console.log(
        "quota-gatherer: _claude rate-limited - retry " +
          (attempt + 1) +
          "/" +
          CLAUDE_RETRY_MAX_ATTEMPTS +
          " in " +
          delayMs +
          "ms (#8725)",
      );
      await new Promise((resolve) => setTimeout(resolve, delayMs));
      const retryT0 = Date.now();
      try {
        const retryStdout = await execSubcommand(subcommandArgv(["_claude"]));
        const retryResults = parseSubcommandResults(retryStdout, "claude retry").filter(
          (r) => r.providerId === "_claude",
        );
        if (retryResults.length) {
          run = finalizeRun(
            "_claude",
            retryResults.map((r) => entryFromResult(r, tickAt)),
            retryT0,
          );
          runs[claudeIdx] = run;
        }
      } catch (err) {
        console.log(
          "quota-gatherer: _claude retry " +
            (attempt + 1) +
            " exec failed: " +
            (err instanceof Error ? err.message : String(err)),
        );
      }
      attempt++;
    }
    if (attempt > 0) {
      console.log(
        "quota-gatherer: _claude within-tick retries used=" +
          attempt +
          " recovered=" +
          run.entries.some((e) => e.state === "ok" && entryHasMeasuredWindow(e)) +
          " (#8725)",
      );
    }
  }
  return runs;
}

const TURSO_TOKEN_NAME = "turso.fleet-platform-rw-token";
const TURSO_URL = fleetPlatformUrl();

function sqlString(v: string): string {
  return `'${v.replace(/'/g, "''")}'`;
}

/**
 * The Turso sink: one agent_quota_records row per pool entry that
 * carries a reading (at least one window), in the shape the retired
 * "pm-pool" writer used (payload = {id, ts, plan, state, source,
 * windows[{name, percent, resetsAt}], provider}), with source =
 * GATHERER_SOURCE naming the single source. Runs AFTER
 * quota-current.json is written; fail-soft by contract (a sink
 * failure is logged, never fails the tick, never zeroes a row).
 * Until the 2026-11-06 Wave 3 cutover (code#5690) this wrote to
 * Supabase agent.quota_records (muse-dev2) via the Management API;
 * the table now lives in Turso fleet-platform and is written through
 * the libsql HTTP pipeline with INSERT OR IGNORE on the
 * (source, id, ts) PK.
 */
async function writeTursoRecords(entries: QuotaCurrentEntry[], tickAt: string): Promise<void> {
  if (process.env.TURSO_QUOTA_SINK === "0") return;
  try {
    const token = await resolveBwsSecret(TURSO_TOKEN_NAME);
    if (!token) {
      console.log("quota-gatherer: turso sink skipped \u2014 token unresolved by name");
      return;
    }
    const readable = entries.filter((e) => e.windows.length > 0);
    if (!readable.length) {
      console.log("quota-gatherer: turso sink \u2014 no entries with windows this tick");
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
      )}, ${sqlString(tickAt)})`;
    });
    const query =
      "insert or ignore into agent_quota_records (id, source, ts, plan, weekly_percent, resets, payload, inserted_at) values " +
      values.join(", ") +
      ";";
    const res = await fetch(`${TURSO_URL}/v2/pipeline`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({
        requests: [{ type: "execute", stmt: { sql: query } }, { type: "close" }],
      }),
    });
    if (!res.ok) {
      console.log(`quota-gatherer: turso sink failed-soft: HTTP ${res.status}`);
      return;
    }
    const out = (await res.json()) as { results?: { type: string; error?: { message?: string } }[] };
    const failed = (out.results ?? []).find((r) => r.type === "error");
    if (failed) {
      console.log(`quota-gatherer: turso sink failed-soft: ${failed.error?.message ?? "pipeline error"}`);
      return;
    }
    console.log(`quota-gatherer: turso sink wrote ${readable.length} quota_records rows (source=${GATHERER_SOURCE})`);
  } catch (err) {
    console.log(
      `quota-gatherer: turso sink failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
    );
  }
}

/**
 * The forecast ledger sink (#12695, design D1): one
 * agent_quota_records row per pool/window per tick, beside the
 * reading rows writeTursoRecords writes, source "quota-forecast"
 * with the forecast shape in the payload — so the Stage-3 accuracy
 * scoring joins forecasts to outcomes in one place. Same contract
 * as the reading sink: INSERT OR IGNORE on the (source, id, ts) PK,
 * fail-soft (logged, never fails the tick), and TURSO_QUOTA_SINK=0
 * disables it together with the reading sink — one ledger, one
 * switch.
 */
async function writeTursoForecastRecords(
  rows: ReturnType<typeof buildForecastLedgerRows>,
  tickAt: string,
): Promise<void> {
  if (process.env.TURSO_QUOTA_SINK === "0") return;
  if (!rows.length) return;
  try {
    const token = await resolveBwsSecret(TURSO_TOKEN_NAME);
    if (!token) {
      console.log("quota-gatherer: forecast turso sink skipped — token unresolved by name");
      return;
    }
    const values = rows.map(
      (r) =>
        `(${sqlString(r.id)}, ${sqlString(r.source)}, ${sqlString(r.ts)}, ${r.plan ? sqlString(r.plan) : "NULL"}, ${
          r.weeklyPercent ?? "NULL"
        }, ${r.resets ? sqlString(r.resets) : "NULL"}, ${sqlString(JSON.stringify(r.payload))}, ${sqlString(tickAt)})`,
    );
    const query =
      "insert or ignore into agent_quota_records (id, source, ts, plan, weekly_percent, resets, payload, inserted_at) values " +
      values.join(", ") +
      ";";
    const res = await fetch(`${TURSO_URL}/v2/pipeline`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({
        requests: [{ type: "execute", stmt: { sql: query } }, { type: "close" }],
      }),
    });
    if (!res.ok) {
      console.log(`quota-gatherer: forecast turso sink failed-soft: HTTP ${res.status}`);
      return;
    }
    const out = (await res.json()) as { results?: { type: string; error?: { message?: string } }[] };
    const failed = (out.results ?? []).find((r) => r.type === "error");
    if (failed) {
      console.log(`quota-gatherer: forecast turso sink failed-soft: ${failed.error?.message ?? "pipeline error"}`);
      return;
    }
    console.log(`quota-gatherer: forecast turso sink wrote ${rows.length} forecast rows (source=quota-forecast)`);
  } catch (err) {
    console.log(
      `quota-gatherer: forecast turso sink failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
    );
  }
}

/**
 * The Stage-3 anomaly detectors (#12699, design D6), run beside
 * the tick after the forecast publish. All logic is
 * src/quota-anomaly.ts (pure); this wrapper does the state/history
 * file IO and the alerts-ledger posts. Fail-soft throughout: any
 * failure is logged and the tick still succeeds; a missing or
 * corrupt state file re-baselines from empty (the detectors'
 * recorded shapes/limits rebuild from the next ticks' parses, and
 * re-baselining files no findings — a state loss never becomes an
 * alert storm). No alerts store (Turso unavailable) = the history
 * records still stand and the skip is logged, the meta-keeper
 * posture.
 */
async function runAnomalyDetectors(args: {
  entries: QuotaCurrentEntry[];
  fresh: SnapshotRecord[];
  tickAt: string;
}): Promise<void> {
  try {
    let state: AnomalyStateFile = emptyAnomalyState(new Date(args.tickAt));
    try {
      const raw: unknown = JSON.parse(await readFile(anomalyStatePath, "utf8"));
      if (isAnomalyStateFile(raw)) state = raw;
      else console.log("quota-gatherer: anomaly state file unreadable as a v1 state — re-baselining");
    } catch {
      // No state file yet (or unreadable): re-baseline from empty.
    }
    const stored = await new SnapshotStore(snapshotsDir).readAll().catch((): SnapshotRecord[] => []);
    const result = runAnomalyTick({
      state,
      entries: args.entries,
      freshRecords: args.fresh,
      history: [...stored, ...args.fresh],
      now: new Date(args.tickAt),
    });
    if (result.findings.length) {
      await mkdir(dirname(anomalyHistoryPath), { recursive: true });
      await appendFile(
        anomalyHistoryPath,
        result.findings.map((f) => JSON.stringify({ kind: "quota-anomaly-finding", version: 1, ...f })).join("\n") +
          "\n",
        "utf8",
      );
      const store = await getAlertsStore();
      if (!store) {
        console.log(
          `quota-gatherer: anomaly: ${result.findings.length} finding(s) recorded in history; no alerts store — no ledger post`,
        );
      } else {
        for (const f of result.findings) {
          try {
            await store.postIncident(anomalyIncidentInput(f), {
              detector: f.detector,
              serviceId: f.serviceId,
              window: f.window,
              baseline: f.baseline,
              observed: f.observed,
              detail: f.detail,
            });
          } catch (err) {
            console.log(
              `quota-gatherer: anomaly: ledger post failed-soft for ${f.key}: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
            );
          }
        }
      }
    }
    await writeAtomic(anomalyStatePath, JSON.stringify(result.state, null, 2) + "\n");
    const unevaluable = result.evaluations.filter((e) => e.state === "unevaluable").length;
    console.log(
      `quota-gatherer: anomaly detectors: ${result.findings.length} new finding(s), ${result.evaluations.length} evaluations (${unevaluable} unevaluable)`,
    );
  } catch (err) {
    console.error(
      `quota-gatherer: anomaly detectors failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
    );
  }
}

/**
 * The C4.5 forecast push alerts (#13261, component of #9572; C4
 * design §1/§8), run beside the tick after the forecast + downtime
 * publish. All selectivity/dedupe logic is
 * src/quota-forecast-alerts.ts (pure); this wrapper does the
 * prediction-log read, the workhorse join, the state-file IO and
 * the alerts-ledger posts. Fail-soft throughout like
 * runAnomalyDetectors: any failure is logged and the tick still
 * succeeds; a missing or corrupt state file re-baselines from
 * empty (the live predictions re-fire once — bounded to the
 * newest per subject by the log reader, never the whole log).
 * No alerts store (Turso unavailable) = the state still advances
 * and the skip is logged, the meta-keeper posture.
 */
async function runForecastAlerts(args: { entries: QuotaCurrentEntry[]; tickAt: string }): Promise<void> {
  try {
    let state = emptyForecastAlertState();
    try {
      const raw: unknown = JSON.parse(await readFile(forecastAlertStatePath, "utf8"));
      if (isForecastAlertState(raw)) state = raw;
      else console.log("quota-gatherer: forecast-alert state file unreadable as a v1 state — re-baselining");
    } catch {
      // No state file yet (or unreadable): re-baseline from empty.
    }
    // The C4 prediction log holds every tick's appended records;
    // the reader takes the newest per (kind, subject), so a fresh
    // or re-baselined state fires the live predictions, never the
    // log's superseded history.
    const logText = await readFile(predictionsPath, "utf8").catch(() => "");
    const records = parseForecastAlertLog(logText);
    // The workhorse pool set joins the deployed OpenCode config's
    // workhorse route section to this tick's pool ids (aa.ts
    // sectionPoolIdsFromConfig) — the alert module names no roster
    // of its own. An unreadable config yields no membership: pool
    // walls stay silent while downtime windows still fire.
    const poolIds = [...new Set(args.entries.flatMap((e) => [e.providerId, e.serviceId]))];
    const configText = await readFile(
      process.env.OPENCODE_CONFIG_PATH ?? join(HOME, ".local/state/opencode/opencode-runtime.jsonc"),
      "utf8",
    ).catch(() => "");
    const workhorsePools = new Set(sectionPoolIdsFromConfig(configText, poolIds).workhorse ?? []);
    const result = evaluateForecastAlerts({ records, state, workhorsePools, now: new Date(args.tickAt) });
    if (result.findings.length) {
      const store = await getAlertsStore();
      if (!store) {
        console.log(
          `quota-gatherer: forecast alerts: ${result.findings.length} finding(s); no alerts store — no ledger post`,
        );
      } else {
        for (const f of result.findings) {
          try {
            await store.postIncident(forecastAlertIncidentInput(f), { ...f.detail });
          } catch (err) {
            console.log(
              `quota-gatherer: forecast alerts: ledger post failed-soft for ${f.key}: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
            );
          }
        }
      }
    }
    await writeAtomic(forecastAlertStatePath, JSON.stringify(result.state, null, 2) + "\n");
    const unevaluable = result.evaluations.filter((e) => e.state === "unevaluable").length;
    console.log(
      `quota-gatherer: forecast alerts: ${result.findings.length} new finding(s), ${result.evaluations.length} evaluations (${unevaluable} unevaluable)`,
    );
  } catch (err) {
    console.error(
      `quota-gatherer: forecast alerts failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
    );
  }
}

export async function main(): Promise<number> {
  const started = Date.now();
  const tickAt = new Date(started).toISOString();

  // The OpenDesign keeper runs first: a session renewed this tick is
  // the one the subcommand's OpenDesign read uses.
  await runOpenDesignRenewal(tickAt);
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
  // whole-fetch timeout). Preserve confirmed lapse rows alongside
  // retained siblings: retention must never erase a terminal state
  // reported by this tick.
  // #5928: cross-tick memory for the _claude read. The fetcher's own
  // 429 cooldown/cache live in the long-lived OpenChamber process; this
  // oneshot carries its own across ticks (persisted in the state file,
  // see quota-gather-map). A success refreshes the last-good entry and
  // clears the cooldown; a fresh 429 sets the cooldown from
  // Retry-After (fallback 5 min, cap 60 min) and the tick emits the
  // last-good entry retained; a tick inside the cooldown likewise
  // emits the last-good entry retained instead of surfacing the fresh
  // failure. The bare error surfaces only when no read has ever
  // succeeded. The 401 dead-token mapping never engages the cooldown.
  // Claude only - every other provider flows into the #1268 retention
  // below unchanged. (Seam note: the shared listing subcommand still
  // performs its own fetch; what the cooldown governs is the
  // gatherer's consumption of the claude result, which is the
  // aggregator-side seam this fix owns.)
  const claudeState = await readState();
  let claudeMemory = parseClaudeCrossTick(claudeState.get(CLAUDE_CROSS_TICK_STATE_KEY));
  let claudeMemoryDirty = false;
  const claudeRun = runs.find((r) => r.id === "_claude");
  if (claudeRun) {
    const nowMs = Date.now();
    const okEntry = claudeRun.entries.find((e) => e.state === "ok" && entryHasMeasuredWindow(e));
    if (okEntry) {
      const next: ClaudeCrossTickMemory = {
        lastGoodEntry: JSON.parse(JSON.stringify(okEntry)) as QuotaCurrentEntry,
        cooldownUntil: 0,
      };
      if (serializeClaudeCrossTick(next) !== serializeClaudeCrossTick(claudeMemory)) {
        claudeMemory = next;
        claudeMemoryDirty = true;
      }
    } else {
      const failureText = claudeRun.error ?? claudeRun.entries.find((e) => e.error)?.error ?? "";
      if (isClaudeRateLimitFailure(failureText) && nowMs >= claudeMemory.cooldownUntil) {
        claudeMemory = { ...claudeMemory, cooldownUntil: nowMs + claudeCooldownMs(failureText) };
        claudeMemoryDirty = true;
      }
      if (claudeMemory.lastGoodEntry && nowMs < claudeMemory.cooldownUntil) {
        claudeRun.entries = markRetained([claudeMemory.lastGoodEntry], tickAt, claudeFailureKind(failureText));
        claudeRun.retained = true;
      }
    }
  }

  const prevEntries = await readPreviousEntries();
  if (prevEntries.length) {
    for (const run of runs) {
      // #8294: a Meta session lapse is temporary (remedy: re-auth on
      // the hub), unlike the terminal Go lapse — the pool keeps its
      // last good windows visible as stale rows while lapsed, so a
      // lapsed Meta run engages retention exactly like an error run,
      // and its own credential's readings are retained too.
      const metaLapsed = run.state === "lapsed" && run.id === "meta";
      if ((run.state !== "error" && !metaLapsed) || run.entries.some((e) => e.state === "ok")) continue;
      const retained = lastKnownEntries(prevEntries, run.id);
      if (retained.length) {
        const lapsed = run.entries.filter((e) => e.state === "lapsed");
        // Do not retain the previous ok reading for the same credential
        // that has just reported a lapse. Other credentials in the same
        // provider run may still retain their last-known readings.
        // (Meta is the exception, per the #8294 note above: its lapse
        // is the pool's own session, and retaining its last good
        // windows beside the lapsed row is the point.)
        const retainedSiblings = metaLapsed
          ? retained
          : retained.filter(
              (previous) =>
                !lapsed.some(
                  (current) =>
                    (current.credentialId && previous.credentialId === current.credentialId) ||
                    (current.credentialLabel &&
                      previous.credentialLabel?.trim().toLowerCase() === current.credentialLabel.trim().toLowerCase()),
                ),
            );
        run.entries = [...markRetained(retainedSiblings, tickAt), ...lapsed];
        run.retained = retainedSiblings.length > 0;
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
      ...(run.id === "_claude" && run.error ? { errorKind: claudeFailureKind(run.error) } : {}),
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
  // #6671: embed the keeper alert projection so the board/readout
  // surfaces a dying provider session within one gather cycle. The
  // keepers ran earlier this tick; reading their state files back
  // also covers a keeper that failed before writing (missing/stale
  // project as "keeper not reporting", never silence). Fail-soft:
  // the reader never throws.
  const providerAuth = await readProviderAuthAlerts({
    opendesign: process.env.OPENDESIGN_RENEWAL_STATE_PATH ?? join(dirname(currentPath), "opendesign-renewal.json"),
    meta: process.env.META_RENEWAL_STATE_PATH ?? join(dirname(currentPath), "meta-renewal.json"),
  });
  const file: QuotaCurrentFile = {
    version: 1,
    generatedAt: tickAt,
    source: GATHERER_SOURCE,
    heartbeat,
    providers: entries,
    providerAuth,
  };
  await writeAtomic(currentPath, JSON.stringify(file, null, 2) + "\n");
  await writeHeartbeat(heartbeat);

  // #1338: project this tick's readings into the babysitter's
  // pool-state.json (src/pool-state.ts). Fail-soft, after the file:
  // quota-current.json is already written, and a missing feed reads
  // as "unread" on the claimant side, never as a false all-clear.
  await writeAtomic(poolStatePath, JSON.stringify(projectPoolState(file), null, 2) + "\n").catch((err) =>
    console.error(`quota-gatherer: pool-state write failed (${err}); babysitter pool state stays unread this tick`),
  );

  // Snapshot records: append only what moved, to the aggregator's
  // store directly (or the writers' outbox when explicitly routed).
  const lastAppended = await readState();
  if (claudeMemoryDirty) lastAppended.set(CLAUDE_CROSS_TICK_STATE_KEY, serializeClaudeCrossTick(claudeMemory));
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

  if (!fresh.length && claudeMemoryDirty) {
    await writeAtomic(statePath, JSON.stringify(Object.fromEntries(lastAppended), null, 2) + "\n").catch((err) =>
      console.error(`quota-gatherer: state write failed (${err}); next tick may re-append`),
    );
  }

  // Turso: the same readings land in agent_quota_records on the
  // same tick (Drew's regular-basis rule). Fail-soft, after the file.
  await writeTursoRecords(entries, tickAt);

  // Stage-3 S1 (#12695, design D1/D2): publish the forecast product —
  // quota-forecast.json beside quota-current.json, plus the ledger
  // rows — projected by src/quota-forecast.ts from this tick's
  // entries and the snapshot store's history (this tick's appended
  // records merged in, so the current point is in the series even
  // when the append rode the outbox). Fail-soft like the pool-state
  // write: any failure logs and the tick still succeeds; the
  // previous forecast stands and ages, exactly as a failed tick
  // leaves the previous quota-current.json standing.
  let forecast: QuotaForecastFile | null = null;
  let history: SnapshotRecord[] = [];
  try {
    const stored = await new SnapshotStore(snapshotsDir).readAll().catch((): SnapshotRecord[] => []);
    history = [...stored, ...fresh];
    forecast = buildQuotaForecast(file, history, new Date(tickAt));
    await writeAtomic(forecastPath, JSON.stringify(forecast, null, 2) + "\n");
    const forecastRows = buildForecastLedgerRows(forecast, tickAt);
    await writeTursoForecastRecords(forecastRows, tickAt);
    const poolCount = Object.values(forecast.pools).reduce((n, ps) => n + ps.length, 0);
    console.log(
      `quota-gatherer: forecast wrote ${poolCount} pools / ${forecastRows.length} windows -> ${forecastPath}`,
    );
  } catch (err) {
    console.error(
      `quota-gatherer: forecast failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
    );
  }

  // C4.1 (#13257, component of #9572; C4 design §1/§7): the
  // complete-downtime prediction records — derived by
  // src/quota-downtime.ts from this tick's forecast product,
  // appended to the C4 prediction log beside the forecast
  // product, and mirrored into the Turso ledger under the C4
  // source "quota-prediction". Advisory only: no placement path
  // reads them (the C6 pages and forecast alerts consume them).
  // Fail-soft like the forecast write; a null forecast (the
  // block above failed) derives nothing — the previous records
  // stand and age with the previous forecast. The supersedes
  // link reads the log's tail for the last downtime record per
  // pool set; a missing or torn log just starts the chain over.
  if (forecast) {
    try {
      const previous = new Map<string, string>();
      const existing = await readFile(predictionsPath, "utf8").catch(() => "");
      for (const line of existing.split("\n").slice(-500)) {
        if (!line.trim()) continue;
        try {
          const parsed = JSON.parse(line) as { kind?: string; id?: string; subject?: { pools?: string[] } };
          if (parsed.kind === "downtime_window" && parsed.id && parsed.subject?.pools) {
            previous.set(parsed.subject.pools.join("+"), parsed.id);
          }
        } catch {
          // A torn last line is not fatal.
        }
      }
      const records = deriveDowntimeWindowRecords(forecast, previous);
      if (records.length) {
        await mkdir(dirname(predictionsPath), { recursive: true });
        await appendFile(predictionsPath, records.map((r) => JSON.stringify(r)).join("\n") + "\n", "utf8");
        await writeTursoForecastRecords(buildDowntimeLedgerRows(records, tickAt), tickAt);
      }
      console.log(
        `quota-gatherer: downtime predictions ${records.map((r) => r.state).join(",") || "none"} -> ${predictionsPath}`,
      );
    } catch (err) {
      console.error(
        `quota-gatherer: downtime failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
      );
    }
  }

  // C4.2 (#13258, C4 design §1/§7): the reset-duration prediction
  // records — derived by src/quota-reset-duration.ts for each walled
  // or near-wall pool window from the forecast product and the
  // snapshot history's resetsAt cadence, appended to the same C4
  // prediction log and mirrored into the Turso ledger under source
  // "quota-prediction". Advisory only; fail-soft like the downtime
  // block. Honesty states (no_basis, insufficient_history) publish
  // as records, never as invented ranges.
  if (forecast) {
    try {
      const previous = new Map<string, string>();
      const existing = await readFile(predictionsPath, "utf8").catch(() => "");
      for (const line of existing.split("\n").slice(-500)) {
        if (!line.trim()) continue;
        try {
          const parsed = JSON.parse(line) as { kind?: string; id?: string; subject?: { pool?: string; window?: string } };
          if (parsed.kind === "reset_duration" && parsed.id && parsed.subject?.pool && parsed.subject.window) {
            previous.set(resetDurationSubjectKey(parsed.subject.pool, parsed.subject.window), parsed.id);
          }
        } catch {
          // A torn last line is not fatal.
        }
      }
      const records = deriveResetDurationRecords(forecast, resetObservationsFor(forecast, history), previous);
      if (records.length) {
        await mkdir(dirname(predictionsPath), { recursive: true });
        await appendFile(predictionsPath, records.map((r) => JSON.stringify(r)).join("\n") + "\n", "utf8");
        await writeTursoForecastRecords(buildResetDurationLedgerRows(records, tickAt), tickAt);
      }
      console.log(
        `quota-gatherer: reset-duration predictions ${records.map((r) => r.state).join(",") || "none"} -> ${predictionsPath}`,
      );
    } catch (err) {
      console.error(
        `quota-gatherer: reset-duration failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
      );
    }
  }

  // Stage-3 S3 (#12697, design D3): the shadow placement record —
  // the forecast-informed ordering logged beside the live ordering
  // as a per-decision diff record (src/quota-shadow-placement.ts),
  // written as the latest-state file plus one appended line per
  // tick in the history JSONL the weekly review (S7) scores.
  // SHADOW ONLY: no live path reads the record, and it is computed
  // after every live output above is already written; a null
  // forecast (the block above failed) still yields a record, with
  // the shadow ordering equal to the live one. Fail-soft like the
  // forecast write: any failure logs and the tick still succeeds.
  try {
    const shadow = buildShadowPlacement(file, forecast, new Date(tickAt));
    await writeAtomic(shadowPlacementPath, JSON.stringify(shadow, null, 2) + "\n");
    await mkdir(dirname(shadowPlacementHistoryPath), { recursive: true });
    await appendFile(shadowPlacementHistoryPath, JSON.stringify(shadow) + "\n", "utf8");
    console.log(
      `quota-gatherer: shadow placement wrote ${shadow.summary.pools} pools, ${shadow.summary.demotedByForecast} forecast demotions (shadow only) -> ${shadowPlacementPath}`,
    );
  } catch (err) {
    console.error(
      `quota-gatherer: shadow placement failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
    );
  }

  // C4.3 (#13259 criterion 2, component of #9572; C4 design §5):
  // the day-type + spike-day SHADOW records — this tick's v1
  // forecast adjusted by the admitted day-type cells' measured
  // ratios, plus the explicit spike-day verdict — appended to
  // quota-daytype-shadow.jsonl beside the forecast product.
  // SHADOW ONLY (design §5 earn-in): no live path reads them and
  // the published forecast is never modified; they accumulate
  // for the criterion-3 S2-harness comparison. Fail-soft like
  // the shadow placement write; a null forecast (the forecast
  // block above failed) derives nothing. The supersedes chain
  // reads the file's tail for the last shadow record per
  // subject; a missing or torn file just starts the chain over.
  if (forecast) {
    try {
      const stored = await new SnapshotStore(snapshotsDir).readAll().catch((): SnapshotRecord[] => []);
      const previous = new Map<string, string>();
      const existing = await readFile(daytypeShadowPath, "utf8").catch(() => "");
      for (const line of existing.split("\n").slice(-2000)) {
        if (!line.trim()) continue;
        try {
          const parsed = JSON.parse(line) as {
            kind?: string;
            id?: string;
            subject?: { pool?: string; serviceId?: string; window?: string };
          };
          if (
            parsed.kind === "daytype_shadow" &&
            parsed.id &&
            parsed.subject?.pool &&
            parsed.subject?.serviceId &&
            parsed.subject?.window
          ) {
            previous.set(
              dayTypeShadowSubjectKey(parsed.subject.pool, parsed.subject.serviceId, parsed.subject.window),
              parsed.id,
            );
          }
        } catch {
          // A torn last line is not fatal.
        }
      }
      const records = buildDayTypeShadowRecords(forecast, [...stored, ...fresh], previous, new Date(tickAt));
      if (records.length) {
        await mkdir(dirname(daytypeShadowPath), { recursive: true });
        await appendFile(daytypeShadowPath, records.map((r) => JSON.stringify(r)).join("\n") + "\n", "utf8");
      }
      console.log(
        `quota-gatherer: daytype shadow ${records.filter((r) => r.state === "shadow").length} adjusted / ${records.filter((r) => r.state === "adjustment_unavailable").length} adjustment_unavailable -> ${daytypeShadowPath}`,
      );
    } catch (err) {
      console.error(
        `quota-gatherer: daytype shadow failed-soft: ${safeProviderReason(err instanceof Error ? err.message : String(err))}`,
      );
    }
  }

  // C4.5 (#13261, component of #9572; C4 design §1/§8): the
  // forecast push alerts over this tick's prediction log (the
  // downtime records appended above, plus the C4.1/C4.2 producers'
  // records as they publish). Fail-soft inside runForecastAlerts;
  // the tick's own result never depends on it.
  await runForecastAlerts({ entries, tickAt });

  // Stage-3 S5 (#12699, design D6): the anomaly detectors beside
  // the tick (burn spike / schema drift / plan change). Fail-soft
  // inside runAnomalyDetectors; the tick's own result never
  // depends on it.
  await runAnomalyDetectors({ entries, fresh, tickAt });

  const okCount = runs.filter((r) => r.state === "ok").length;
  console.log(
    `quota-gatherer: tick ${tickAt} providers=${runs.length} ok=${okCount} records=+${fresh.length} durationMs=${heartbeat.durationMs} -> ${currentPath}`,
  );
  return 0;
}

if (import.meta.main) process.exit(await main());
