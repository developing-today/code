// The alert throat (#3676; design #3450 as amended by Drew
// 2026-11-01): every sentinel posts incidents to ONE intake; this
// module owns dedupe, delivery, escalation, and ack state; every
// transition lands in the Supabase muse-dev2 `alerts` ledger
// (incidents / incident_events / push_subscriptions /
// notification_sends). Delivery is Web Push per the #1254 design
// (doc/2026-11-01T03-54-00Z_design_pwa_push_notifications.md): the
// `incident` class is the breakthrough class — it delivers
// immediately, quiet hours or not, because the sentinels are
// external to the thing that might be down. Warnings never push in
// v1 (page + ledger only).
//
// The throat's truth contract (#3702 Phase 3, 2026-11-03): the alert
// throat's records and responses tell the truth and keep its secrets.
// A state transition is recorded only when the event it names happened
// (delivered / escalated require at least one accepted send; `gone` is
// not a send). Subscription key material (`p256dh`, `auth`) never
// leaves the server: store rows are internal, any HTTP surface
// projects — the page's projection is the only read shape. Subscription
// preferences are validated as values, not shapes, and an invalid value
// falls back per-field to the default — never stored, never
// reinterpreted.
import type { Config } from "./config.ts";
import { sendPush, type PushOutcome, type PushTarget } from "./push.ts";
import { getSupabaseClient, sqlInt, sqlJson, sqlStr, type SupabaseClient } from "./supabase.ts";
import {
  buildNotificationCandidates,
  decideNotification,
  nextNotificationDigestAt,
  NOTIFICATION_CLASS_DEFAULTS,
  type NotificationHistoryEntry,
  type NotificationSnapshot,
} from "./notifications.ts";
import { JsonlNotificationHistory, type NotificationHistoryStore } from "./notification-history.ts";

// --- Shapes ----------------------------------------------------------------

export type Severity = "critical" | "warning" | "info";
export type IncidentStatus = "open" | "acked" | "closed";

export interface Incident {
  id: string;
  source: string;
  incident_key: string;
  severity: Severity;
  title: string;
  body: string | null;
  status: IncidentStatus;
  first_seen_at: string;
  last_seen_at: string;
  delivered_at: string | null;
  escalated_at: string | null;
  acked_at: string | null;
  closed_at: string | null;
  occurrences: number;
}

export interface IncidentEvent {
  id: number;
  incident_id: string;
  at: string;
  kind: "opened" | "duplicate_suppressed" | "delivered" | "escalated" | "acked" | "closed" | "reopened";
  detail: Record<string, unknown> | null;
  source?: string;
  incident_key?: string;
  title?: string;
}

export interface PushSubscriptionRow {
  id: string;
  endpoint: string;
  p256dh: string;
  auth: string;
  classes: Record<string, boolean>;
  quiet: QuietPrefs;
  user_agent: string | null;
  created_at: string;
  last_seen_at: string;
  gone_count: number;
}

/** One alerts.notification_sends ledger row (#3319): one per send
 * attempt, written by logSend; acted_on_at is stamped by the
 * exactly-once Ack transition. The insert names no timestamp column,
 * so the send time is the row's defaulted created_at; sent_at is
 * read first when a row carries one. */
export interface NotificationSendRow {
  class: string;
  object_ref: string;
  incident_id: string | null;
  outcome: string;
  sent_at?: string | null;
  created_at?: string | null;
  acted_on_at: string | null;
}

export interface QuietPrefs {
  start: string;
  end: string;
  tz: string;
  /** Drew 2026-11-01: quiet hours can be turned OFF entirely — no
   * time-of-day limit at all. Absent (older rows) reads as on. */
  enabled?: boolean;
}

/** #3693: the public shape of a push subscription — everything the
 * /alerts page already renders, minus the Web Push key material.
 * p256dh + auth (RFC 8291 push-encryption secrets) never leave the
 * server; delivery reads the full rows via the store instead. */
export interface PublicPushSubscription {
  id: string;
  endpoint: string;
  classes: Record<string, boolean>;
  quiet: QuietPrefs;
  user_agent: string | null;
  created_at: string;
  last_seen_at: string;
  gone_count: number;
}

/** #3693: strip the push-encryption secrets before serializing a
 * subscription to an HTTP response. */
export function toPublicSubscription(sub: PushSubscriptionRow): PublicPushSubscription {
  return {
    id: sub.id,
    endpoint: sub.endpoint,
    classes: sub.classes,
    quiet: sub.quiet,
    user_agent: sub.user_agent,
    created_at: sub.created_at,
    last_seen_at: sub.last_seen_at,
    gone_count: sub.gone_count,
  };
}

// --- Pure helpers (unit-tested) ---------------------------------------------

/** Design §1: the incident class defaults ON for new subscriptions. */
export const DEFAULT_CLASSES: Record<string, boolean> = NOTIFICATION_CLASS_DEFAULTS;
/** Design §2: default quiet window 23:00–07:00 America/Chicago. */
export const DEFAULT_QUIET: QuietPrefs = { start: "23:00", end: "07:00", tz: "America/Chicago", enabled: true };

