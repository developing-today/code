// Decision-set EVENT LOG (#9335, Decision Sets v2 Track A).
//
// Append-only structured events for EVERY decision-set interaction:
// set creation, submits, defers, request-info, answer edits and draft
// (typed-but-not-submitted) changes reported by the set page. The log
// is the local source the activity processor polls; the Turso /
// Cloudflare sync leg (a separate track) drains the OUTBOX.
//
// Storage (both files live next to the decisions store):
//   - events file: JSONL, one DecisionEvent per line, append-only.
//     Default: <state dir>/decision-events.jsonl
//     (PM_AGG_DECISION_EVENTS_FILE overrides).
//   - outbox file: JSONL, one DecisionEventOutboxRecord per line —
//     the same event wrapped with { outbox: { status: "pending",
//     enqueuedAt, sink } }. The sync leg ships pending records to
//     Turso/Cloudflare and rewrites/compacts the outbox as it acks;
//     this track only guarantees every appended event lands in the
//     outbox exactly once, in append order.
//
// Event shape (the contract the activity processor + sync leg read):
//   { eventId, setId, questionRef?, type, payload, ts }
//   questionRef = { ledgerId, qid, questionIndex? } — qid is
//   `<ledgerId>#<1-based index>`, matching DecisionQuestion.qid.
// Read contract: readDecisionEvents(eventsFile, setId, { since })
// returns the set`s events in append order plus nextCursor (the last
// returned eventId; pass it back as `since` to page forward). `since`
// may be an eventId (events strictly after it), an ISO timestamp
// (events with ts after it), or absent (all events for the set).
import { randomUUID } from "node:crypto";
import { appendFile, mkdir, readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

export const DECISION_EVENT_TYPES = [
  "set.created",
  "submit",
  "defer",
  "request-info",
  "draft",
  "edit",
  "answer",
] as const;
export type DecisionEventType = (typeof DECISION_EVENT_TYPES)[number];

export function isDecisionEventType(value: unknown): value is DecisionEventType {
  return typeof value === "string" && (DECISION_EVENT_TYPES as readonly string[]).includes(value);
}

export interface DecisionEventQuestionRef {
  ledgerId: string;
  qid: string;
  /** 1-based index of the question within its ledger item, when known. */
  questionIndex?: number;
}

export interface DecisionEvent {
  eventId: string;
  setId: string;
  questionRef?: DecisionEventQuestionRef;
  type: DecisionEventType;
  payload: Record<string, unknown>;
  /** ISO-8601 append time (server clock). */
  ts: string;
}

export interface DecisionEventOutboxRecord extends DecisionEvent {
  outbox: {
    status: "pending";
    enqueuedAt: string;
    /** Intended downstream: the Turso + Cloudflare Worker/D1/DO leg. */
    sink: "turso-cloudflare";
  };
}

export interface DecisionEventInput {
  setId: string;
  type: DecisionEventType;
  questionRef?: DecisionEventQuestionRef;
  payload?: Record<string, unknown>;
  ts?: string;
}

/** Default events file: env override, else next to the decisions store. */
export function defaultDecisionEventsFile(
  env: Record<string, string | undefined> = process.env,
  decisionsFile?: string,
): string {
  const v = env.PM_AGG_DECISION_EVENTS_FILE?.trim();
  if (v) return v;
  if (decisionsFile) return join(dirname(decisionsFile), "decision-events.jsonl");
  return join(homedir(), ".local", "state", "pm-aggregator", "decision-events.jsonl");
}

/** Default outbox file: env override, else next to the decisions store. */
export function defaultDecisionOutboxFile(
  env: Record<string, string | undefined> = process.env,
  decisionsFile?: string,
): string {
  const v = env.PM_AGG_DECISION_OUTBOX_FILE?.trim();
  if (v) return v;
  if (decisionsFile) return join(dirname(decisionsFile), "decision-events-outbox.jsonl");
  return join(homedir(), ".local", "state", "pm-aggregator", "decision-events-outbox.jsonl");
}

const PAYLOAD_MAX_BYTES = 16384;

function sanitizePayload(raw: unknown): Record<string, unknown> {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  let out = raw as Record<string, unknown>;
  try {
    if (JSON.stringify(out).length > PAYLOAD_MAX_BYTES) {
      out = { truncated: true, preview: JSON.stringify(out).slice(0, 1024) };
    }
  } catch {
    return { unserializable: true };
  }
  return out;
}

export function makeDecisionEvent(input: DecisionEventInput): DecisionEvent {
  return {
    eventId: randomUUID(),
    setId: input.setId,
    ...(input.questionRef ? { questionRef: input.questionRef } : {}),
    type: input.type,
    payload: sanitizePayload(input.payload ?? {}),
    ts: input.ts ?? new Date().toISOString(),
  };
}

// Appends are serialized per file pair so concurrent request handlers
// can never interleave lines or split an event from its outbox twin.
const appendChains = new Map<string, Promise<unknown>>();
function serializeAppend<T>(key: string, fn: () => Promise<T>): Promise<T> {
  const prev = appendChains.get(key) ?? Promise.resolve();
  const run = prev.then(fn, fn);
  appendChains.set(
    key,
    run.catch(() => {}),
  );
  return run;
}

/**
 * Append events to the log AND the outbox (one outbox record per
 * event, same order). Returns the stored events (with eventIds/ts).
 * A failure throws — callers on request paths catch and log, because
 * an event failure must never fail the interaction it records.
 */
export async function appendDecisionEvents(
  eventsFile: string,
  outboxFile: string,
  inputs: DecisionEventInput[],
): Promise<DecisionEvent[]> {
  if (inputs.length === 0) return [];
  const events = inputs.map(makeDecisionEvent);
  const key = `${eventsFile}\n${outboxFile}`;
  return serializeAppend(key, async () => {
    await mkdir(dirname(eventsFile), { recursive: true });
    if (dirname(outboxFile) !== dirname(eventsFile)) await mkdir(dirname(outboxFile), { recursive: true });
    const enqueuedAt = new Date().toISOString();
    const logLines = events.map((e) => JSON.stringify(e)).join("\n") + "\n";
    const outboxLines =
      events
        .map((e) =>
          JSON.stringify({
            ...e,
            outbox: { status: "pending", enqueuedAt, sink: "turso-cloudflare" },
          } satisfies DecisionEventOutboxRecord),
        )
        .join("\n") + "\n";
    await appendFile(eventsFile, logLines, "utf8");
    await appendFile(outboxFile, outboxLines, "utf8");
    return events;
  });
}

export async function appendDecisionEvent(
  eventsFile: string,
  outboxFile: string,
  input: DecisionEventInput,
): Promise<DecisionEvent> {
  const [event] = await appendDecisionEvents(eventsFile, outboxFile, [input]);
  return event!;
}

function parseEventLine(line: string): DecisionEvent | null {
  try {
    const v: unknown = JSON.parse(line);
    if (!v || typeof v !== "object" || Array.isArray(v)) return null;
    const e = v as Record<string, unknown>;
    if (
      typeof e.eventId !== "string" ||
      typeof e.setId !== "string" ||
      typeof e.type !== "string" ||
      typeof e.ts !== "string"
    )
      return null;
    return v as DecisionEvent;
  } catch {
    return null; // a torn final line never poisons the read
  }
}

/** Every event in the log (all sets), in append order. */
export async function readAllDecisionEvents(eventsFile: string): Promise<DecisionEvent[]> {
  let text: string;
  try {
    text = await readFile(eventsFile, "utf8");
  } catch {
    return [];
  }
  const out: DecisionEvent[] = [];
  for (const line of text.split("\n")) {
    if (!line.trim()) continue;
    const e = parseEventLine(line);
    if (e) out.push(e);
  }
  return out;
}

export interface DecisionEventsPage {
  events: DecisionEvent[];
  /** Pass back as `since` to fetch only newer events. */
  nextCursor: string | null;
  count: number;
}

/**
 * One set`s events in append order, paged by cursor. `since` is the
 * previous page`s nextCursor (an eventId): only events after it are
 * returned. An ISO-timestamp `since` returns events with a later ts.
 * An unknown cursor falls back to the full set history (a poller that
 * lost its cursor re-syncs rather than silently missing events).
 */
export async function readDecisionEvents(
  eventsFile: string,
  setId: string,
  opts: { since?: string; limit?: number } = {},
): Promise<DecisionEventsPage> {
  const all = (await readAllDecisionEvents(eventsFile)).filter((e) => e.setId === setId);
  let start = 0;
  if (opts.since) {
    const idx = all.findIndex((e) => e.eventId === opts.since);
    if (idx >= 0) {
      start = idx + 1;
    } else if (Number.isFinite(Date.parse(opts.since))) {
      const sinceMs = Date.parse(opts.since);
      const firstNewer = all.findIndex((e) => Date.parse(e.ts) > sinceMs);
      start = firstNewer >= 0 ? firstNewer : all.length;
    }
  }
  const limit = opts.limit && opts.limit > 0 ? Math.min(opts.limit, 1000) : 1000;
  const events = all.slice(start, start + limit);
  return {
    events,
    nextCursor: events.length ? events[events.length - 1]!.eventId : (opts.since ?? null),
    count: events.length,
  };
}

/** Validate one client-reported event (POST /api/decisions/<id>/events). */
export function validateDecisionEventInput(
  value: unknown,
):
  | {
      ok: true;
      value: { type: DecisionEventType; questionRef?: DecisionEventQuestionRef; payload: Record<string, unknown> };
    }
  | { ok: false; error: string } {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { ok: false, error: "event must be an object" };
  }
  const b = value as Record<string, unknown>;
  if (!isDecisionEventType(b.type)) {
    return { ok: false, error: `event type must be one of ${DECISION_EVENT_TYPES.join("|")}` };
  }
  let questionRef: DecisionEventQuestionRef | undefined;
  if (b.questionRef !== undefined) {
    if (!b.questionRef || typeof b.questionRef !== "object" || Array.isArray(b.questionRef)) {
      return { ok: false, error: "event questionRef must be an object" };
    }
    const r = b.questionRef as Record<string, unknown>;
    if (typeof r.ledgerId !== "string" || !r.ledgerId || typeof r.qid !== "string" || !r.qid) {
      return { ok: false, error: "event questionRef requires ledgerId and qid" };
    }
    questionRef = {
      ledgerId: r.ledgerId,
      qid: r.qid,
      ...(typeof r.questionIndex === "number" && Number.isFinite(r.questionIndex)
        ? { questionIndex: r.questionIndex }
        : {}),
    };
  }
  if (b.payload !== undefined && (!b.payload || typeof b.payload !== "object" || Array.isArray(b.payload))) {
    return { ok: false, error: "event payload must be an object" };
  }
  return {
    ok: true,
    value: { type: b.type, ...(questionRef ? { questionRef } : {}), payload: sanitizePayload(b.payload ?? {}) },
  };
}
