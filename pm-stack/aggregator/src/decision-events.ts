// Decision-set EVENT LOG sync (#9335 Track B — the external pipeline).
//
// Track A gives every decision set an append-only structured event log
// on the laptop: one event per interaction (submit, defer, request-info,
// answer edit, typed-but-unsubmitted draft change, set raised, ...).
// This module is the OUTBOUND half: it defines the wire shape those
// events travel in, reads the local outbox Track A appends to, and
// syncs it — idempotently, keyed by `eventId` — to the two external
// sinks Drew ordered:
//
//   1. Turso, following the fleet-platform pattern of code#5690: the
//      libSQL HTTP pipeline (POST <db>/v2/pipeline, typed args, never
//      string interpolation) with INSERT OR IGNORE on the event_id PK,
//      exactly like scripts/gather-quota.ts `writeTursoRecords` writes
//      agent_quota_records. Target DB: fleet-platform, table
//      `decision_set_events`; token resolved BY NAME
//      (`turso.fleet-platform-rw-token`, see src/secrets.ts) — the value
//      is never logged, filed, or printed.
//   2. The decision-events Cloudflare Worker
//      (pm-stack/decision-events-worker), which stores the same events
//      in D1 and fans them out through a Durable Object per set. The
//      sync POSTs batches to the Worker's authenticated webhook; the
//      shared secret is likewise resolved BY NAME
//      (`cloudflare.decision-events.webhook-secret`).
//
// Outbox contract (the interface between Track A and this sync): a
// JSONL file, one event per line, in the shape below. Default path
// mirrors the other aggregator stores (~/.local/state/pm-aggregator/);
// DECISION_EVENTS_OUTBOX_FILE overrides it, and the sync script also
// accepts --file / --outbox-dir for any other producer. A line that
// fails validation is reported and skipped, never silently dropped and
// never allowed to poison the batch. Delivery is at-least-once: the
// per-leg sync state (acked eventIds) only advances for events a sink
// accepted, and both sinks dedupe on eventId, so re-running the sync
// is always safe.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { isIsoDate } from "./validate.ts";

/** One decision-set activity event — the #9335 wire shape. */
export interface DecisionSetEvent {
  /** Stable unique id assigned by the producer; the dedupe key in
   * every sink (Turso PK, D1 PK, Durable Object fan-out). */
  eventId: string;
  /** The decision set this event belongs to (the set's UUID/slug). */
  setId: string;
  /** The question the event concerns (`<ledgerId>#<index>` per
   * decisions.ts), or null for set-level events (raised, submitted). */
  questionRef: string | null;
  /** Event kind slug: set-raised | question-answered | question-deferred
   * | info-requested | answer-edited | draft-changed | set-submitted |
   * ... Producers may add kinds; consumers must tolerate unknown ones. */
  type: string;
  /** Kind-specific structured detail; always an object ({} if none). */
  payload: Record<string, unknown>;
  /** ISO timestamp of when the event happened (producer clock). */
  ts: string;
}

/** Event kinds the current producers emit; validation accepts any
 * slug-shaped kind so new kinds never need a sync deploy first. */
export const DECISION_EVENT_TYPES: readonly string[] = [
  "set-raised",
  "question-answered",
  "question-deferred",
  "info-requested",
  "answer-edited",
  "draft-changed",
  "set-submitted",
];

export const DECISION_EVENTS_TURSO_URL = "https://fleet-platform-dezren39.aws-us-east-1.turso.io";
export const DECISION_EVENTS_TURSO_TOKEN_NAME = "turso.fleet-platform-rw-token";
export const DECISION_EVENTS_WEBHOOK_SECRET_NAME = "cloudflare.decision-events.webhook-secret";
export const DECISION_SET_EVENTS_TABLE = "decision_set_events";

/** DDL for the Turso table (identical DDL runs in the Worker's D1 —
 * keep pm-stack/decision-events-worker/schema.sql in step). */
export const DECISION_SET_EVENTS_DDL: readonly string[] = [
  `CREATE TABLE IF NOT EXISTS ${DECISION_SET_EVENTS_TABLE} (
     event_id TEXT PRIMARY KEY,
     set_id TEXT NOT NULL,
     question_ref TEXT,
     type TEXT NOT NULL,
     payload TEXT NOT NULL,
     ts TEXT NOT NULL
   )`,
  `CREATE INDEX IF NOT EXISTS decision_set_events_set_ts ON ${DECISION_SET_EVENTS_TABLE} (set_id, ts)`,
  `CREATE INDEX IF NOT EXISTS decision_set_events_type_ts ON ${DECISION_SET_EVENTS_TABLE} (type, ts)`,
];

