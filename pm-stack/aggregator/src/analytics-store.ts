// Data layer for the /quota "Analytics & projection" section
// (Drew's order, 2026-11-13). Everything here is a READ of a store
// that already exists — nothing is estimated into existence:
//
//   - Burn, models, efforts, legs: the route proxy's route_attempt
//     ledger in the OpenCode database (~/.local/share/opencode/
//     opencode.db) — one row per model call with provider, model,
//     variant (effort), token columns (input / output / reasoning /
//     cache read / cache write), recorded cost, and outcome.
//   - Sessions, sub-sessions, turns, tool calls, skill use, mcpx:
//     session_v2 + session_message in the same database (tool parts
//     carry the tool name; `skill` parts carry the skill name in
//     their input; mcpx tools are named mcpx_*).
//   - Fleet binary use per subcommand: INFERRED — counted by
//     scanning shell tool-call inputs for `fleet <subcommand>`.
//     There is no fleet invocation ledger; the UI labels this
//     "inferred" and the gap is filed as an issue.
//   - Pool percent history (for projections + week-over-week):
//     the aggregator's own usage-snapshot store, via UsageView
//     .history — dated quota reads, never a model.
//
// The OpenCode database is opened READ-ONLY. Any failure (missing
// file, locked, schema drift) degrades the section to an explicit
// unavailable state — the page never breaks over analytics.

import { Database } from "bun:sqlite";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { QuotaPool, QuotaView, SnapshotRecord, UsageView } from "./model.ts";
import {
  projectWindow,
  tokenRatios,
  unlockPicture,
  usFederalHoliday,
  wowDelta,
  type PctPoint,
  type TokenTotals,
  type UnlockPicture,
  type WindowProjection,
  type WowDelta,
} from "./quota-analytics.ts";

export const OPENCODE_DB_DEFAULT = join(homedir(), ".local", "share", "opencode", "opencode.db");

const HOUR_MS = 3_600_000;
const DAY_MS = 24 * HOUR_MS;

// ---------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------

export interface BurnBucket extends TokenTotals {
  calls: number;
  success: number;
  /** Recorded cost (route proxy's list-price equivalent), summed
   * over the rows that carry one; costRows says how many did. */
  costUsd: number;
  costRows: number;
}

export const emptyBurn = (): BurnBucket => ({
  calls: 0,
  success: 0,
  input: 0,
  output: 0,
  reasoning: 0,
  cacheRead: 0,
  cacheWrite: 0,
  costUsd: 0,
  costRows: 0,
});

export interface BurnGroup extends BurnBucket {
  key: string;
  label: string;
}

export interface PoolWindowAnalytics {
  name: string;
  percent: number;
  resetsAt: string | null;
  /** ms from now until the reset; null when no reset is recorded. */
  durationUntilResetMs: number | null;
  projection: WindowProjection;
  /** Duration-until-wall now vs the projection computed as of 7
   * days ago from the same series; null when either side has no
   * measured duration. */
  wowDurationUntilWall: WowDelta | null;
}

export interface PoolAnalytics {
  poolId: string;
  provider: string;
  plan: string;
  state: string;
  windows: PoolWindowAnalytics[];
  /** The window that gates the pool: walled first, then earliest
   * projected wall, then fullest. Null when the pool has none. */
  bindingWindow: string | null;
  unlock: UnlockPicture | null;
  /** Burn attributed to this pool's legs (see POOL_LEG_BASIS);
   * null when no leg maps to the pool. */
  burnToday: BurnBucket | null;
  burn7d: BurnBucket | null;
  burnWowTokens: WowDelta | null;
  /** How the burn attribution was made — shown in the UI. */
  legBasis: "credential" | "provider-id" | "shared-credential" | null;
  legLabel: string | null;
}

export interface ActivityCounts {
  /** Window the counts cover. */
  rangeLabel: string;
  sessions: number;
  rootSessions: number;
  subSessions: number;
  turns: number;
  toolCallsTotal: number;
  topTools: { name: string; count: number }[];
  skillTotal: number;
  topSkills: { name: string; count: number }[];
  /** Direct mcpx tool calls (mcpx_discover / mcpx_exec / mcpx_project). */
  mcpx: { name: string; count: number }[];
  /** The `execute` code-gateway tool (mcpx-style namespace calls go
   * through it; counted separately, not folded into mcpx). */
  executeGateway: number;
  /** INFERRED fleet binary use from shell tool-call inputs. */
  fleetInferred: { subcommand: string; count: number }[];
  fleetInferredTotal: number;
}

export interface QuotaAnalytics {
  generatedAt: string;
  now: number;
  dbAvailable: boolean;
  /** route_attempt coverage, ISO instants; null when unavailable. */
  attemptsFrom: string | null;
  attemptsTo: string | null;
  /** Percent of route_attempt rows in the last 7d that carry a
   * credential id (the pool-attribution coverage). */
  credentialCoveragePct: number | null;
  pools: PoolAnalytics[];
  totals: { lastHour: BurnBucket; today: BurnBucket; last7d: BurnBucket; prev7d: BurnBucket };
  totalsWowTokens: WowDelta;
  byModel7d: BurnGroup[];
  byEffort7d: BurnGroup[];
  byLeg7d: BurnGroup[];
  hourly7d: { hour: number; calls: number; tokens: number }[];
  dowHistory: { dow: number; calls: number; tokens: number }[];
  daily21d: { day: string; calls: number; tokens: number; holiday: string | null }[];
  /** Null while the first (slow) transcript scan is still running —
   * the section says so instead of blocking the page. */
  activity7d: ActivityCounts | null;
}