export function mergeClasses(input: unknown): Record<string, boolean> {
  const out = { ...DEFAULT_CLASSES };
  if (input && typeof input === "object") {
    for (const [k, v] of Object.entries(input as Record<string, unknown>)) {
      if (typeof v === "boolean") out[k] = v;
    }
  }
  return out;
}

/** #3686: clock-shape plus numeric range — hours 00–23, minutes 00–59.
 * Shape-only checks accept impossible values (99:99, 24:60) that
 * inQuietHours then reinterprets as a different window. */
function isClockTime(s: string): boolean {
  const m = /^(\d{2}):(\d{2})$/.exec(s);
  if (!m) return false;
  const hh = Number(m[1]);
  const mm = Number(m[2]);
  return hh >= 0 && hh <= 23 && mm >= 0 && mm <= 59;
}

export function mergeQuiet(input: unknown): QuietPrefs {
  const out = { ...DEFAULT_QUIET };
  if (input && typeof input === "object") {
    const q = input as Record<string, unknown>;
    if (typeof q.start === "string" && isClockTime(q.start)) out.start = q.start;
    if (typeof q.end === "string" && isClockTime(q.end)) out.end = q.end;
    if (typeof q.tz === "string" && q.tz.trim()) {
      const tz = q.tz.trim();
      try {
        new Intl.DateTimeFormat("en-US", { timeZone: tz });
        out.tz = tz;
      } catch {
        // Keep the default timezone for an invalid stored preference.
      }
    }
    if (typeof q.enabled === "boolean") out.enabled = q.enabled;
  }
  return out;
}

/** Preserve existing subscription choices when new classes are added:
 * only the original incident class defaults on for old rows; new rows
 * receive the reviewed defaults from mergeClasses at creation. */
export function classEnabled(sub: { classes: Record<string, boolean> }, cls: string): boolean {
  return sub.classes?.[cls] ?? (cls === "incident");
}

/** Is `now` inside the quiet window (evaluated in the prefs' timezone)?
 * The incident class never consults this (breakthrough class, #3676);
 * it exists for the dashboard classes of #3317. */
// ⚡ Bolt: inQuietHours is evaluated per subscription on every digest /
// alert pass, and Intl.DateTimeFormat construction dominates it (~29µs).
// The formatter depends only on the timezone, so cache one per tz; the
// map is bounded by the set of tz names users actually store (a handful).
const QUIET_HOURS_FORMATTERS = new Map<string, Intl.DateTimeFormat>();
function quietHoursFormatter(tz: string): Intl.DateTimeFormat {
  let fmt = QUIET_HOURS_FORMATTERS.get(tz);
  if (!fmt) {
    fmt = new Intl.DateTimeFormat("en-US", {
      timeZone: tz,
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    });
    QUIET_HOURS_FORMATTERS.set(tz, fmt);
  }
  return fmt;
}

export function inQuietHours(quiet: QuietPrefs, now: Date): boolean {
  if (quiet.enabled === false) return false; // quiet hours off — no time-of-day limit
  let mins: number;
  try {
    const parts = quietHoursFormatter(quiet.tz).formatToParts(now);
    const h = Number(parts.find((p) => p.type === "hour")?.value ?? "0");
    const m = Number(parts.find((p) => p.type === "minute")?.value ?? "0");
    mins = h * 60 + m;
  } catch {
    return false; // unknown tz — do not suppress
  }
  const toMin = (t: string) => {
    const [h, m] = t.split(":").map(Number);
    return (h || 0) * 60 + (m || 0);
  };
  const start = toMin(quiet.start);
  const end = toMin(quiet.end);
  return start <= end ? mins >= start && mins < end : mins >= start || mins < end;
}

export const incidentObjectRef = (id: string): string => `incident:${id}`;

/** Deep link (design §3): the incident's own anchored card on /alerts. */
export const incidentUrl = (origin: string, id: string): string => `${origin}/alerts#incident-${id}`;

export interface PushPayload {
  class: string;
  objectRef: string;
  url: string;
  title: string;
  body: string;
  escalation?: boolean;
  items?: Array<{ class: string; objectRef: string; title: string; url: string }>;
}

export function buildIncidentPayload(incident: Incident, origin: string, escalation: boolean): PushPayload {
  const base = incident.body?.trim() ? `${incident.title} — ${incident.body.trim()}` : incident.title;
  return {
    class: "incident",
    objectRef: incidentObjectRef(incident.id),
    url: incidentUrl(origin, incident.id),
    title: escalation ? `Escalation: ${incident.title}` : incident.title,
    body: escalation
      ? `Still unacknowledged: ${base} (source: ${incident.source})`
      : `${base} (source: ${incident.source})`,
    ...(escalation ? { escalation: true } : {}),
  };
}

/**
 * A repeated post writes a `duplicate_suppressed` event only when a
 * delivery would otherwise repeat — i.e. the open incident was
 * already delivered once, so a naive engine would push again.
 */
export function duplicateWouldRepeatDelivery(incident: Incident): boolean {
  return incident.delivered_at !== null && incident.status !== "closed";
}

const SEVERITIES: ReadonlySet<string> = new Set(["critical", "warning", "info"]);

export interface NewIncident {
  source: string;
  key: string;
  severity: Severity;
  title: string;
  body?: string;
}

