// The pm-notify relay's pure core (#8569 Phase 2, design record
// doc/2026-11-17_design_pm8569_notifications_phase2.md): payload
// validation, the subscription-snapshot seq rules, and the
// dead-man's-switch episode state machine. Everything here is a
// pure function of (state, input, now) so the oracle tests in
// test/relay.test.ts exercise the exact transitions the Durable
// Object performs; src/index.ts is only storage + scheduling
// around this module.

import type { AnsweredRecord, PendingAnswer } from "./answers.ts";

/** A heartbeat every ~60s; the alarm sits at last-beat + 150s
 * (~2 missed beats + margin, design §2). The alarm is the only
 * scheduler in the design. */
export const ALARM_AFTER_MS = 150_000;

/** The class the edge's two sends carry, per design §3: the
 * stack.silent episode rides the `incident` preference class
 * (default-on, breakthrough — quiet hours never hold it). */
export const SILENT_CLASS = "incident";
export const SILENT_OBJECT_REF = "stack:silent";

/** One mirrored subscription: the DO's send authority, mirrored in
 * full from the aggregator's alerts store (design §1). Mirrors
 * PushSubscriptionRow's delivery-relevant fields. */
export interface MirroredSubscription {
  endpoint: string;
  p256dh: string;
  auth: string;
  classes: Record<string, boolean>;
  quiet: { start: string; end: string; tz: string; enabled?: boolean };
}

export interface SubscriptionSnapshot {
  snapshotSeq: number;
  subscriptions: MirroredSubscription[];
}

export interface Heartbeat {
  stackEpoch: string;
  seq: number;
  /** The sender's clock, informational only — the DO's own clock
   * is authoritative for lastBeatAt and the alarm. */
  at?: string;
}

/** A push-worthy throat event mirrored for reconciliation. The
 * payload is descriptive only: this Durable Object never delivers it. */
export interface MirrorEvent {
  mirrorId: string;
  objectRef: string;
  transition: string;
  raisedAt: string;
  payload: Record<string, unknown>;
}

export const MIRROR_RETENTION_MS = 30 * 24 * 60 * 60_000;

export interface HeartbeatState {
  stackEpoch: string | null;
  lastSeq: number | null;
  lastBeatAt: number | null;
  /** When the current silent episode began (= the last beat before
   * the silence, so downtime measures the true silent span), or
   * null when no episode is open. */
  silentSince: number | null;
}

export interface DeliveryRecord {
  kind: "silent" | "recovered";
  at: string;
  sent: number;
  gone: number;
  failed: number;
  error?: string;
}

export interface RelayState {
  snapshot: SubscriptionSnapshot | null;
  heartbeat: HeartbeatState;
  /** The Worker origin learned from forwarded laptop requests;
   * the push payloads' /status deep link. */
  origin: string | null;
  lastDelivery: DeliveryRecord | null;
  mirrorEvents: MirrorEvent[];
  /** Answers taken at the edge, waiting for the aggregator's drain
   * (step 3, src/answers.ts). */
  pendingAnswers: PendingAnswer[];
  /** The first answer per question, kept after its drain so a
   * repeat is answered the same way. */
  answered: AnsweredRecord[];
}

export const emptyRelayState = (): RelayState => ({
  snapshot: null,
  heartbeat: { stackEpoch: null, lastSeq: null, lastBeatAt: null, silentSince: null },
  origin: null,
  lastDelivery: null,
  mirrorEvents: [],
  pendingAnswers: [],
  answered: [],
});

/** Keep the 30-day mirror log bounded by event time (the laptop's
 * transition time), so retries cannot extend an old event's life. */
export function pruneMirrorEvents(events: MirrorEvent[], now: number): MirrorEvent[] {
  const cutoff = now - MIRROR_RETENTION_MS;
  return events.filter((event) => Date.parse(event.raisedAt) >= cutoff);
}

export function applyMirrorEvent(
  state: RelayState,
  event: MirrorEvent,
  now: number,
): { state: RelayState; outcome: "stored" | "duplicate" | "expired" } {
  const retained = pruneMirrorEvents(state.mirrorEvents, now);
  if (Date.parse(event.raisedAt) < now - MIRROR_RETENTION_MS) {
    return { state: { ...state, mirrorEvents: retained }, outcome: "expired" };
  }
  if (retained.some((existing) => existing.mirrorId === event.mirrorId)) {
    return { state: { ...state, mirrorEvents: retained }, outcome: "duplicate" };
  }
  return { state: { ...state, mirrorEvents: [...retained, event] }, outcome: "stored" };
}