// ---------------------------------------------------------------------------
// OpenCode DB access (read-only, fail-soft)
// ---------------------------------------------------------------------------

function openDb(path: string): Database | null {
  try {
    if (!existsSync(path)) return null;
    return new Database(path, { readonly: true });
  } catch {
    return null;
  }
}

/** One shared read-only handle for the process: the background
 * activity scan runs after the triggering request returns, so the
 * handle must outlive any single build. Read-only + WAL means the
 * running OpenCode server is never disturbed. */
let sharedDb: { path: string; db: Database | null } | null = null;
function getDb(path: string): Database | null {
  if (!sharedDb || sharedDb.path !== path) sharedDb = { path, db: openDb(path) };
  return sharedDb.db;
}

interface DayLegRow {
  day: string;
  provider_id: string;
  credential_id: string | null;
  calls: number;
  success: number;
  input: number;
  output: number;
  reasoning: number;
  cache_read: number;
  cache_write: number;
  cost: number;
  cost_rows: number;
}

interface DayModelRow {
  day: string;
  model_id: string;
  variant: string | null;
  calls: number;
  success: number;
  input: number;
  output: number;
  reasoning: number;
  cache_read: number;
  cache_write: number;
  cost: number;
  cost_rows: number;
}

/** Every route_attempt-derived input the builder needs, computed in
 * one pass over the ledger. */
interface AttemptAggregates {
  legRows: DayLegRow[];
  modelRows: DayModelRow[];
  coverage: { f: number; t: number };
  hourRows: DayModelRow[];
  credentialCoveragePct: number | null;
  hourly7d: { hour: number; calls: number; tokens: number }[];
  dowHistory: { dow: number; calls: number; tokens: number }[];
  daily21d: { day: string; calls: number; tokens: number }[];
}

const DAY_SELECT = `date(time_started/1000, 'unixepoch', 'localtime')`;

function queryLegRowsRange(db: Database, from: number, to: number | null): DayLegRow[] {
  return db
    .query(
      `SELECT ${DAY_SELECT} AS day,
              provider_id, credential_id,
              COUNT(*) AS calls,
              SUM(outcome = 'success') AS success,
              COALESCE(SUM(tokens_input), 0) AS input,
              COALESCE(SUM(tokens_output), 0) AS output,
              COALESCE(SUM(tokens_reasoning), 0) AS reasoning,
              COALESCE(SUM(tokens_cache_read), 0) AS cache_read,
              COALESCE(SUM(tokens_cache_write), 0) AS cache_write,
              COALESCE(SUM(cost_usd), 0) AS cost,
              COUNT(cost_usd) AS cost_rows
         FROM route_attempt
        WHERE time_started >= ? AND (? IS NULL OR time_started < ?)
        GROUP BY day, provider_id, credential_id`,
    )
    .all(from, to, to) as DayLegRow[];
}

function queryLegRows(db: Database, now: number): DayLegRow[] {
  return queryLegRowsRange(db, now - 15 * DAY_MS, null);
}

function queryModelRowsRange(db: Database, from: number, to: number | null): DayModelRow[] {
  return db
    .query(
      `SELECT ${DAY_SELECT} AS day,
              model_id, variant,
              COUNT(*) AS calls,
              SUM(outcome = 'success') AS success,
              COALESCE(SUM(tokens_input), 0) AS input,
              COALESCE(SUM(tokens_output), 0) AS output,
              COALESCE(SUM(tokens_reasoning), 0) AS reasoning,
              COALESCE(SUM(tokens_cache_read), 0) AS cache_read,
              COALESCE(SUM(tokens_cache_write), 0) AS cache_write,
              COALESCE(SUM(cost_usd), 0) AS cost,
              COUNT(cost_usd) AS cost_rows
         FROM route_attempt
        WHERE time_started >= ? AND (? IS NULL OR time_started < ?)
        GROUP BY day, model_id, variant`,
    )
    .all(from, to, to) as DayModelRow[];
}

function queryModelRows(db: Database, now: number): DayModelRow[] {
  return queryModelRowsRange(db, now - 15 * DAY_MS, null);
}

function queryCoverage(db: Database): { f: number; t: number } {
  return db.query(`SELECT MIN(time_started) AS f, MAX(time_started) AS t FROM route_attempt`).get() as { f: number; t: number };
}

function queryHourRows(db: Database, now: number): DayModelRow[] {
  return db
    .query(
      `SELECT model_id, variant, COUNT(*) AS calls, SUM(outcome='success') AS success,
              COALESCE(SUM(tokens_input),0) AS input, COALESCE(SUM(tokens_output),0) AS output,
              COALESCE(SUM(tokens_reasoning),0) AS reasoning, COALESCE(SUM(tokens_cache_read),0) AS cache_read,
              COALESCE(SUM(tokens_cache_write),0) AS cache_write, COALESCE(SUM(cost_usd),0) AS cost, COUNT(cost_usd) AS cost_rows
         FROM route_attempt WHERE time_started >= ? GROUP BY model_id, variant`,
    )
    .all(now - HOUR_MS) as DayModelRow[];
}

function queryCredentialCoverageRange(db: Database, from: number, to: number | null): { n: number; c: number } {
  return db
    .query(`SELECT COUNT(*) AS n, COUNT(credential_id) AS c FROM route_attempt WHERE time_started >= ? AND (? IS NULL OR time_started < ?)`)
    .get(from, to, to) as { n: number; c: number };
}