export function validateNewIncident(body: unknown): { ok: true; value: NewIncident } | { ok: false; error: string } {
  if (!body || typeof body !== "object") return { ok: false, error: "JSON object body required" };
  const b = body as Record<string, unknown>;
  const source = typeof b.source === "string" ? b.source.trim() : "";
  const key = typeof b.key === "string" ? b.key.trim() : "";
  const title = typeof b.title === "string" ? b.title.trim() : "";
  const severity = typeof b.severity === "string" ? b.severity : "";
  if (!source) return { ok: false, error: "source required" };
  if (!key) return { ok: false, error: "key required" };
  if (!title) return { ok: false, error: "title required" };
  if (!SEVERITIES.has(severity)) return { ok: false, error: 'severity must be "critical" | "warning" | "info"' };
  return {
    ok: true,
    value: {
      source,
      key,
      severity: severity as Severity,
      title,
      body: typeof b.body === "string" ? b.body : undefined,
    },
  };
}

export interface NewSubscription {
  endpoint: string;
  p256dh: string;
  auth: string;
  classes: Record<string, boolean>;
  /** The class keys the caller explicitly sent (#7193 Codex): on an
   * upsert of an existing endpoint only these keys change; the stored
   * choices for every other class survive (the dashboard Alerts form
   * submits only classes.incident, and mergeClasses would otherwise
   * silently re-enable every other class from the defaults). */
  providedClasses?: Record<string, boolean>;
  quiet: QuietPrefs;
  userAgent?: string;
}