/** Default outbox path (mirrors defaultDecisionsFile in decisions.ts). */
export function defaultDecisionEventsOutboxFile(
  env: Record<string, string | undefined> = process.env,
): string {
  const v = env.DECISION_EVENTS_OUTBOX_FILE?.trim();
  return v ? v : join(homedir(), ".local", "state", "pm-aggregator", "decision-events-outbox.jsonl");
}

/** Default per-leg sync-state path: beside the outbox file. */
export function defaultDecisionEventsStateFile(
  env: Record<string, string | undefined> = process.env,
): string {
  const v = env.DECISION_EVENTS_STATE_FILE?.trim();
  return v ? v : `${defaultDecisionEventsOutboxFile(env)}.sync-state.json`;
}

const ID_RE = /^\S{1,128}$/;
const TYPE_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$/;
const MAX_PAYLOAD_BYTES = 64 * 1024;

export function validateDecisionEvent(
  value: unknown,
): { ok: true; value: DecisionSetEvent } | { ok: false; error: string } {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { ok: false, error: "event must be an object" };
  }
  const b = value as Record<string, unknown>;
  const eventId = typeof b.eventId === "string" ? b.eventId.trim() : "";
  if (!ID_RE.test(eventId)) return { ok: false, error: "event.eventId required (no whitespace, <= 128 chars)" };
  const setId = typeof b.setId === "string" ? b.setId.trim() : "";
  if (!ID_RE.test(setId)) return { ok: false, error: "event.setId required (no whitespace, <= 128 chars)" };
  let questionRef: string | null = null;
  if (b.questionRef !== undefined && b.questionRef !== null) {
    if (typeof b.questionRef !== "string" || !ID_RE.test(b.questionRef.trim())) {
      return { ok: false, error: "event.questionRef must be a non-empty string or null" };
    }
    questionRef = b.questionRef.trim();
  }
  const type = typeof b.type === "string" ? b.type.trim() : "";
  if (!TYPE_RE.test(type)) return { ok: false, error: "event.type required (slug, <= 80 chars)" };
  let payload: Record<string, unknown> = {};
  if (b.payload !== undefined) {
    if (!b.payload || typeof b.payload !== "object" || Array.isArray(b.payload)) {
      return { ok: false, error: "event.payload must be an object" };
    }
    payload = b.payload as Record<string, unknown>;
    if (JSON.stringify(payload).length > MAX_PAYLOAD_BYTES) {
      return { ok: false, error: "event.payload exceeds 64 KiB" };
    }
  }
  if (typeof b.ts !== "string" || !isIsoDate(b.ts)) {
    return { ok: false, error: "event.ts must be an ISO timestamp string" };
  }
  return { ok: true, value: { eventId, setId, questionRef, type, payload, ts: b.ts } };
}

export interface OutboxRead {
  events: DecisionSetEvent[];
  /** Lines that failed to parse/validate, with 1-based line numbers. */
  errors: { line: number; error: string }[];
  /** eventIds seen more than once in the file (first copy kept). */
  duplicates: string[];
}

/** Read + validate a JSONL outbox file. A missing file is an empty
 * outbox, not an error (the producer may not have run yet). */
export async function readOutboxEvents(path: string): Promise<OutboxRead> {
  let raw: string;
  try {
    raw = await readFile(path, "utf8");
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === "ENOENT") return { events: [], errors: [], duplicates: [] };
    throw err;
  }
  const events: DecisionSetEvent[] = [];
  const errors: { line: number; error: string }[] = [];
  const duplicates: string[] = [];
  const seen = new Set<string>();
  const lines = raw.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = (lines[i] as string).trim();
    if (!line) continue;
    let parsed: unknown;
    try {
      parsed = JSON.parse(line);
    } catch {
      errors.push({ line: i + 1, error: "invalid JSON" });
      continue;
    }
    const v = validateDecisionEvent(parsed);
    if (!v.ok) {
      errors.push({ line: i + 1, error: v.error });
      continue;
    }
    if (seen.has(v.value.eventId)) {
      duplicates.push(v.value.eventId);
      continue;
    }
    seen.add(v.value.eventId);
    events.push(v.value);
  }
  return { events, errors, duplicates };
}

// --- libSQL HTTP pipeline client (typed args; cf. pkgs/github-ci/worker) ---

export type TursoArg = { type: "null" } | { type: "text"; value: string } | { type: "integer"; value: string };

export const tArg = (v: string | null | undefined): TursoArg =>
  v === null || v === undefined ? { type: "null" } : { type: "text", value: v };

export interface TursoStatement {
  sql: string;
  args?: TursoArg[];
}