function queryCredentialCoveragePct(db: Database, now: number): number | null {
  const credCov = queryCredentialCoverageRange(db, now - 7 * DAY_MS, null);
  return credCov.n > 0 ? (credCov.c / credCov.n) * 100 : null;
}

function queryHourly7dRange(db: Database, from: number, to: number | null): { hour: number; calls: number; tokens: number }[] {
  return db
    .query(
      `SELECT CAST(strftime('%H', time_started/1000, 'unixepoch', 'localtime') AS INTEGER) AS hour,
              COUNT(*) AS calls,
              COALESCE(SUM(tokens_input + tokens_output + tokens_cache_read + tokens_cache_write), 0) AS tokens
         FROM route_attempt WHERE time_started >= ? AND (? IS NULL OR time_started < ?) GROUP BY hour ORDER BY hour`,
    )
    .all(from, to, to) as { hour: number; calls: number; tokens: number }[];
}

function queryHourly7d(db: Database, now: number): { hour: number; calls: number; tokens: number }[] {
  return queryHourly7dRange(db, now - 7 * DAY_MS, null);
}

function queryDowHistoryRange(db: Database, from: number, to: number): { dow: number; calls: number; tokens: number }[] {
  return db
    .query(
      `SELECT CAST(strftime('%w', time_started/1000, 'unixepoch', 'localtime') AS INTEGER) AS dow,
              COUNT(*) AS calls,
              COALESCE(SUM(tokens_input + tokens_output + tokens_cache_read + tokens_cache_write), 0) AS tokens
         FROM route_attempt WHERE time_started >= ? AND time_started < ? GROUP BY dow ORDER BY dow`,
    )
    .all(from, to) as { dow: number; calls: number; tokens: number }[];
}

function queryDowHistory(db: Database): { dow: number; calls: number; tokens: number }[] {
  return db
    .query(
      `SELECT CAST(strftime('%w', time_started/1000, 'unixepoch', 'localtime') AS INTEGER) AS dow,
              COUNT(*) AS calls,
              COALESCE(SUM(tokens_input + tokens_output + tokens_cache_read + tokens_cache_write), 0) AS tokens
         FROM route_attempt GROUP BY dow ORDER BY dow`,
    )
    .all() as { dow: number; calls: number; tokens: number }[];
}

function queryDaily21dRange(db: Database, from: number, to: number | null): { day: string; calls: number; tokens: number }[] {
  return db
    .query(
      `SELECT ${DAY_SELECT} AS day,
              COUNT(*) AS calls,
              COALESCE(SUM(tokens_input + tokens_output + tokens_cache_read + tokens_cache_write), 0) AS tokens
         FROM route_attempt WHERE time_started >= ? AND (? IS NULL OR time_started < ?) GROUP BY day ORDER BY day`,
    )
    .all(from, to, to) as { day: string; calls: number; tokens: number }[];
}

function queryDaily21d(db: Database, now: number): { day: string; calls: number; tokens: number }[] {
  return queryDaily21dRange(db, now - 21 * DAY_MS, null);
}

/** [start, end) slice bounds covering [from, to): the first slice ends
 * at the next local midnight after `from`, the rest are whole local
 * days, the last is clipped at `to`. Day-aligned slices keep every
 * day-grouped query exact under plain concatenation. */
function daySliceBounds(from: number, to: number): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  let start = from;
  while (start < to) {
    const d = new Date(start);
    d.setHours(24, 0, 0, 0);
    const end = Math.min(d.getTime(), to);
    out.push([start, end]);
    start = end;
  }
  return out;
}

/** [start, end) bounds covering [from, to) in fixed-width chunks. */
function chunkBounds(from: number, to: number, widthMs: number): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  for (let start = from; start < to; start += widthMs) out.push([start, Math.min(start + widthMs, to)]);
  return out;
}

function computeAttemptAggregates(db: Database, now: number): AttemptAggregates {
  return {
    legRows: queryLegRows(db, now),
    modelRows: queryModelRows(db, now),
    coverage: queryCoverage(db),
    hourRows: queryHourRows(db, now),
    credentialCoveragePct: queryCredentialCoveragePct(db, now),
    hourly7d: queryHourly7d(db, now),
    dowHistory: queryDowHistory(db),
    daily21d: queryDaily21d(db, now),
  };
}

// #5082: ALL route_attempt aggregates ride ONE stale-while-revalidate
// cache. The scans cost seconds each over the 21 GB ledger (measured
// 2026-11-13: ~5.4 s of uncached scans per build, ~10 s cold) and the
// builder runs synchronously on the server's only event loop, so
// per-request scans made /quota take 5-19 s and stalled every other
// page queued behind them. The aggregates are slow-moving (hour/day
// buckets over 7-21 day windows), so a minutes-old copy is honest;
// refreshes run chunked in the background, one query per macrotask,
// so a refresh never monopolizes the loop either. The first build in
// a process computes synchronously — server.ts warms it at boot, off
// the request path.
//
// #5082 follow-up (same day): one query per macrotask was not enough —
// the widest single scans (leg/model rows over 15d, the unbounded
// day-of-week history) still blocked the loop for ~2.6 s each. The
// refresh now runs every windowed scan in day slices (2-day chunks
// for the day-of-week history, whose span is set by the coverage
// read): the widest measured slice is ~0.3 s, and slices merge
// exactly — day-grouped rows are disjoint across day-aligned slices,
// hour/day-of-week buckets merge by summation.
const AGGREGATES_TTL_MS = 300_000;
let aggregatesCache: { path: string; at: number; data: AttemptAggregates } | null = null;
let aggregatesRefreshing = false;

