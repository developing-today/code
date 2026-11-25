// The pm-notify relay's pure core (#8569 Phase 2, design record
// doc/2026-11-17_design_pm8569_notifications_phase2.md): payload
// validation, the subscription-snapshot seq rules, and the
// dead-man's-switch episode state machine. Everything here is a
// pure function of (state, input, now) so the oracle tests in
// test/relay.test.ts exercise the exact transitions the Durable
// Object performs; src/index.ts is only storage + scheduling
// around this module.

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

export interface MirroredQuestion {
  ledgerId: string;
  question: string;
  context?: string;
  options: Array<{ id: string; label: string }>;
  links: string[];
  raisedAt: string;
  answerTokenHash: string;
  acceptedAnswerId?: string;
}

export interface PendingAnswer {
  answerId: string;
  ledgerId: string;
  optionId?: string;
  freeText?: string;
  answeredAt: string;
  answerTokenHash: string;
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
  questions: MirroredQuestion[];
  pendingAnswers: PendingAnswer[];
}

export const emptyRelayState = (): RelayState => ({
  snapshot: null,
  heartbeat: { stackEpoch: null, lastSeq: null, lastBeatAt: null, silentSince: null },
  origin: null,
  lastDelivery: null,
  mirrorEvents: [],
  questions: [],
  pendingAnswers: [],
});

export function validateMirroredQuestions(body: unknown): Validation<MirroredQuestion[]> {
  if (!isRecord(body) || !Array.isArray(body.questions) || body.questions.length > 256) {
    return { ok: false, error: "questions must be an array of at most 256 entries" };
  }
  const questions: MirroredQuestion[] = [];
  const ids = new Set<string>();
  for (const [index, raw] of body.questions.entries()) {
    if (!isRecord(raw)) return { ok: false, error: `questions[${index}] must be an object` };
    const { ledgerId, question, context, options, links, raisedAt, answerTokenHash } = raw;
    if (typeof ledgerId !== "string" || !ledgerId || ledgerId.length > 256 || ids.has(ledgerId)) {
      return { ok: false, error: `questions[${index}].ledgerId must be unique and non-empty` };
    }
    if (typeof question !== "string" || !question || question.length > 4000) {
      return { ok: false, error: `questions[${index}].question must be non-empty` };
    }
    if (context !== undefined && (typeof context !== "string" || context.length > 8000)) {
      return { ok: false, error: `questions[${index}].context must be a string` };
    }
    if (!Array.isArray(options) || options.length > 8) {
      return { ok: false, error: `questions[${index}].options must be an array of at most 8 entries` };
    }
    const cleanOptions: Array<{ id: string; label: string }> = [];
    for (const option of options) {
      if (!isRecord(option) || typeof option.id !== "string" || !option.id || option.id.length > 128 ||
        typeof option.label !== "string" || !option.label || option.label.length > 512) {
        return { ok: false, error: `questions[${index}].options must contain valid options` };
      }
      cleanOptions.push({ id: option.id, label: option.label });
    }
    if (!Array.isArray(links) || links.length > 32 || links.some((link) => typeof link !== "string")) {
      return { ok: false, error: `questions[${index}].links must be strings` };
    }
    if (typeof raisedAt !== "string" || !Number.isFinite(Date.parse(raisedAt))) {
      return { ok: false, error: `questions[${index}].raisedAt must be a valid date` };
    }
    if (typeof answerTokenHash !== "string" || !/^[a-f0-9]{64}$/.test(answerTokenHash)) {
      return { ok: false, error: `questions[${index}].answerTokenHash must be a SHA-256 hex digest` };
    }
    ids.add(ledgerId);
    questions.push({ ledgerId, question, ...(typeof context === "string" ? { context } : {}),
      options: cleanOptions,
      links: (links as string[]).slice(), raisedAt, answerTokenHash });
  }
  return { ok: true, value: questions };
}

export function applyMirroredQuestions(state: RelayState, questions: MirroredQuestion[]): RelayState {
  // Snapshot absence revokes intake, never a previously accepted mailbox
  // entry. An expiry/restart between acceptance and drain must not lose it.
  return { ...state, questions: questions.map((question) => {
    const previous = state.questions.find((old) => old.ledgerId === question.ledgerId && old.answerTokenHash === question.answerTokenHash);
    return previous?.acceptedAnswerId ? { ...question, acceptedAnswerId: previous.acceptedAnswerId } : question;
  }) };
}

/** Step 3 drain oracles (pure, so the edge tests exercise the exact
 * transitions the Durable Object performs). Questions older than the
 * Phase-1 open window refuse intake; the mailbox keeps an accepted
 * answer until the aggregator acks it by id. */
export const OPEN_MAX_AGE_MS = 7 * 24 * 60 * 60_000;

export function isQuestionOpen(question: MirroredQuestion, now: number): boolean {
  return Number.isFinite(Date.parse(question.raisedAt)) && now - Date.parse(question.raisedAt) < OPEN_MAX_AGE_MS;
}

export interface EdgeAnswerInput {
  ledgerId: string;
  answerId: string;
  optionId?: string;
  freeText?: string;
  answeredAt: string;
  answerTokenHash: string;
}

export function applyEdgeAnswer(
  state: RelayState,
  input: EdgeAnswerInput,
  now: number,
): { state: RelayState; outcome: "stored" | "duplicate" | "expired" | "unknown" } {
  const item = state.questions.find((question) => question.ledgerId === input.ledgerId);
  if (!item || item.answerTokenHash !== input.answerTokenHash) return { state, outcome: "unknown" };
  if (!isQuestionOpen(item, now)) return { state, outcome: "expired" };
  if (item.acceptedAnswerId) {
    return { state, outcome: "duplicate" };
  }
  const answer: PendingAnswer = {
    answerId: input.answerId,
    ledgerId: input.ledgerId,
    ...(input.optionId !== undefined ? { optionId: input.optionId } : {}),
    ...(input.freeText !== undefined ? { freeText: input.freeText } : {}),
    answeredAt: input.answeredAt,
    answerTokenHash: input.answerTokenHash,
  };
  return {
    state: {
      ...state,
      questions: state.questions.map((question) =>
        question === item ? { ...item, acceptedAnswerId: input.answerId } : question),
      pendingAnswers: [...state.pendingAnswers, answer],
    },
    outcome: "stored",
  };
}

export function applyAnswerAck(state: RelayState, answerId: string): { state: RelayState; acknowledged: boolean } {
  const pendingAnswers = state.pendingAnswers.filter((answer) => answer.answerId !== answerId);
  const acknowledged = pendingAnswers.length !== state.pendingAnswers.length;
  return { state: acknowledged ? { ...state, pendingAnswers } : state, acknowledged };
}

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