// ---------------------------------------------------------------- auth

/** Constant-time string compare over the encoded bytes (same shape
 * as decision-events-worker's timingSafeEqual). */
export function timingSafeEqual(a: string, b: string): boolean {
  const enc = new TextEncoder();
  const ab = enc.encode(a);
  const bb = enc.encode(b);
  if (ab.length !== bb.length) return false;
  let diff = 0;
  for (let i = 0; i < ab.length; i++) diff |= ab[i] ^ bb[i];
  return diff === 0;
}

/** Shared-secret auth for the laptop -> edge POSTs: Bearer token or
 * the X-Relay-Secret header, mirroring the decision-events
 * worker's isAuthorized. No secret configured => nothing is
 * authorized (fail closed). */
export function isAuthorized(headers: Headers, secret: string | undefined): boolean {
  if (!secret) return false;
  const auth = headers.get("Authorization");
  if (auth?.startsWith("Bearer ") && timingSafeEqual(auth.slice("Bearer ".length), secret)) return true;
  const headerSecret = headers.get("X-Relay-Secret");
  if (headerSecret && timingSafeEqual(headerSecret, secret)) return true;
  return false;
}

// ---------------------------------------------------------- validation

const MAX_SUBSCRIPTIONS = 32;
const MAX_CLASS_KEYS = 16;

export type Validation<T> = { ok: true; value: T } | { ok: false; error: string };

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

function isBase64UrlOfLength(text: string, bytes: number): boolean {
  if (!/^[A-Za-z0-9_-]+$/.test(text)) return false;
  try {
    const padded = text.replaceAll("-", "+").replaceAll("_", "/") + "=".repeat((4 - (text.length % 4)) % 4);
    return atob(padded).length === bytes;
  } catch {
    return false;
  }
}

function validateSubscription(raw: unknown, index: number): Validation<MirroredSubscription> {
  if (!isRecord(raw)) return { ok: false, error: `subscriptions[${index}] must be an object` };
  const { endpoint, p256dh, auth, classes, quiet } = raw;
  if (typeof endpoint !== "string" || endpoint.length > 1024) {
    return { ok: false, error: `subscriptions[${index}].endpoint must be a string of at most 1024 chars` };
  }
  let url: URL;
  try {
    url = new URL(endpoint);
  } catch {
    return { ok: false, error: `subscriptions[${index}].endpoint is not a URL` };
  }
  if (url.protocol !== "https:" || url.username || url.password) {
    return { ok: false, error: `subscriptions[${index}].endpoint must be an https URL without credentials` };
  }
  // An uncompressed P-256 point is 65 bytes; the auth secret is 16.
  if (typeof p256dh !== "string" || !isBase64UrlOfLength(p256dh, 65)) {
    return { ok: false, error: `subscriptions[${index}].p256dh must be a base64url 65-byte point` };
  }
  if (typeof auth !== "string" || !isBase64UrlOfLength(auth, 16)) {
    return { ok: false, error: `subscriptions[${index}].auth must be a base64url 16-byte secret` };
  }
  if (!isRecord(classes)) return { ok: false, error: `subscriptions[${index}].classes must be an object` };
  const classKeys = Object.keys(classes);
  if (classKeys.length > MAX_CLASS_KEYS || classKeys.some((k) => typeof classes[k] !== "boolean")) {
    return { ok: false, error: `subscriptions[${index}].classes must map at most ${MAX_CLASS_KEYS} keys to booleans` };
  }
  if (!isRecord(quiet)) return { ok: false, error: `subscriptions[${index}].quiet must be an object` };
  const { start, end, tz, enabled } = quiet;
  if (typeof start !== "string" || typeof end !== "string" || typeof tz !== "string") {
    return { ok: false, error: `subscriptions[${index}].quiet needs string start/end/tz` };
  }
  if (enabled !== undefined && typeof enabled !== "boolean") {
    return { ok: false, error: `subscriptions[${index}].quiet.enabled must be a boolean when present` };
  }
  return {
    ok: true,
    value: {
      endpoint: url.href,
      p256dh,
      auth,
      classes: { ...classes } as Record<string, boolean>,
      quiet: { start, end, tz, ...(enabled !== undefined ? { enabled } : {}) },
    },
  };
}