export function validateNewSubscription(
  body: unknown,
): { ok: true; value: NewSubscription } | { ok: false; error: string } {
  if (!body || typeof body !== "object") return { ok: false, error: "JSON object body required" };
  const b = body as Record<string, unknown>;
  const keys = (b.keys ?? {}) as Record<string, unknown>;
  const endpoint = typeof b.endpoint === "string" ? b.endpoint.trim() : "";
  const p256dh = typeof keys.p256dh === "string" ? keys.p256dh : typeof b.p256dh === "string" ? b.p256dh : "";
  const auth = typeof keys.auth === "string" ? keys.auth : typeof b.auth === "string" ? b.auth : "";
  if (!endpoint || !/^https:\/\//.test(endpoint)) return { ok: false, error: "endpoint must be an https URL" };
  if (!p256dh) return { ok: false, error: "keys.p256dh required" };
  if (!auth) return { ok: false, error: "keys.auth required" };
  const providedClasses: Record<string, boolean> = {};
  if (b.classes && typeof b.classes === "object") {
    for (const [k, v] of Object.entries(b.classes as Record<string, unknown>)) {
      if (typeof v === "boolean") providedClasses[k] = v;
    }
  }
  return {
    ok: true,
    value: {
      endpoint,
      p256dh,
      auth,
      classes: mergeClasses(b.classes),
      providedClasses,
      quiet: mergeQuiet(b.quiet),
      userAgent: typeof b.userAgent === "string" ? b.userAgent : undefined,
    },
  };
}

// --- The store (Supabase `alerts` schema) ------------------------------------

interface PendingIncidentCursor {
  timestamp: string;
  id: string;
}

type PendingIncidentOrder = "first_seen_at" | "delivered_at";

export class AlertsStore {
  // Each queue advances independently, even when a send fails and leaves
  // the incident eligible. The keyset cursor lives for this store's lifetime.
  private readonly deliveryCursor: { last: PendingIncidentCursor | null } = { last: null };
  private readonly escalationCursor: { last: PendingIncidentCursor | null } = { last: null };

  constructor(private readonly db: SupabaseClient) {}

  private async pendingBatch(
    eligibility: string,
    orderColumn: PendingIncidentOrder,
    cursor: { last: PendingIncidentCursor | null },
    limit: number,
  ): Promise<Incident[]> {
    const previous = cursor.last;
    const page = (wrap: boolean, pageLimit: number) => {
      const cursorFilter = previous
        ? `and (${orderColumn}, id) ${wrap ? "<=" : ">"} (${sqlStr(previous.timestamp)}, ${sqlStr(previous.id)})`
        : "";
      return this.db.query<Incident>(
        `select * from alerts.incidents
         where ${eligibility} ${cursorFilter}
         order by ${orderColumn}, id limit ${sqlInt(pageLimit)}`,
      );
    };

    const rows = await page(false, limit);
    let advanceTo = rows.at(-1);
    if (previous && rows.length < limit) {
      const wrapped = await page(true, limit - rows.length);
      rows.push(...wrapped);
      if (wrapped.length) advanceTo = wrapped.at(-1);
    }

    if (advanceTo) {
      const timestamp = advanceTo[orderColumn];
      if (timestamp === null) throw new Error(`pending incident has no ${orderColumn}`);
      cursor.last = { timestamp, id: advanceTo.id };
    }
    return rows;
  }

  private async addEvent(incidentId: string, kind: IncidentEvent["kind"], detail?: Record<string, unknown>) {
    await this.db.query(
      `insert into alerts.incident_events (incident_id, kind, detail) values (${sqlStr(incidentId)}, ${sqlStr(kind)}, ${sqlJson(detail ?? null)})`,
    );
  }

  async upsertSubscription(sub: NewSubscription): Promise<PushSubscriptionRow> {
    const rows = await this.db.query<PushSubscriptionRow>(
      `insert into alerts.push_subscriptions (endpoint, p256dh, auth, classes, quiet, user_agent)
       values (${sqlStr(sub.endpoint)}, ${sqlStr(sub.p256dh)}, ${sqlStr(sub.auth)}, ${sqlJson(sub.classes)}, ${sqlJson(sub.quiet)}, ${
         sub.userAgent ? sqlStr(sub.userAgent) : "null"
       })
       on conflict (endpoint) do update set
         p256dh = excluded.p256dh, auth = excluded.auth,
         classes = push_subscriptions.classes || ${sqlJson(sub.providedClasses ?? {})},
         quiet = excluded.quiet, user_agent = excluded.user_agent,
         last_seen_at = now(), gone_count = 0
       returning *`,
    );
    return rows[0];
  }

  async deleteSubscription(endpoint: string): Promise<boolean> {
    const rows = await this.db.query<{ id: string }>(
      `delete from alerts.push_subscriptions where endpoint = ${sqlStr(endpoint)} returning id`,
    );
    return rows.length > 0;
  }

  async listSubscriptions(): Promise<PushSubscriptionRow[]> {
    const rows = await this.db.query<PushSubscriptionRow>(
      `select * from alerts.push_subscriptions order by created_at`,
    );
    // #3702 decision 3: old stored prefs are not rewritten, but must not
    // be reinterpreted on read. Apply the same per-field fallbacks used at
    // intake so delivery and the prefs page see safe values.
    return rows.map((row) => ({ ...row, quiet: mergeQuiet(row.quiet) }));
  }

  /** Design §4: a 404/410 marks the subscription gone; two consecutive
   * gone responses prune it. A successful send resets the count (the
   * upsert path resets it too, on re-subscribe). */
  async noteSubscriptionGone(endpoint: string): Promise<"marked" | "pruned"> {
    const rows = await this.db.query<{ gone_count: number }>(
      `update alerts.push_subscriptions set gone_count = gone_count + 1 where endpoint = ${sqlStr(endpoint)} returning gone_count`,
    );
    const count = rows[0]?.gone_count ?? 0;
    if (count >= 2) {
      await this.deleteSubscription(endpoint);
      return "pruned";
    }
    return "marked";
  }

  async noteSubscriptionAlive(endpoint: string): Promise<void> {
    await this.db.query(
      `update alerts.push_subscriptions set gone_count = 0, last_seen_at = now() where endpoint = ${sqlStr(endpoint)} and gone_count <> 0`,
    );
  }

  /**
   * Intake dedupe rides the partial unique index on (source,
   * incident_key) WHERE status <> 'closed': a fresh insert opens the
   * incident (+ `opened` event); a conflict bumps occurrences /
   * last_seen_at (+ `duplicate_suppressed` event only when a delivery
   * would otherwise repeat).
   */
  async postIncident(input: NewIncident): Promise<{ incident: Incident; created: boolean }> {
    const inserted = await this.db.query<Incident>(
      `insert into alerts.incidents (source, incident_key, severity, title, body)
       values (${sqlStr(input.source)}, ${sqlStr(input.key)}, ${sqlStr(input.severity)}, ${sqlStr(input.title)}, ${
         input.body ? sqlStr(input.body) : "null"
       })
       on conflict (source, incident_key) where status <> 'closed' do nothing
       returning *`,
    );
    if (inserted[0]) {
      await this.addEvent(inserted[0].id, "opened", { source: input.source, key: input.key, severity: input.severity });
      return { incident: inserted[0], created: true };
    }
    const bumped = await this.db.query<Incident>(
      `update alerts.incidents set occurrences = occurrences + 1, last_seen_at = now()
       where source = ${sqlStr(input.source)} and incident_key = ${sqlStr(input.key)} and status <> 'closed'
       returning *`,
    );
    const incident = bumped[0];
    if (!incident) throw new Error("incident vanished between insert conflict and dedupe update");
    if (duplicateWouldRepeatDelivery(incident)) {
      await this.addEvent(incident.id, "duplicate_suppressed", { occurrences: incident.occurrences });
    }
    return { incident, created: false };
  }

  async getIncident(id: string): Promise<Incident | null> {
    const rows = await this.db.query<Incident>(`select * from alerts.incidents where id = ${sqlStr(id)}`);
    return rows[0] ?? null;
  }

  async listIncidents(limit = 100): Promise<Incident[]> {
    return this.db.query<Incident>(
      `select * from alerts.incidents order by (status = 'closed'), first_seen_at desc limit ${sqlInt(limit)}`,
    );
  }

  async recentEvents(limit = 100): Promise<IncidentEvent[]> {
    return this.db.query<IncidentEvent>(
      `select e.*, i.source, i.incident_key, i.title
       from alerts.incident_events e join alerts.incidents i on i.id = e.incident_id
       order by e.id desc limit ${sqlInt(limit)}`,
    );
  }

  /** Ack is exactly-once: only the open -> acked transition writes an
   * event; a repeat ack returns the current state unchanged. */
  async ackIncident(id: string): Promise<{ incident: Incident | null; transitioned: boolean }> {
    const rows = await this.db.query<Incident>(
      `update alerts.incidents set status = 'acked', acked_at = now()
       where id = ${sqlStr(id)} and status = 'open' returning *`,
    );
    if (rows[0]) {
      await this.addEvent(id, "acked", {});
      await this.db.query(
        `update alerts.notification_sends set acted_on_at = now() where incident_id = ${sqlStr(id)} and acted_on_at is null`,
      );
      return { incident: rows[0], transitioned: true };
    }
    return { incident: await this.getIncident(id), transitioned: false };
  }

  async closeIncident(id: string): Promise<{ incident: Incident | null; transitioned: boolean }> {
    const rows = await this.db.query<Incident>(
      `update alerts.incidents set status = 'closed', closed_at = now()
       where id = ${sqlStr(id)} and status <> 'closed' returning *`,
    );
    if (rows[0]) {
      await this.addEvent(id, "closed", {});
      return { incident: rows[0], transitioned: true };
    }
    return { incident: await this.getIncident(id), transitioned: false };
  }

  async pendingDeliveries(limit = 20): Promise<Incident[]> {
    return this.pendingBatch(
      "status = 'open' and severity = 'critical' and delivered_at is null",
      "first_seen_at",
      this.deliveryCursor,
      limit,
    );
  }

  async pendingEscalations(ackWindowMin: number, limit = 20): Promise<Incident[]> {
    return this.pendingBatch(
      `status = 'open' and severity = 'critical'
       and delivered_at is not null and escalated_at is null
       and delivered_at < now() - make_interval(mins => ${sqlInt(ackWindowMin)})`,
      "delivered_at",
      this.escalationCursor,
      limit,
    );
  }

  async markDelivered(id: string, detail: Record<string, unknown>): Promise<boolean> {
    const rows = await this.db.query<{ id: string }>(
      `update alerts.incidents set delivered_at = now() where id = ${sqlStr(id)} and delivered_at is null returning id`,
    );
    if (rows.length) await this.addEvent(id, "delivered", detail);
    return rows.length > 0;
  }

  async markEscalated(id: string, detail: Record<string, unknown>): Promise<boolean> {
    const rows = await this.db.query<{ id: string }>(
      `update alerts.incidents set escalated_at = now() where id = ${sqlStr(id)} and escalated_at is null returning id`,
    );
    if (rows.length) await this.addEvent(id, "escalated", detail);
    return rows.length > 0;
  }

  async logSend(entry: {
    class: string;
    objectRef: string;
    incidentId?: string;
    outcome: "sent" | "suppressed-quiet" | "suppressed-dedupe" | "gone" | "error";
  }): Promise<void> {
    await this.db.query(
      `insert into alerts.notification_sends (class, object_ref, incident_id, outcome)
       values (${sqlStr(entry.class)}, ${sqlStr(entry.objectRef)}, ${
         entry.incidentId ? sqlStr(entry.incidentId) : "null"
       }, ${sqlStr(entry.outcome)})`,
    );
  }

  /** Every send-ledger row (#3319 roll-up input). The ledger is one
   * row per attempt, so a full read stays small; the rolling window
   * is applied by the pure roll-up in src/notifications.ts. */
  async listSends(): Promise<NotificationSendRow[]> {
    return this.db.query<NotificationSendRow>(`select * from alerts.notification_sends`);
  }
}

/** The test alert (#3676 follow-up, Drew 2026-11-01): the /alerts
 * page's "Send a test alert" button. Clearly a test in title + body;
 * objectRef is a stable non-incident ref so the sends ledger keeps
 * test traffic distinguishable from incident traffic. */
export const TEST_OBJECT_REF = "test:push";

export function buildTestPayload(origin: string): PushPayload {
  return {
    class: "test",
    objectRef: TEST_OBJECT_REF,
    url: `${origin}/alerts`,
    title: "Test alert — PM Stack",
    body: "This is a test alert from the Alerts page. Push delivery works on this device.",
  };
}

export interface TestPushSummary {
  subscriptions: number;
  sent: number;
  gone: number;
  errors: number;
}

/**
 * Send the test push to every current subscription, recording one
 * notification_sends row (class 'test') per subscription. The sender
 * is injectable so the sends-row behavior is unit-testable without a
 * network; production uses the real Web Push sender.
 */
export async function sendTestPush(
  store: Pick<AlertsStore, "listSubscriptions" | "logSend" | "noteSubscriptionGone" | "noteSubscriptionAlive">,
  origin: string,
  send: typeof sendPush = sendPush,
): Promise<TestPushSummary> {
  const payload = buildTestPayload(origin);
  const subs = await store.listSubscriptions();
  const summary: TestPushSummary = { subscriptions: subs.length, sent: 0, gone: 0, errors: 0 };
  for (const sub of subs) {
    const target: PushTarget = { endpoint: sub.endpoint, p256dh: sub.p256dh, auth: sub.auth };
    const outcome = await send(target, payload);
    if (outcome.ok) {
      summary.sent += 1;
      await store.logSend({ class: "test", objectRef: TEST_OBJECT_REF, outcome: "sent" });
      await store.noteSubscriptionAlive(sub.endpoint);
    } else if (outcome.gone) {
      summary.gone += 1;
      await store.logSend({ class: "test", objectRef: TEST_OBJECT_REF, outcome: "gone" });
      await store.noteSubscriptionGone(sub.endpoint);
    } else {
      summary.errors += 1;
      await store.logSend({ class: "test", objectRef: TEST_OBJECT_REF, outcome: "error" });
    }
  }
  return summary;
}

/**
 * Create the shared store resolver. Keep the in-flight attempt and successful
 * store cached, but clear null/rejected attempts so a later call can recover.
 * Lazy-init contract (success-only memoization): README "Lazy-initialization contract" (#3879).
 */
export function createAlertsStoreGetter(
  getClient: typeof getSupabaseClient = getSupabaseClient,
): () => Promise<AlertsStore | null> {
  let storePromise: Promise<AlertsStore | null> | null = null;

  return function getAlertsStore(): Promise<AlertsStore | null> {
    if (!storePromise) {
      const pending = getClient().then((db) => (db ? new AlertsStore(db) : null));
      storePromise = pending;
      void pending.then(
        (store) => {
          if (store === null && storePromise === pending) storePromise = null;
        },
        () => {
          if (storePromise === pending) storePromise = null;
        },
      );
    }
    return storePromise;
  }
}

/** The shared store; null when the Supabase path is unavailable. */
export const getAlertsStore = createAlertsStoreGetter();

// --- The engine ---------------------------------------------------------------

interface DeliverySummary {
  subscriptions: number;
  sent: number;
  gone: number;
  errors: number;
}

type PushSender = (target: PushTarget, payload: unknown) => Promise<PushOutcome>;

async function deliverToAll(
  store: AlertsStore,
  targets: PushSubscriptionRow[],
  incident: Incident,
  origin: string,
  escalation: boolean,
  push: PushSender,
): Promise<DeliverySummary> {
  const payload = buildIncidentPayload(incident, origin, escalation);
  const summary: DeliverySummary = { subscriptions: targets.length, sent: 0, gone: 0, errors: 0 };
  for (const sub of targets) {
    const target: PushTarget = { endpoint: sub.endpoint, p256dh: sub.p256dh, auth: sub.auth };
    const outcome = await push(target, payload);
    if (outcome.ok) {
      summary.sent += 1;
      await store.logSend({
        class: "incident",
        objectRef: payload.objectRef,
        incidentId: incident.id,
        outcome: "sent",
      });
      await store.noteSubscriptionAlive(sub.endpoint);
    } else if (outcome.gone) {
      summary.gone += 1;
      await store.logSend({
        class: "incident",
        objectRef: payload.objectRef,
        incidentId: incident.id,
        outcome: "gone",
      });
      await store.noteSubscriptionGone(sub.endpoint);
    } else {
      summary.errors += 1;
      await store.logSend({
        class: "incident",
        objectRef: payload.objectRef,
        incidentId: incident.id,
        outcome: "error",
      });
    }
  }
  return summary;
}

/**
 * One engine tick: deliver new open criticals, then escalate unacked
 * criticals past the ack window — exactly once each (delivered_at /
 * escalated_at are the guards, set only after at least one send succeeds).
 * With zero subscriptions there is nothing to deliver to, so the
 * incident stays undelivered (and unescalated) until someone
 * subscribes — a later subscription still gets the push.
 */
export interface AlertTickOptions {
  dashboardSnapshot?: () => Promise<NotificationSnapshot>;
  notificationHistory?: NotificationHistoryStore;
  now?: number;
}

async function runDashboardNotificationTick(
  cfg: Config,
  store: AlertsStore,
  subscriptions: PushSubscriptionRow[],
  push: PushSender,
  options: AlertTickOptions,
): Promise<void> {
  if (!options.dashboardSnapshot || subscriptions.length === 0) return;
  const now = options.now ?? Date.now();
  const historyStore = options.notificationHistory ?? new JsonlNotificationHistory(cfg.notificationsFile);
  const [snapshot, history] = await Promise.all([options.dashboardSnapshot(), historyStore.read()]);
  const candidates = buildNotificationCandidates(snapshot, now);

  const keyOf = (entry: Pick<NotificationHistoryEntry, "class" | "objectRef" | "stateFingerprint" | "subscriptionId">) =>
    `${entry.subscriptionId ?? ""}\u0000${entry.class}\u0000${entry.objectRef}\u0000${entry.stateFingerprint}`;
  const pendingFor = (subId: string): Map<string, NotificationHistoryEntry> => {
    const pending = new Map<string, NotificationHistoryEntry>();
    for (const entry of history) {
      if (entry.subscriptionId !== subId) continue;
      const key = keyOf(entry);
      if (entry.outcome === "queued") pending.set(key, entry);
      else if (entry.outcome === "queue-cancelled" || entry.outcome === "sent" || entry.outcome === "gone") pending.delete(key);
    }
    return pending;
  };
  const append = async (entry: NotificationHistoryEntry) => {
    await historyStore.append(entry);
    history.push(entry);
  };

  for (const sub of subscriptions) {
    const enabledCandidates = candidates.filter((candidate) => classEnabled(sub, candidate.class));
    const currentKeys = new Set(enabledCandidates.map((candidate) => keyOf({ ...candidate, subscriptionId: sub.id })));
    const pending = pendingFor(sub.id);
    const subHistory = history.filter((entry) => entry.subscriptionId === sub.id);

    // A disappeared state or a muted class cancels its pending digest entry.
    for (const [key, queued] of pending) {
      if (currentKeys.has(key)) continue;
      await append({
        at: new Date(now).toISOString(),
        class: queued.class,
        objectRef: queued.objectRef,
        stateFingerprint: queued.stateFingerprint,
        outcome: "queue-cancelled",
        subscriptionId: sub.id,
      });
      pending.delete(key);
    }

    // Recovery markers: a previously recorded candidate state that is no
    // longer among this subscription's current candidates has ended its
    // episode. decideNotification dedupes only against sends recorded
    // after the latest recovery, so a recurrence notifies again.
    const latestByKey = new Map<string, NotificationHistoryEntry>();
    for (const entry of subHistory) {
      const key = keyOf(entry);
      const prev = latestByKey.get(key);
      if (!prev || Date.parse(entry.at) >= Date.parse(prev.at)) latestByKey.set(key, entry);
    }
    for (const [key, latest] of latestByKey) {
      if (currentKeys.has(key) || latest.outcome === "recovered") continue;
      const marker: NotificationHistoryEntry = {
        at: new Date(now).toISOString(),
        class: latest.class,
        objectRef: latest.objectRef,
        stateFingerprint: latest.stateFingerprint,
        outcome: "recovered",
        subscriptionId: sub.id,
      };
      await append(marker);
      subHistory.push(marker);
    }

    for (const candidate of enabledCandidates) {
      const key = keyOf({ ...candidate, subscriptionId: sub.id });
      const decision = decideNotification(candidate, subHistory, now, inQuietHours(sub.quiet, new Date(now)));
      const suppressedOutcome: NotificationHistoryEntry["outcome"] =
        decision === "digest"
          ? "suppressed-quiet"
          : decision === "suppress-dedupe"
            ? "suppressed-dedupe"
            : decision === "suppress-cooldown"
              ? "suppressed-cooldown"
              : "sent";
      if (decision === "digest") {
        if (
          !subHistory.some(
            (entry) =>
              entry.class === candidate.class &&
              entry.objectRef === candidate.objectRef &&
              entry.stateFingerprint === candidate.stateFingerprint &&
              entry.outcome === suppressedOutcome,
          )
        ) {
          const suppressed: NotificationHistoryEntry = {
            at: new Date(now).toISOString(),
            class: candidate.class,
            objectRef: candidate.objectRef,
            stateFingerprint: candidate.stateFingerprint,
            outcome: "suppressed-quiet",
            subscriptionId: sub.id,
          };
          await append(suppressed);
          subHistory.push(suppressed);
        }
        if (!pending.has(key)) {
          const queued: NotificationHistoryEntry = {
            at: new Date(now).toISOString(),
            class: candidate.class,
            objectRef: candidate.objectRef,
            stateFingerprint: candidate.stateFingerprint,
            outcome: "queued",
            subscriptionId: sub.id,
            candidate,
            dueAt: new Date(nextNotificationDigestAt(now, sub.quiet)).toISOString(),
          };
          await append(queued);
          pending.set(key, queued);
        }
        continue;
      }
      if (decision !== "send") {
        if (
          !subHistory.some(
            (entry) =>
              entry.class === candidate.class &&
              entry.objectRef === candidate.objectRef &&
              entry.stateFingerprint === candidate.stateFingerprint &&
              entry.outcome === suppressedOutcome,
          )
        ) {
          const suppressed: NotificationHistoryEntry = {
            at: new Date(now).toISOString(),
            class: candidate.class,
            objectRef: candidate.objectRef,
            stateFingerprint: candidate.stateFingerprint,
            outcome: suppressedOutcome,
            subscriptionId: sub.id,
          };
          await append(suppressed);
          subHistory.push(suppressed);
        }
        continue;
      }
      if (pending.has(key)) continue; // due entries are sent together below
      const url = /^https?:\/\//.test(candidate.url) ? candidate.url : `${cfg.publicOrigin}${candidate.url}`;
      const payload: PushPayload = {
        class: candidate.class,
        objectRef: candidate.objectRef,
        url,
        title: candidate.title,
        body: candidate.body,
        ...(candidate.escalationStep ? { escalation: true } : {}),
      };
      const target: PushTarget = { endpoint: sub.endpoint, p256dh: sub.p256dh, auth: sub.auth };
      const result = await push(target, payload);
      const outcome = result.ok ? "sent" : result.gone ? "gone" : "error";
      const entry: NotificationHistoryEntry = {
        at: new Date(now).toISOString(),
        class: candidate.class,
        objectRef: candidate.objectRef,
        stateFingerprint: candidate.stateFingerprint,
        outcome,
        subscriptionId: sub.id,
      };
      await append(entry);
      subHistory.push(entry);
      await store.logSend({ class: candidate.class, objectRef: candidate.objectRef, outcome });
      if (result.ok) await store.noteSubscriptionAlive(sub.endpoint);
      else if (result.gone) await store.noteSubscriptionGone(sub.endpoint);
    }

    const due = [...pending.entries()].filter(([, entry]) => Date.parse(entry.dueAt ?? "") <= now);
    const digestCandidates = due.map(([, entry]) => entry.candidate).filter((candidate): candidate is NonNullable<typeof candidate> => !!candidate);
    if (digestCandidates.length === 0) continue;
    // #7193 Codex: revalidate the CURRENT quiet window before draining —
    // the persisted dueAt was computed under an earlier window (or the
    // server was offline past it into a new quiet period). Defer to the
    // next allowed boundary instead of sending inside quiet hours.
    if (inQuietHours(sub.quiet, new Date(now))) {
      for (const [key, queued] of due) {
        const retry: NotificationHistoryEntry = {
          ...queued,
          at: new Date(now).toISOString(),
          dueAt: new Date(nextNotificationDigestAt(now + 60_000, sub.quiet)).toISOString(),
        };
        await append(retry);
        pending.set(key, retry);
      }
      continue;
    }
    const refs = digestCandidates.map((candidate) => candidate.objectRef);
    const items = digestCandidates.map((candidate) => ({
      class: candidate.class,
      objectRef: candidate.objectRef,
      title: candidate.title,
      url: /^https?:\/\//.test(candidate.url) ? candidate.url : `${cfg.publicOrigin}${candidate.url}`,
    }));
    const payload: PushPayload = {
      class: "digest",
      objectRef: `digest:${new Date(now).toISOString().slice(0, 10)}`,
      url: `${cfg.publicOrigin}/attention?refs=${encodeURIComponent(refs.join(","))}`,
      title: `${digestCandidates.length} dashboard update${digestCandidates.length === 1 ? "" : "s"}`,
      body: digestCandidates.map((candidate) => candidate.title).join(" · "),
      items,
    };
    const result = await push({ endpoint: sub.endpoint, p256dh: sub.p256dh, auth: sub.auth }, payload);
    const isGone = "gone" in result && result.gone;
    const outcome = result.ok ? "sent" : isGone ? "gone" : "error";
    for (const [key, queued] of due) {
      const candidate = queued.candidate;
      if (candidate) {
        await append({
          at: new Date(now).toISOString(),
          class: candidate.class,
          objectRef: candidate.objectRef,
          stateFingerprint: candidate.stateFingerprint,
          outcome,
          subscriptionId: sub.id,
        });
      }
      if (!result.ok && !isGone && candidate) {
        const retry: NotificationHistoryEntry = {
          ...queued,
          at: new Date(now).toISOString(),
          dueAt: new Date(nextNotificationDigestAt(now + 60_000, sub.quiet)).toISOString(),
        };
        await append(retry);
        pending.set(key, retry);
      } else {
        if (isGone && candidate) {
          await append({
            at: new Date(now).toISOString(),
            class: candidate.class,
            objectRef: candidate.objectRef,
            stateFingerprint: candidate.stateFingerprint,
            outcome: "queue-cancelled",
            subscriptionId: sub.id,
          });
        }
        pending.delete(key);
      }
    }
    await store.logSend({ class: "digest", objectRef: payload.objectRef, outcome });
    if (result.ok) await store.noteSubscriptionAlive(sub.endpoint);
    else if (isGone) await store.noteSubscriptionGone(sub.endpoint);
  }
}

export async function runAlertTick(
  cfg: Config,
  store: AlertsStore,
  push: PushSender = sendPush,
  options: AlertTickOptions = {},
): Promise<void> {
  const subs = await store.listSubscriptions();
  const targets = subs.filter((s) => classEnabled(s, "incident"));
  if (targets.length > 0) {
    for (const incident of await store.pendingDeliveries()) {
      const summary = await deliverToAll(store, targets, incident, cfg.publicOrigin, false, push);
      if (summary.sent > 0) {
        await store.markDelivered(incident.id, { ...summary });
      } else {
        console.warn(
          `alerts engine: delivery not marked for incident ${incident.id} (sent=0, gone=${summary.gone}, errors=${summary.errors})`,
        );
      }
    }
    for (const incident of await store.pendingEscalations(cfg.alerts.ackWindowMin)) {
      const summary = await deliverToAll(store, targets, incident, cfg.publicOrigin, true, push);
      if (summary.sent > 0) {
        await store.markEscalated(incident.id, { ...summary, ackWindowMin: cfg.alerts.ackWindowMin });
      } else {
        console.warn(
          `alerts engine: escalation not marked for incident ${incident.id} (sent=0, gone=${summary.gone}, errors=${summary.errors})`,
        );
      }
    }
  }
  await runDashboardNotificationTick(cfg, store, subs, push, options);
}

let engineStarted = false;

/** Start the ~60s engine loop (live mode only; fail-soft per tick). */
export function startAlertEngine(cfg: Config, dashboardSnapshot?: () => Promise<NotificationSnapshot>): void {
  if (engineStarted || cfg.source !== "live") return;
  engineStarted = true;
  let ticking = false;
  const tick = async () => {
    if (ticking) return;
    ticking = true;
    try {
      const store = await getAlertsStore();
      if (store) await runAlertTick(cfg, store, sendPush, { dashboardSnapshot });
    } catch (err) {
      console.error(`alerts engine: tick failed (${err instanceof Error ? err.message : "error"})`);
    } finally {
      ticking = false;
    }
  };
  setTimeout(tick, 10_000);
  setInterval(tick, cfg.alerts.tickMs);
}