interface PipelineResult {
  type: string;
  error?: { message?: string };
  // libSQL answers each execute with type "ok" and the count under
  // `affected_row_count` (verified live against fleet-platform,
  // 2026-11-12); `rows_affected` is accepted defensively.
  response?: { result?: { rows?: TursoArg[][]; affected_row_count?: number; rows_affected?: number } };
}

export type FetchFn = (url: string, init: RequestInit) => Promise<Response>;

/** Run statements in one /v2/pipeline call; throws on any error result
 * or a malformed envelope (a 200 that is not the pipeline shape must
 * never read as a successful write — #6949's contract). */
export async function tursoPipeline(
  url: string,
  token: string,
  stmts: TursoStatement[],
  fetchFn: FetchFn = fetch,
): Promise<PipelineResult[]> {
  if (!url || !token) throw new Error("turso url/token unset");
  const res = await fetchFn(`${url.replace(/\/+$/, "")}/v2/pipeline`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: JSON.stringify({
      requests: [
        ...stmts.map((s) => ({ type: "execute", stmt: s.args ? { sql: s.sql, args: s.args } : { sql: s.sql } })),
        { type: "close" },
      ],
    }),
  });
  if (!res.ok) throw new Error(`turso pipeline failed: HTTP ${res.status}`);
  let out: { results?: PipelineResult[] };
  try {
    out = (await res.json()) as { results?: PipelineResult[] };
  } catch {
    throw new Error("turso pipeline failed: malformed response (not JSON)");
  }
  if (!out || typeof out !== "object" || !Array.isArray(out.results)) {
    throw new Error("turso pipeline failed: malformed response (missing results)");
  }
  const failed = out.results.find((r) => r?.type === "error");
  if (failed) throw new Error(`turso pipeline failed: ${failed.error?.message ?? "pipeline error"}`);
  return out.results;
}

export function decisionEventInsertStatement(e: DecisionSetEvent): TursoStatement {
  return {
    sql: `INSERT OR IGNORE INTO ${DECISION_SET_EVENTS_TABLE} (event_id, set_id, question_ref, type, payload, ts) VALUES (?, ?, ?, ?, ?, ?)`,
    args: [tArg(e.eventId), tArg(e.setId), tArg(e.questionRef), tArg(e.type), tArg(JSON.stringify(e.payload)), tArg(e.ts)],
  };
}

export async function ensureDecisionEventsTable(url: string, token: string, fetchFn: FetchFn = fetch): Promise<void> {
  await tursoPipeline(url, token, DECISION_SET_EVENTS_DDL.map((sql) => ({ sql })), fetchFn);
}

/** Insert events into Turso in pipeline batches. INSERT OR IGNORE on
 * the event_id PK makes re-runs no-ops; the return counts only rows
 * actually inserted (rows_affected), so idempotence is measured, not
 * assumed. Throws on the first failed batch — events in later batches
 * are simply still pending for the next run. */
export async function insertDecisionEventsTurso(
  url: string,
  token: string,
  events: DecisionSetEvent[],
  fetchFn: FetchFn = fetch,
  batchSize = 100,
): Promise<{ attempted: number; inserted: number }> {
  let inserted = 0;
  for (let i = 0; i < events.length; i += batchSize) {
    const batch = events.slice(i, i + batchSize);
    const results = await tursoPipeline(url, token, batch.map(decisionEventInsertStatement), fetchFn);
    for (const r of results) inserted += r.response?.result?.affected_row_count ?? r.response?.result?.rows_affected ?? 0;
  }
  return { attempted: events.length, inserted };
}

/** POST one batch to the decision-events Worker webhook. The Worker
 * answers 200 with {accepted, inserted, duplicates}; a non-200 (or a
 * 200 without that shape) is a failed leg, never a silent success. */
export async function postDecisionEventsWebhook(
  webhookUrl: string,
  secret: string,
  events: DecisionSetEvent[],
  fetchFn: FetchFn = fetch,
  batchSize = 100,
): Promise<{ attempted: number; inserted: number }> {
  if (!webhookUrl || !secret) throw new Error("webhook url/secret unset");
  let inserted = 0;
  for (let i = 0; i < events.length; i += batchSize) {
    const batch = events.slice(i, i + batchSize);
    const res = await fetchFn(webhookUrl, {
      method: "POST",
      headers: { Authorization: `Bearer ${secret}`, "Content-Type": "application/json" },
      body: JSON.stringify({ events: batch }),
      signal: AbortSignal.timeout(30_000),
    });
    if (!res.ok) throw new Error(`webhook failed: HTTP ${res.status}`);
    const out = (await res.json().catch(() => null)) as { inserted?: unknown } | null;
    if (!out || typeof out.inserted !== "number") throw new Error("webhook failed: malformed response");
    inserted += out.inserted;
  }
  return { attempted: events.length, inserted };
}