/** Validate a POST /mirror/subscriptions body (design §1: a
 * full-snapshot replace under a monotonically increasing seq). */
export function validateSnapshot(body: unknown): Validation<SubscriptionSnapshot> {
  if (!isRecord(body)) return { ok: false, error: "body must be an object" };
  const { snapshotSeq, subscriptions } = body;
  if (typeof snapshotSeq !== "number" || !Number.isSafeInteger(snapshotSeq) || snapshotSeq < 1) {
    return { ok: false, error: "snapshotSeq must be a positive integer" };
  }
  if (!Array.isArray(subscriptions)) return { ok: false, error: "subscriptions must be an array" };
  if (subscriptions.length > MAX_SUBSCRIPTIONS) {
    return { ok: false, error: `subscriptions is capped at ${MAX_SUBSCRIPTIONS} entries` };
  }
  const validated: MirroredSubscription[] = [];
  for (let i = 0; i < subscriptions.length; i++) {
    const one = validateSubscription(subscriptions[i], i);
    if (!one.ok) return one;
    validated.push(one.value);
  }
  return { ok: true, value: { snapshotSeq, subscriptions: validated } };
}

/** Validate a POST /heartbeat body (design §1). */
export function validateHeartbeat(body: unknown): Validation<Heartbeat> {
  if (!isRecord(body)) return { ok: false, error: "body must be an object" };
  const { stackEpoch, seq, at } = body;
  if (typeof stackEpoch !== "string" || stackEpoch.length === 0 || stackEpoch.length > 64) {
    return { ok: false, error: "stackEpoch must be a non-empty string of at most 64 chars" };
  }
  if (typeof seq !== "number" || !Number.isSafeInteger(seq) || seq < 0) {
    return { ok: false, error: "seq must be a non-negative integer" };
  }
  if (at !== undefined && typeof at !== "string") return { ok: false, error: "at must be a string when present" };
  return { ok: true, value: { stackEpoch, seq, ...(at !== undefined ? { at } : {}) } };
}

/** Validate the authenticated /mirror/event append shape. */
export function validateMirrorEvent(body: unknown): Validation<MirrorEvent> {
  if (!isRecord(body)) return { ok: false, error: "body must be an object" };
  const { mirrorId, objectRef, transition, raisedAt, payload } = body;
  if (typeof mirrorId !== "string" || mirrorId.length === 0 || mirrorId.length > 160) {
    return { ok: false, error: "mirrorId must be a non-empty string of at most 160 chars" };
  }
  if (typeof objectRef !== "string" || objectRef.length === 0 || objectRef.length > 512) {
    return { ok: false, error: "objectRef must be a non-empty string of at most 512 chars" };
  }
  if (typeof transition !== "string" || transition.length === 0 || transition.length > 64) {
    return { ok: false, error: "transition must be a non-empty string of at most 64 chars" };
  }
  if (typeof raisedAt !== "string" || !Number.isFinite(Date.parse(raisedAt))) {
    return { ok: false, error: "raisedAt must be a valid date string" };
  }
  if (!isRecord(payload)) return { ok: false, error: "payload must be an object" };
  return { ok: true, value: { mirrorId, objectRef, transition, raisedAt, payload } };
}

// -------------------------------------------------------- state machine

export type SnapshotOutcome = "stored" | "duplicate" | "regression";

/** Apply a validated snapshot under the design §1 seq rules: the
 * DO keeps only the highest seq; an equal seq is an idempotent
 * retry of the same snapshot (ack it, change nothing); a lower
 * seq is a regression, rejected loudly by the caller. */
export function applySnapshot(
  state: RelayState,
  snapshot: SubscriptionSnapshot,
): { state: RelayState; outcome: SnapshotOutcome } {
  const current = state.snapshot?.snapshotSeq ?? 0;
  if (snapshot.snapshotSeq < current) return { state, outcome: "regression" };
  if (snapshot.snapshotSeq === current) return { state, outcome: "duplicate" };
  return { state: { ...state, snapshot }, outcome: "stored" };
}

