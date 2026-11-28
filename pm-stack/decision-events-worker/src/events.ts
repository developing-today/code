// Pure event-contract helpers for the decision-events Worker (#9335
// Track B). Kept free of Cloudflare runtime types so the contract is
// unit-testable anywhere (bun test); src/index.ts wires these to D1
// and the Durable Object.
//
// The shape is the shared #9335 wire contract — the same one the
// aggregator validates in pm-stack/aggregator/src/decision-events-sync.ts.
// The two implementations are deliberately independent (the Worker is
// a standalone package) and must be kept in step by the tests on both
// sides, not by a shared import.

export interface DecisionSetEvent {
  eventId: string;
  setId: string;
  questionRef: string | null;
  type: string;
  payload: Record<string, unknown>;
  ts: string;
}

const ID_RE = /^\S{1,128}$/;
const TYPE_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$/;
const MAX_PAYLOAD_BYTES = 64 * 1024;
export const MAX_BATCH_EVENTS = 500;

function isIsoTimestamp(v: unknown): v is string {
  if (typeof v !== "string") return false;
  const ms = Date.parse(v);
  if (Number.isNaN(ms)) return false;
  // Reject Date.parse rollovers like 2026-02-30 -> Mar 2 for date-only
  // prefixes; full timestamps with a T are taken as parsed.
  if (!v.includes("T")) return false;
  return true;
}

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
  if (!isIsoTimestamp(b.ts)) return { ok: false, error: "event.ts must be an ISO timestamp string" };
  return { ok: true, value: { eventId, setId, questionRef, type, payload, ts: b.ts } };
}

export interface ParsedBatch {
  events: DecisionSetEvent[];
  errors: { index: number; error: string }[];
}

/** Parse a webhook body: either {events: [...]}, a bare array, or a
 * single event object. Invalid entries are reported per-index and
 * excluded — one bad event never poisons a batch. */
export function parseEventBatch(body: unknown): { ok: true; value: ParsedBatch } | { ok: false; error: string } {
  let raw: unknown[];
  if (Array.isArray(body)) raw = body;
  else if (body && typeof body === "object" && Array.isArray((body as { events?: unknown }).events)) {
    raw = (body as { events: unknown[] }).events;
  } else if (body && typeof body === "object" && "eventId" in body) raw = [body];
  else return { ok: false, error: "body must be {events: [...]}, an array, or a single event" };
  if (raw.length === 0) return { ok: false, error: "batch is empty" };
  if (raw.length > MAX_BATCH_EVENTS) return { ok: false, error: `batch exceeds ${MAX_BATCH_EVENTS} events` };
  const events: DecisionSetEvent[] = [];
  const errors: { index: number; error: string }[] = [];
  const seen = new Set<string>();
  raw.forEach((item, index) => {
    const v = validateDecisionEvent(item);
    if (!v.ok) {
      errors.push({ index, error: v.error });
      return;
    }
    if (seen.has(v.value.eventId)) {
      errors.push({ index, error: `duplicate eventId ${v.value.eventId} in batch` });
      return;
    }
    seen.add(v.value.eventId);
    events.push(v.value);
  });
  return { ok: true, value: { events, errors } };
}

/** Constant-time string comparison for the shared-secret check. */
export function timingSafeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

/** The caller is authorized when either the Authorization Bearer or
 * the X-Webhook-Secret header matches the configured secret. Fails
 * closed when no secret is configured. */
export function isAuthorized(headers: Headers, secret: string | undefined): boolean {
  if (!secret) return false;
  const auth = headers.get("Authorization");
  if (auth?.startsWith("Bearer ") && timingSafeEqual(auth.slice("Bearer ".length), secret)) return true;
  const headerSecret = headers.get("X-Webhook-Secret");
  if (headerSecret && timingSafeEqual(headerSecret, secret)) return true;
  return false;
}

/** Group events by setId, preserving first-seen set order. */
export function groupBySet(events: DecisionSetEvent[]): Map<string, DecisionSetEvent[]> {
  const map = new Map<string, DecisionSetEvent[]>();
  for (const e of events) {
    const list = map.get(e.setId);
    if (list) list.push(e);
    else map.set(e.setId, [e]);
  }
  return map;
}

/** Composite read-back cursor for GET /sets/<setId>/events (#9614).
 *
 * Pagination orders by (ts, event_id), so the cursor must carry BOTH
 * components: a bare-ts cursor (`ts > after`) makes every event after
 * the first page boundary inside a same-ts group unreachable. The
 * wire form is `<ts>|<eventId>` — ts never contains `|`, so splitting
 * at the first `|` is unambiguous even if an eventId contains one.
 * A bare ISO timestamp is still accepted as a legacy cursor and keeps
 * its original meaning (strictly after that ts). */
export interface EventsCursor {
  ts: string;
  eventId: string | null;
}

export function encodeEventsCursor(cursor: { ts: string; eventId: string }): string {
  return `${cursor.ts}|${cursor.eventId}`;
}

export function parseEventsCursor(raw: string): { ok: true; value: EventsCursor } | { ok: false; error: string } {
  const sep = raw.indexOf("|");
  if (sep === -1) {
    if (!isIsoTimestamp(raw)) {
      return { ok: false, error: "after must be an ISO timestamp or a <ts>|<eventId> cursor" };
    }
    return { ok: true, value: { ts: raw, eventId: null } };
  }
  const ts = raw.slice(0, sep);
  const eventId = raw.slice(sep + 1);
  if (!isIsoTimestamp(ts)) {
    return { ok: false, error: "after cursor ts must be an ISO timestamp" };
  }
  if (!ID_RE.test(eventId)) {
    return { ok: false, error: "after cursor eventId is invalid" };
  }
  return { ok: true, value: { ts, eventId } };
}