function refreshAttemptAggregates(db: Database, path: string, onSettled?: () => void): void {
  if (aggregatesRefreshing) return;
  aggregatesRefreshing = true;
  const now = Date.now();
  const until = now + 1;
  const data: Partial<AttemptAggregates> = { legRows: [], modelRows: [], daily21d: [] };
  const hourlyByHour = new Map<number, { hour: number; calls: number; tokens: number }>();
  const dowByDow = new Map<number, { dow: number; calls: number; tokens: number }>();
  const cred = { n: 0, c: 0 };
  const steps: Array<() => void> = [];
  for (const [from, to] of daySliceBounds(now - 15 * DAY_MS, until)) {
    steps.push(() => { data.legRows!.push(...queryLegRowsRange(db, from, to)); });
    steps.push(() => { data.modelRows!.push(...queryModelRowsRange(db, from, to)); });
  }
  steps.push(() => { data.hourRows = queryHourRows(db, now); });
  for (const [from, to] of daySliceBounds(now - 7 * DAY_MS, until)) {
    steps.push(() => {
      const part = queryCredentialCoverageRange(db, from, to);
      cred.n += part.n;
      cred.c += part.c;
    });
    steps.push(() => {
      for (const r of queryHourly7dRange(db, from, to)) {
        const acc = hourlyByHour.get(r.hour) ?? { hour: r.hour, calls: 0, tokens: 0 };
        acc.calls += r.calls;
        acc.tokens += r.tokens;
        hourlyByHour.set(r.hour, acc);
      }
    });
  }
  for (const [from, to] of daySliceBounds(now - 21 * DAY_MS, until)) {
    steps.push(() => { data.daily21d!.push(...queryDaily21dRange(db, from, to)); });
  }
  steps.push(() => {
    data.coverage = queryCoverage(db);
    // The day-of-week history is unbounded; slice its full span (set
    // by the coverage read) into 2-day chunks merged by summation.
    // (30-day chunks were tried first: with ~20 days of history that
    // was the whole scan in one step — measured 2.58 s, the one slow
    // poll hit in the post-deploy probe. 2-day chunks measure ~0.3 s.)
    const from = data.coverage.f ?? until;
    for (const [cf, ct] of chunkBounds(from, until, 2 * DAY_MS)) {
      steps.push(() => {
        for (const r of queryDowHistoryRange(db, cf, ct)) {
          const acc = dowByDow.get(r.dow) ?? { dow: r.dow, calls: 0, tokens: 0 };
          acc.calls += r.calls;
          acc.tokens += r.tokens;
          dowByDow.set(r.dow, acc);
        }
      });
    }
    steps.push(() => {
      data.credentialCoveragePct = cred.n > 0 ? (cred.c / cred.n) * 100 : null;
      data.hourly7d = [...hourlyByHour.values()].sort((a, b) => a.hour - b.hour);
      data.dowHistory = [...dowByDow.values()].sort((a, b) => a.dow - b.dow);
    });
  });
  let idx = 0;
  const step = (): void => {
    if (idx >= steps.length) {
      aggregatesCache = { path, at: Date.now(), data: data as AttemptAggregates };
      aggregatesRefreshing = false;
      onSettled?.();
      return;
    }
    const next = steps[idx++];
    try {
      next();
    } catch {
      // fail-soft: keep serving the stale copy; a later stale read retries
      aggregatesRefreshing = false;
      onSettled?.();
      return;
    }
    setTimeout(step, 0);
  };
  setTimeout(step, 0);
}

function loadAttemptAggregates(db: Database, path: string, now: number): AttemptAggregates {
  if (aggregatesCache && aggregatesCache.path === path) {
    if (now - aggregatesCache.at < AGGREGATES_TTL_MS) return aggregatesCache.data;
    refreshAttemptAggregates(db, path);
    return aggregatesCache.data;
  }
  const data = computeAttemptAggregates(db, now);
  aggregatesCache = { path, at: now, data };
  return data;
}

/**
 * Boot prime (#5082 follow-up, 2026-11-13): fill the aggregates cache
 * through the SLICED refresh instead of the synchronous first build.
 * The cold computeAttemptAggregates path measures ~12 s of
 * uninterrupted event-loop time over the live ledger; when server.ts
 * paid it inside the boot warm-up, every request that arrived during
 * the warm-up queued behind it — the first /api/program.json after a
 * restart measured 26.4 s while every later hit measured ~0.03 s.
 * Priming runs the same day-sliced steps as a stale refresh (widest
 * slice ~0.3 s, one per macrotask), so the loop stays responsive and
 * no request path ever pays the cold build at boot. A /quota request
 * that lands before the prime settles still takes the synchronous
 * cold path in loadAttemptAggregates — unchanged semantics, bounded
 * to that one request.
 *
 * Returns true when a prime was started. When there is nothing to do
 * (cache already populated for this DB, a refresh already in flight,
 * or no readable DB) it returns false and `onSettled` fires
 * immediately, so an awaiting caller never hangs; when a prime is
 * started, `onSettled` fires once it lands or aborts fail-soft.
 */
export function primeAttemptAggregates(
  opencodeDbPath: string = OPENCODE_DB_DEFAULT,
  onSettled?: () => void,
): boolean {
  if (aggregatesCache && aggregatesCache.path === opencodeDbPath) {
    onSettled?.();
    return false;
  }
  if (aggregatesRefreshing) return false;
  const db = getDb(opencodeDbPath);
  if (!db) {
    onSettled?.();
    return false;
  }
  refreshAttemptAggregates(db, opencodeDbPath, onSettled);
  return true;
}