export interface HeartbeatOutcome {
  state: RelayState;
  /** Set when this beat closes a silent episode: the caller owes
   * exactly one recovered push naming this downtime. */
  recovered: { downtimeMs: number; epochChanged: boolean } | null;
  /** The alarm instant the caller must set (design §2). */
  alarmAt: number;
}

/** Apply one heartbeat. Episode close: the design names an epoch
 * change as the recovery signal ("a heartbeat with the same epoch
 * after a silent episode cannot occur — a live process keeps
 * beating"). A suspended laptop is the real-world exception: it
 * stops beating without dying and resumes with the same epoch.
 * Any first beat after silence therefore closes the episode —
 * the design's invariant (exactly one down push, exactly one
 * recovered push per episode) holds under both signals, and a
 * same-epoch resume can never strand an episode open. Recorded
 * as a deliberate widening in the package README. */
export function applyHeartbeat(state: RelayState, beat: Heartbeat, now: number): HeartbeatOutcome {
  const hb = state.heartbeat;
  let recovered: HeartbeatOutcome["recovered"] = null;
  let silentSince = hb.silentSince;
  if (silentSince !== null) {
    recovered = { downtimeMs: Math.max(0, now - silentSince), epochChanged: beat.stackEpoch !== hb.stackEpoch };
    silentSince = null;
  }
  return {
    state: {
      ...state,
      heartbeat: { stackEpoch: beat.stackEpoch, lastSeq: beat.seq, lastBeatAt: now, silentSince },
    },
    recovered,
    alarmAt: now + ALARM_AFTER_MS,
  };
}

export interface AlarmOutcome {
  state: RelayState;
  /** True when this alarm fire opens a new silent episode: the
   * caller owes exactly one stack.silent push. */
  opened: boolean;
}

/** Apply an alarm fire. Opens an episode only when beats have
 * actually stopped for the full window — an alarm that races a
 * fresh beat, or fires with no beat ever seen, opens nothing. */
export function applyAlarm(state: RelayState, now: number): AlarmOutcome {
  const hb = state.heartbeat;
  if (hb.silentSince !== null || hb.lastBeatAt === null) return { state, opened: false };
  if (now < hb.lastBeatAt + ALARM_AFTER_MS) return { state, opened: false };
  return {
    state: { ...state, heartbeat: { ...hb, silentSince: hb.lastBeatAt } },
    opened: true,
  };
}

/** Whether a mirrored subscription takes the edge's sends: the
 * incident class is enabled unless explicitly toggled off (the
 * aggregator's mergeClasses default), and incident breakthrough
 * means quiet hours never hold it — so quiet is not consulted. */
export function takesSilentClass(sub: MirroredSubscription): boolean {
  return sub.classes[SILENT_CLASS] !== false;
}

// ------------------------------------------------------------ payloads

export function formatDuration(ms: number): string {
  const totalSeconds = Math.round(ms / 1000);
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest > 0 ? `${hours}h ${rest}m` : `${hours}h`;
}

const statusUrl = (origin: string | null): string => (origin ? `${origin}/status` : "/status");

/** The stack.silent push payload, in the aggregator's PushPayload
 * shape (pm-stack/aggregator/src/alerts.ts) so the phone's service
 * worker renders it exactly like a laptop-sent incident. */
export function buildSilentPayload(state: RelayState) {
  const since = state.heartbeat.silentSince;
  return {
    class: SILENT_CLASS,
    objectRef: SILENT_OBJECT_REF,
    url: statusUrl(state.origin),
    title: "PM stack is down",
    body: since
      ? `No heartbeat since ${new Date(since).toISOString()}. The laptop stack is unreachable — the board and answers are paused until it returns.`
      : "No heartbeat from the laptop stack. It is unreachable — the board and answers are paused until it returns.",
  };
}

/** The recovered push payload: exactly one per episode, naming
 * the downtime (design §3). */
export function buildRecoveredPayload(state: RelayState, downtimeMs: number) {
  return {
    class: SILENT_CLASS,
    objectRef: SILENT_OBJECT_REF,
    url: statusUrl(state.origin),
    title: "PM stack recovered",
    body: `The laptop stack is back after ${formatDuration(downtimeMs)} of silence.`,
  };
}