// --- Per-leg sync state (which eventIds each sink has acked) ---

export type SyncLeg = "turso" | "webhook";
export const SYNC_LEGS: readonly SyncLeg[] = ["turso", "webhook"];

export interface SyncState {
  version: 1;
  acked: Record<SyncLeg, string[]>;
  updatedAt: string;
}

export function emptySyncState(): SyncState {
  return { version: 1, acked: { turso: [], webhook: [] }, updatedAt: new Date(0).toISOString() };
}

export async function loadSyncState(path: string): Promise<SyncState> {
  try {
    const raw = JSON.parse(await readFile(path, "utf8")) as Partial<SyncState>;
    if (raw.version !== 1 || !raw.acked) return emptySyncState();
    return {
      version: 1,
      acked: {
        turso: Array.isArray(raw.acked.turso) ? raw.acked.turso.filter((x) => typeof x === "string") : [],
        webhook: Array.isArray(raw.acked.webhook) ? raw.acked.webhook.filter((x) => typeof x === "string") : [],
      },
      updatedAt: typeof raw.updatedAt === "string" ? raw.updatedAt : new Date(0).toISOString(),
    };
  } catch {
    return emptySyncState(); // missing/corrupt state re-syncs; sinks dedupe
  }
}

export async function saveSyncState(path: string, state: SyncState): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const tmp = `${path}.tmp-${process.pid}`;
  await writeFile(tmp, `${JSON.stringify(state, null, 2)}\n`, "utf8");
  await rename(tmp, path);
}

export function pendingForLeg(events: DecisionSetEvent[], state: SyncState, leg: SyncLeg): DecisionSetEvent[] {
  const acked = new Set(state.acked[leg]);
  return events.filter((e) => !acked.has(e.eventId));
}

export function markAcked(state: SyncState, leg: SyncLeg, events: DecisionSetEvent[]): void {
  const acked = new Set(state.acked[leg]);
  for (const e of events) acked.add(e.eventId);
  state.acked[leg] = [...acked];
  state.updatedAt = new Date().toISOString();
}

// --- Orchestration: sync pending events to each requested leg ---

export interface LegReport {
  leg: SyncLeg;
  pending: number;
  attempted: number;
  inserted: number;
  error?: string;
}

export interface SyncOptions {
  events: DecisionSetEvent[];
  state: SyncState;
  legs: SyncLeg[];
  tursoUrl?: string;
  tursoToken?: string | null;
  webhookUrl?: string;
  webhookSecret?: string | null;
  fetchFn?: FetchFn;
  batchSize?: number;
  /** Skip the CREATE TABLE IF NOT EXISTS pass (tests / read-only tokens). */
  skipEnsureSchema?: boolean;
}

/** Sync every pending event to each leg independently: one leg's
 * failure never blocks the other, and state advances only for events
 * a leg accepted (the caller persists state). Never throws for a leg
 * failure — the report carries it; the caller decides the exit code. */
export async function syncDecisionEvents(opts: SyncOptions): Promise<LegReport[]> {
  const fetchFn = opts.fetchFn ?? fetch;
  const reports: LegReport[] = [];
  for (const leg of opts.legs) {
    const pending = pendingForLeg(opts.events, opts.state, leg);
    const report: LegReport = { leg, pending: pending.length, attempted: 0, inserted: 0 };
    reports.push(report);
    if (pending.length === 0) continue;
    try {
      if (leg === "turso") {
        if (!opts.tursoToken) throw new Error(`turso token unresolved by name (${DECISION_EVENTS_TURSO_TOKEN_NAME})`);
        const url = opts.tursoUrl ?? DECISION_EVENTS_TURSO_URL;
        if (!opts.skipEnsureSchema) await ensureDecisionEventsTable(url, opts.tursoToken, fetchFn);
        const r = await insertDecisionEventsTurso(url, opts.tursoToken, pending, fetchFn, opts.batchSize);
        report.attempted = r.attempted;
        report.inserted = r.inserted;
      } else {
        if (!opts.webhookUrl) throw new Error("webhook URL unset (DECISION_EVENTS_WEBHOOK_URL)");
        if (!opts.webhookSecret) {
          throw new Error(`webhook secret unresolved by name (${DECISION_EVENTS_WEBHOOK_SECRET_NAME})`);
        }
        const r = await postDecisionEventsWebhook(opts.webhookUrl, opts.webhookSecret, pending, fetchFn, opts.batchSize);
        report.attempted = r.attempted;
        report.inserted = r.inserted;
      }
      markAcked(opts.state, leg, pending);
    } catch (err) {
      report.error = err instanceof Error ? err.message : String(err);
    }
  }
  return reports;
}