function addInto(into: BurnBucket, r: { calls: number; success: number; input: number; output: number; reasoning: number; cache_read: number; cache_write: number; cost: number; cost_rows: number }): void {
  into.calls += r.calls;
  into.success += r.success;
  into.input += r.input;
  into.output += r.output;
  into.reasoning += r.reasoning;
  into.cacheRead += r.cache_read;
  into.cacheWrite += r.cache_write;
  into.costUsd += r.cost;
  into.costRows += r.cost_rows;
}

/** Local (America/Chicago on the laptop) ISO day for an instant. */
function localDay(ms: number): string {
  const d = new Date(ms);
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  return `${d.getFullYear()}-${mm}-${dd}`;
}

function dayRange(now: number, daysBack: number): Set<string> {
  const out = new Set<string>();
  for (let i = 0; i < daysBack; i++) out.add(localDay(now - i * DAY_MS));
  return out;
}

// ---------------------------------------------------------------------------
// Activity scan (tool calls / skills / mcpx / fleet) — slow over the
// transcript store, so it runs on a stale-while-revalidate cache: the
// first caller gets null + a background computation; later callers
// get the last computed value.
//
// #5082 follow-up (2026-11-13): the background computation used to be
// ONE synchronous block — measured 15.5 s of json_each scans over
// session_message for the 7d window (tool rows 7.4 s, skill rows
// 3.8 s, shell rows 4.0 s) — executed in a single setTimeout callback
// on the server's only event loop. Every request that arrived during
// the block stalled behind it: the hourly watch measured / at 17.5 s
// and /today at 9.7 s on its first pass (the no-await / shell has no
// other way to be slow), clean on the second pass. The computation
// now runs in 12-hour slices, one query per macrotask (the widest
// measured slice is ~0.1-0.5 s): range bounds let SQLite use the
// time_created index per slice, and per-slice counts merge by
// summation, so the totals are exactly the whole-window counts.
// ---------------------------------------------------------------------------

const ACTIVITY_TTL_MS = 600_000;
const ACTIVITY_SLICE_MS = 12 * HOUR_MS;
let activityCache: { at: number; data: ActivityCounts } | null = null;
let activityComputing = false;

/** Build the chunked activity computation: a list of single-query
 * steps (run one per macrotask by startActivityCompute) plus the
 * assembler that folds the accumulated slices into ActivityCounts. */
function activityComputePlan(db: Database, now: number): { steps: Array<() => void>; assemble: () => ActivityCounts } {
  const since = now - 7 * DAY_MS;
  const until = now + 1;
  const toolCounts = new Map<string, number>();
  const skillCounts = new Map<string, number>();
  const fleet = new Map<string, number>();
  let fleetTotal = 0;
  let sessionRow: { total: number; roots: number; subs: number } = { total: 0, roots: 0, subs: 0 };
  let turnCount = 0;
  const steps: Array<() => void> = [];
  for (const [from, to] of chunkBounds(since, until, ACTIVITY_SLICE_MS)) {
    steps.push(() => {
      const rows = db
        .query(
          `SELECT json_extract(j.value, '$.name') AS name, COUNT(*) AS c
             FROM session_message sm, json_each(sm.data, '$.content') j
            WHERE sm.type = 'assistant' AND sm.time_created >= ? AND sm.time_created < ?
              AND json_extract(j.value, '$.type') = 'tool'
            GROUP BY name`,
        )
        .all(from, to) as { name: string | null; c: number }[];
      for (const r of rows) if (r.name) toolCounts.set(r.name, (toolCounts.get(r.name) ?? 0) + r.c);
    });
    steps.push(() => {
      // Skill name rides at state.input.id on live parts (verified
      // 2026-11-13: {"input":{"id":"diagnosing-bugs"}}); input.name
      // kept as a fallback for other part generations.
      const rows = db
        .query(
          `SELECT COALESCE(json_extract(j.value, '$.state.input.id'), json_extract(j.value, '$.state.input.name')) AS name, COUNT(*) AS c
             FROM session_message sm, json_each(sm.data, '$.content') j
            WHERE sm.type = 'assistant' AND sm.time_created >= ? AND sm.time_created < ?
              AND json_extract(j.value, '$.type') = 'tool'
              AND json_extract(j.value, '$.name') = 'skill'
            GROUP BY name`,
        )
        .all(from, to) as { name: string | null; c: number }[];
      for (const r of rows) if (r.name) skillCounts.set(r.name, (skillCounts.get(r.name) ?? 0) + r.c);
    });
    steps.push(() => {
      // INFERRED fleet use: shell inputs mentioning the fleet binary.
      const rows = db
        .query(
          `SELECT json_extract(j.value, '$.state.input.command') AS command
             FROM session_message sm, json_each(sm.data, '$.content') j
            WHERE sm.type = 'assistant' AND sm.time_created >= ? AND sm.time_created < ?
              AND json_extract(j.value, '$.type') = 'tool'
              AND json_extract(j.value, '$.name') = 'shell'
              AND json_extract(j.value, '$.state.input.command') LIKE '%fleet%'`,
        )
        .all(from, to) as { command: string | null }[];
      for (const row of rows) {
        if (!row.command) continue;
        for (const m of row.command.matchAll(/(?:^|[\s;|&\/])fleet\s+([a-z][a-z0-9-]*)/g)) {
          fleet.set(m[1], (fleet.get(m[1]) ?? 0) + 1);
          fleetTotal++;
        }
      }
    });
  }
  steps.push(() => {
    sessionRow = db
      .query(
        `SELECT COUNT(*) AS total,
                SUM(parent_id IS NULL) AS roots,
                SUM(parent_id IS NOT NULL) AS subs
           FROM session_v2 WHERE time_created >= ?`,
      )
      .get(since) as { total: number; roots: number; subs: number };
  });
  steps.push(() => {
    const row = db
      .query(`SELECT COUNT(*) AS c FROM session_message WHERE type = 'user' AND time_created >= ?`)
      .get(since) as { c: number };
    turnCount = row.c;
  });
  const assemble = (): ActivityCounts => {
    const byName = toolCounts;
    const total = [...byName.values()].reduce((s, c) => s + c, 0);
    const topTools = [...byName.entries()]
      .map(([name, count]) => ({ name, count }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 12);
    const topSkills = [...skillCounts.entries()]
      .map(([name, count]) => ({ name, count }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 10);
    return {
      rangeLabel: "last 7 days",
      sessions: sessionRow.total,
      rootSessions: sessionRow.roots,
      subSessions: sessionRow.subs,
      turns: turnCount,
      toolCallsTotal: total,
      topTools,
      skillTotal: byName.get("skill") ?? 0,
      topSkills,
      mcpx: ["mcpx_discover", "mcpx_exec", "mcpx_project"]
        .filter((n) => byName.has(n))
        .map((n) => ({ name: n, count: byName.get(n) as number })),
      executeGateway: byName.get("execute") ?? 0,
      fleetInferred: [...fleet.entries()].sort((a, b) => b[1] - a[1]).slice(0, 12).map(([subcommand, count]) => ({ subcommand, count })),
      fleetInferredTotal: fleetTotal,
    };
  };
  return { steps, assemble };
}

function startActivityCompute(db: Database): void {
  activityComputing = true;
  const { steps, assemble } = activityComputePlan(db, Date.now());
  let idx = 0;
  const step = (): void => {
    if (idx >= steps.length) {
      try {
        activityCache = { at: Date.now(), data: assemble() };
      } catch {
        // fail-soft: leave the cache as-is; a later call retries
      } finally {
        activityComputing = false;
      }
      return;
    }
    const next = steps[idx++];
    try {
      next();
    } catch {
      activityComputing = false;
      return;
    }
    setTimeout(step, 0);
  };
  setTimeout(step, 0);
}

function loadActivity(db: Database, now: number): ActivityCounts | null {
  if (activityCache && now - activityCache.at < ACTIVITY_TTL_MS) return activityCache.data;
  if (!activityComputing) startActivityCompute(db);
  return activityCache?.data ?? null;
}

// ---------------------------------------------------------------------------
// Pool leg attribution
// ---------------------------------------------------------------------------

/** How each quota pool's burn is attributed from route_attempt.
 * "credential": the gatherer's credential id for the pool's service
 * (read live from quota-current.json) — the exact join.
 * "provider-id": no credential join exists for the pool; the route
 * legs are matched by provider_id (the label says so in the UI).
 * "shared-credential": both Antigravity pools ride ONE credential —
 * both cards show the same burn, labelled shared. */
const POOL_LEG_BASIS: Record<string, { basis: NonNullable<PoolAnalytics["legBasis"]>; gathererProviders?: string[]; providerIds?: string[] }> = {
  "_openai.main": { basis: "credential", gathererProviders: ["_openai.main"] },
  "_openai.alt": { basis: "credential", gathererProviders: ["_openai.alt"] },
  "opencode-go.drewrypope": { basis: "credential", gathererProviders: ["opencode-go.drewrypope"] },
  "opencode-go.popesdrew": { basis: "credential", gathererProviders: ["opencode-go.popesdrew"] },
  "opencode-go.developing-today-0": { basis: "credential", gathererProviders: ["opencode-go.developing-today-0"] },
  "cline-pass": { basis: "credential", gathererProviders: ["cline-pass"] },
  _commandcode: { basis: "credential", gathererProviders: ["_commandcode"] },
  mistral: { basis: "credential", gathererProviders: ["mistral"] },
  // Antigravity attempts carry NO credential id (verified live:
  // 21.8k rows, all NULL) — the credential join can never match, so
  // both pools attribute by provider id over the same leg set (the
  // two pools share those legs; each card says so via its label).
  "_antigravity.gemini": { basis: "provider-id", providerIds: ["antigravity-direct", "_antigravity", "antigravity-ls", "antigravity-agy"] },
  "_antigravity.claude-gpt": { basis: "provider-id", providerIds: ["antigravity-direct", "_antigravity", "antigravity-ls", "antigravity-agy"] },
  opendesign: { basis: "provider-id", providerIds: ["opendesign"] },
  "anthropic:claude-pro": { basis: "provider-id", providerIds: ["_claude", "claude-code"] },
  meta: { basis: "provider-id", providerIds: ["_muse", "muse-direct"] },
  deepseek: { basis: "provider-id", providerIds: ["deepseek"] },
};

function readCredentialMap(quotaCurrentPath: string): Map<string, string> {
  const out = new Map<string, string>();
  try {
    const doc = JSON.parse(readFileSync(quotaCurrentPath, "utf8")) as {
      providers?: { providerId?: string; credentialId?: string }[];
    };
    for (const p of doc.providers ?? []) {
      if (p.providerId && p.credentialId && !out.has(p.providerId)) out.set(p.providerId, p.credentialId);
    }
  } catch {
    // fail-soft: credential pools simply get no attribution
  }
  return out;
}

// ---------------------------------------------------------------------------
// Percent series for projections, from the snapshot store
// ---------------------------------------------------------------------------

const WINDOW_METRIC_ALIASES: Record<string, string[]> = {
  "5-hour": ["5h", "5-hour"],
  "7d": ["7d", "weekly"],
  weekly: ["weekly", "7d"],
  monthly: ["monthly"],
  api: ["api"],
  vibe: ["vibe"],
};

function seriesForWindow(history: UsageView["history"], pool: QuotaPool, windowName: string): PctPoint[] {
  // Snapshot service ids drifted across writer generations: the same
  // pool appears as `_openai.main`, `openai.main`, `cline-pass:
  // cline-pass`, `antigravity:gemini` (for pool `_antigravity
  // .gemini`)... Normalize both sides (strip a leading underscore,
  // treat ':' as '.') and accept the pool's own id plus sub-ids
  // under it (`cline-pass.cline-pass` for pool `cline-pass`).
  const normSvc = (s: string) => s.replace(/^_/, "").replace(/:/g, ".");
  const poolSvc = normSvc(pool.id);
  const metrics = new Set<string>(WINDOW_METRIC_ALIASES[windowName] ?? [windowName]);
  const points: PctPoint[] = [];
  const seen = new Set<string>();
  for (const [serviceId, records] of Object.entries(history)) {
    const ns = normSvc(serviceId);
    if (ns !== poolSvc && !ns.startsWith(`${poolSvc}.`)) continue;
    for (const r of records as SnapshotRecord[]) {
      if (typeof r.value !== "number" || r.unit !== "pct") continue;
      if (!metrics.has(r.metricId) && !r.metricId.endsWith(`:${windowName}`) && !r.metricId.endsWith(`:${WINDOW_METRIC_ALIASES[windowName]?.[0] ?? windowName}`)) continue;
      const ts = Date.parse(r.ts);
      if (!Number.isFinite(ts)) continue;
      const key = `${ts}|${r.value}`;
      if (seen.has(key)) continue;
      seen.add(key);
      points.push({ ts, pct: r.value, resetsAt: r.resetsAt ?? null });
    }
  }
  return points.sort((a, b) => a.ts - b.ts);
}

function percentOf(w: QuotaPool["windows"][number]): number {
  if (typeof w.percent === "number") return w.percent;
  return w.limit ? (w.used / w.limit) * 100 : 0;
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

export function buildQuotaAnalytics(args: {
  quota: QuotaView;
  usage: UsageView;
  quotaCurrentPath: string;
  opencodeDbPath?: string;
  now?: number;
}): QuotaAnalytics {
  const now = args.now ?? Date.now();
  const dbPath = args.opencodeDbPath ?? OPENCODE_DB_DEFAULT;
  const db = getDb(dbPath);
  const base: QuotaAnalytics = {
    generatedAt: new Date(now).toISOString(),
    now,
    dbAvailable: !!db,
    attemptsFrom: null,
    attemptsTo: null,
    credentialCoveragePct: null,
    pools: [],
    totals: { lastHour: emptyBurn(), today: emptyBurn(), last7d: emptyBurn(), prev7d: emptyBurn() },
    totalsWowTokens: wowDelta(0, 0),
    byModel7d: [],
    byEffort7d: [],
    byLeg7d: [],
    hourly7d: [],
    dowHistory: [],
    daily21d: [],
    activity7d: null,
  };
  if (!db) return base;
  try {
    const aggregates = loadAttemptAggregates(db, dbPath, now);
    const { legRows, modelRows } = aggregates;
    base.attemptsFrom = aggregates.coverage.f ? new Date(aggregates.coverage.f).toISOString() : null;
    base.attemptsTo = aggregates.coverage.t ? new Date(aggregates.coverage.t).toISOString() : null;

    const today = localDay(now);
    const last7 = dayRange(now, 7);
    const prev7 = new Set([...dayRange(now - 7 * DAY_MS, 7)]);
    const sumWhere = (rows: DayLegRow[] | DayModelRow[], pred: (day: string) => boolean): BurnBucket => {
      const b = emptyBurn();
      for (const r of rows) if (pred(r.day)) addInto(b, r);
      return b;
    };
    base.totals.today = sumWhere(modelRows, (d) => d === today);
    base.totals.last7d = sumWhere(modelRows, (d) => last7.has(d));
    base.totals.prev7d = sumWhere(modelRows, (d) => prev7.has(d));
    base.totalsWowTokens = wowDelta(
      base.totals.last7d.input + base.totals.last7d.output + base.totals.last7d.cacheRead + base.totals.last7d.cacheWrite + base.totals.last7d.reasoning,
      base.totals.prev7d.input + base.totals.prev7d.output + base.totals.prev7d.cacheRead + base.totals.prev7d.cacheWrite + base.totals.prev7d.reasoning,
    );
    base.totals.lastHour = sumWhere(aggregates.hourRows, () => true);

    const group = (rows: DayModelRow[], keyOf: (r: DayModelRow) => { key: string; label: string }): BurnGroup[] => {
      const map = new Map<string, BurnGroup>();
      for (const r of rows) {
        if (!last7.has(r.day)) continue;
        const { key, label } = keyOf(r);
        const g = map.get(key) ?? { key, label, ...emptyBurn() };
        addInto(g, r);
        map.set(key, g);
      }
      return [...map.values()].sort((a, b) => b.calls - a.calls);
    };
    base.byModel7d = group(modelRows, (r) => ({ key: r.model_id, label: r.model_id }));
    base.byEffort7d = group(modelRows, (r) => {
      const v = r.variant && r.variant !== "default" ? r.variant : "(default)";
      return { key: v, label: v };
    });
    const legGroups = new Map<string, BurnGroup>();
    for (const r of legRows) {
      if (!last7.has(r.day)) continue;
      const g = legGroups.get(r.provider_id) ?? { key: r.provider_id, label: r.provider_id, ...emptyBurn() };
      addInto(g, r);
      legGroups.set(r.provider_id, g);
    }
    base.byLeg7d = [...legGroups.values()].sort((a, b) => b.calls - a.calls);

    base.credentialCoveragePct = aggregates.credentialCoveragePct;

    base.hourly7d = aggregates.hourly7d;
    base.dowHistory = aggregates.dowHistory;
    base.daily21d = aggregates.daily21d.map((d) => ({ ...d, holiday: usFederalHoliday(d.day) }));

    // ---- pools -------------------------------------------------------------
    const credMap = readCredentialMap(args.quotaCurrentPath);
    const legBurn = (poolId: string, pred: (day: string) => boolean): { burn: BurnBucket | null; basis: PoolAnalytics["legBasis"]; label: string | null } => {
      const spec = POOL_LEG_BASIS[poolId];
      if (!spec) return { burn: null, basis: null, label: null };
      const credIds = new Set((spec.gathererProviders ?? []).map((p) => credMap.get(p)).filter(Boolean) as string[]);
      const provIds = new Set(spec.providerIds ?? []);
      if (spec.basis !== "provider-id" && credIds.size === 0) return { burn: null, basis: spec.basis, label: null };
      const b = emptyBurn();
      let any = false;
      for (const r of legRows) {
        if (!pred(r.day)) continue;
        const match = spec.basis === "provider-id" ? provIds.has(r.provider_id) : r.credential_id != null && credIds.has(r.credential_id);
        if (match) {
          addInto(b, r);
          any = true;
        }
      }
      const label =
        spec.basis === "provider-id"
          ? [...provIds].join(", ")
          : [...(spec.gathererProviders ?? [])].join(", ") + (spec.basis === "shared-credential" ? " (one credential, both Antigravity pools)" : "");
      return { burn: any ? b : null, basis: spec.basis, label };
    };

    base.pools = args.quota.pools.map((pool) => {
      const windows: PoolWindowAnalytics[] = pool.windows.map((w) => {
        const percent = percentOf(w);
        const series = seriesForWindow(args.usage.history, pool, w.name);
        const projection = projectWindow({ points: series, pctNow: percent, resetsAt: w.resetsAt, now });
        let wow: WowDelta | null = null;
        // Duration-until-wall WoW is only meaningful when BOTH the
        // current and the as-of-last-week projections are real
        // projections — a walled window has duration 0 by fiat and a
        // nearly-flat past slope manufactures absurd durations.
        if (projection.state === "projected" && projection.durationUntilWallMs != null) {
          const then = now - 7 * DAY_MS;
          const past = series.filter((p) => p.ts <= then);
          if (past.length) {
            const last = past[past.length - 1];
            const pastProjection = projectWindow({ points: past, pctNow: last.pct, resetsAt: last.resetsAt, now: then });
            if (pastProjection.state === "projected" && pastProjection.durationUntilWallMs != null) {
              wow = wowDelta(projection.durationUntilWallMs / HOUR_MS, pastProjection.durationUntilWallMs / HOUR_MS);
            }
          }
        }
        return {
          name: w.name,
          percent,
          resetsAt: w.resetsAt,
          durationUntilResetMs: w.resetsAt ? Date.parse(w.resetsAt) - now : null,
          projection,
          wowDurationUntilWall: wow,
        };
      });
      const binding =
        windows.find((w) => w.projection.state === "walled") ??
        windows.filter((w) => w.projection.projectedWallAt != null).sort((a, b) => (a.projection.projectedWallAt as number) - (b.projection.projectedWallAt as number))[0] ??
        windows.slice().sort((a, b) => b.percent - a.percent)[0] ??
        null;
      const unlock = binding
        ? unlockPicture(
            pool.windows.map((w) => ({ name: w.name, percent: percentOf(w), resetsAt: w.resetsAt })),
            { name: binding.name, percent: binding.percent, resetsAt: binding.resetsAt },
          )
        : null;
      const today7 = legBurn(pool.id, (d) => d === today);
      const week = legBurn(pool.id, (d) => last7.has(d));
      const prev = legBurn(pool.id, (d) => prev7.has(d));
      const tok = (b: BurnBucket | null) => (b ? b.input + b.output + b.reasoning + b.cacheRead + b.cacheWrite : 0);
      return {
        poolId: pool.id,
        provider: pool.provider,
        plan: pool.plan,
        state: pool.state,
        windows,
        bindingWindow: binding?.name ?? null,
        unlock,
        burnToday: today7.burn,
        burn7d: week.burn,
        burnWowTokens: week.burn && prev.burn ? wowDelta(tok(week.burn), tok(prev.burn)) : null,
        legBasis: week.basis,
        legLabel: week.label,
      } satisfies PoolAnalytics;
    });

    base.activity7d = loadActivity(db, now);
    return base;
  } catch {
    // Schema drift or a locked DB must never break /quota: the
    // section renders its unavailable state from the base model.
    return { ...base, dbAvailable: false };
  }
}

export { tokenRatios };
