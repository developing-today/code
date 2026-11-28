// The alert throat (#3676; design #3450 as amended by Drew
// 2026-11-01): every sentinel posts incidents to ONE intake; this
// module owns dedupe, delivery, escalation, and ack state; every
// transition lands in the Turso `alerts_*` ledger tables
// (incidents / incident_events / notification_sends); push endpoints
// and their preferences live in the one push registry (#14084 S1,
// push-registry.ts), which replaced alerts.push_subscriptions. Delivery is Web Push per the #1254 design
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
import { getD1StoreClient } from "./d1-stores.ts";
import { getTursoStoreClient } from "./turso-stores.ts";
import {
  canonicalClass,
  DEFAULT_DELIVERY,
  ensurePushRegistry,
  isPushClient,
  LEGACY_CLASS_ALIASES,
  MAX_DEVICE_ID_LENGTH,
  MAX_LABEL_LENGTH,
  ItemStageStore,
  mergeDelivery,
  normalizeClassMap,
  nowIso,
  readDelivery,
  UnifiedSendStore,
  type DeliveryPrefs,
  type PushClient,
  type PushTransport,
  type UnifiedSendInput,
  type UnifiedSendOutcome,
  type UnifiedSendRow,
} from "./push-registry.ts";
import { sqlInt, sqlJson, sqlStr, type SqlClient } from "./sql-client.ts";
import {
  buildNotificationCandidates,
  classTestObjectRef,
  decideNotification,
  nextNotificationDigestAt,
  notificationTestLanding,
  NOTIFICATION_CLASS_DEFAULTS,
  type NotificationHistoryEntry,
  type NotificationSnapshot,
} from "./notifications.ts";
import { JsonlNotificationHistory, type NotificationHistoryStore } from "./notification-history.ts";
import { postCostAnomalyTick, type CostEvidence } from "./cost-anomaly.ts";
import { runPushSentinelTick } from "./push-sentinel.ts";
import { getNeedsDrew } from "./needs-drew.ts";
import { getDecisionSet } from "./decisions.ts";
import { hostedDecisionPageUrl } from "./decision-links.ts";
import {
  notifyRelayIncidentEventCursor,
  requestNotifyRelayCandidatesMirror,
  requestNotifyRelayIncidentEvents,
} from "./notify-relay.ts";

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

export interface IncidentMirrorRow extends Incident {
  mirror_event_id: number;
  mirror_event_at: string;
  mirror_event_kind: IncidentEvent["kind"];
}

/** One endpoint of the push registry (#14084 S1) with its
 * preferences, in the shape every delivery path reads. The fields
 * from `transport` on are the registry's additions; a delivery path
 * that predates the registry reads the same keys it always did. */
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
  transport?: PushTransport;
  client?: PushClient;
  device_id?: string | null;
  label?: string | null;
  delivery?: DeliveryPrefs;
}

/** One alerts.notification_sends ledger row (#3319): one per send
 * attempt, written by logSend; acted_on_at is stamped by the
 * exactly-once Ack transition. #14419: logSend stamps sent_at
 * explicitly in ISO-8601 UTC; legacy rows written before that carry
 * only a defaulted created_at, which the reader normalizes (see
 * normalizeSendTimestamp / repairLegacySendRow in notifications.ts). */
export interface NotificationSendRow {
  class: string;
  object_ref: string;
  incident_id: string | null;
  outcome: string;
  sent_at?: string | null;
  created_at?: string | null;
  /** The live Turso table's defaulted send-time column (named `at` there). */
  at?: string | null;
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
  transport: PushTransport;
  client: PushClient;
  device_id: string | null;
  label: string | null;
  delivery: DeliveryPrefs;
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
    transport: sub.transport ?? "webpush",
    client: sub.client ?? "browser",
    device_id: sub.device_id ?? null,
    label: sub.label ?? null,
    delivery: sub.delivery ?? { ...DEFAULT_DELIVERY },
  };
}

// --- Pure helpers (unit-tested) ---------------------------------------------

/** Design §1: the incident class defaults ON for new subscriptions. */
export const DEFAULT_CLASSES: Record<string, boolean> = NOTIFICATION_CLASS_DEFAULTS;
/** Design §2: default quiet window 23:00–07:00 America/Chicago. */
export const DEFAULT_QUIET: QuietPrefs = { start: "23:00", end: "07:00", tz: "America/Chicago", enabled: true };

/** #4981: how many client-supplied class keys one subscription may carry
 * beyond {@link DEFAULT_CLASSES}. The class registry that would validate
 * names and give the set its real shape arrives with #3317; until then a
 * fixed cap keeps a client from growing the stored `classes` object
 * without bound. Keys past the cap are dropped, not stored. */
export const MAX_CLASS_KEYS = 16;
/** #4981: longest accepted class key. Real class names are short slugs
 * ("quota-wall"); anything longer is not a class this build knows. */
export const MAX_CLASS_KEY_LENGTH = 64;

/** Class ids renamed by a ruling — the registry owns the map;
 * re-exported here so existing importers keep working. */
export { LEGACY_CLASS_ALIASES, canonicalClass, normalizeClassMap };

export function mergeClasses(input: unknown): Record<string, boolean> {
  const out = { ...DEFAULT_CLASSES };
  let clientKeys = 0;
  const seenClient: Record<string, true> = {};
  if (input && typeof input === "object") {
    for (const [k, v] of Object.entries(normalizeClassMap(input as Record<string, unknown>))) {
      // #4981: bound growth — a key must be a plausible class name, and a
      // subscription may carry only MAX_CLASS_KEYS client-supplied keys.
      // Over-limit input is dropped rather than extending the stored
      // object. Object.hasOwn (not `in`) so prototype names like
      // "toString" count against the cap like any other client key.
      if (k.length === 0 || k.length > MAX_CLASS_KEY_LENGTH) continue;
      if (typeof v !== "boolean") continue;
      const isClientKey = !Object.hasOwn(DEFAULT_CLASSES, k) && !Object.hasOwn(seenClient, k);
      if (isClientKey && clientKeys >= MAX_CLASS_KEYS) continue;
      if (isClientKey) {
        clientKeys += 1;
        seenClient[k] = true;
      }
      out[k] = v;
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

/** Whether a subscription receives a notification class.
 *
 * A stored choice always wins, including an explicit opt-out. A class
 * with no stored choice was added after the subscription was created
 * (the stored map carries only the classes of its day, and upserts
 * touch only the keys the caller sent — #7193), so it takes the
 * reviewed default from NOTIFICATION_CLASS_DEFAULTS: the #13940
 * backfill for pre-class rows. Before this, only incident defaulted
 * on for such rows, which silently left pre-class subscriptions —
 * e.g. rows created before the #8568 classes existed — with every
 * later class off, even though the preferences page already rendered
 * those classes at their defaults. A class with no reviewed default
 * stays off. New rows receive the full reviewed defaults from
 * mergeClasses at creation, so this fallback only ever fires for
 * classes newer than the row. */
export function classEnabled(sub: { classes: Record<string, boolean> }, cls: string): boolean {
  const canonical = canonicalClass(cls);
  const legacy = Object.keys(LEGACY_CLASS_ALIASES).find((old) => LEGACY_CLASS_ALIASES[old] === canonical);
  return (
    sub.classes?.[canonical] ?? (legacy ? sub.classes?.[legacy] : undefined) ?? DEFAULT_CLASSES[canonical] ?? false
  );
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
  /** #8568: tappable answers for a needs-Drew question push; `url` is
   * the absolute answer-page URL including the per-question token. */
  answer?: { ledgerId: string; options: Array<{ id: string; label: string }>; url: string };
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
  /** #14084 S1 registry fields. `client` defaults to `browser`. */
  client?: PushClient;
  deviceId?: string;
  label?: string;
  /** Delivery settings, stored only when the caller sent them: an
   * upsert without `delivery` keeps the stored settings (the same
   * rule #7193 set for classes). */
  delivery?: DeliveryPrefs;
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
    for (const [k, v] of Object.entries(normalizeClassMap(b.classes as Record<string, unknown>))) {
      if (typeof v === "boolean") providedClasses[k] = v;
    }
  }
  if (b.client !== undefined && !isPushClient(b.client)) {
    return { ok: false, error: "client must be browser or android-app" };
  }
  const deviceId = typeof b.deviceId === "string" ? b.deviceId.trim() : "";
  if (deviceId.length > MAX_DEVICE_ID_LENGTH) return { ok: false, error: "deviceId too long" };
  const label = typeof b.label === "string" ? b.label.trim().slice(0, MAX_LABEL_LENGTH) : "";
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
      client: isPushClient(b.client) ? b.client : undefined,
      deviceId: deviceId || undefined,
      label: label || undefined,
      delivery: b.delivery !== undefined ? mergeDelivery(b.delivery) : undefined,
    },
  };
}

// --- The store (Turso `alerts_*` tables) ------------------------------------

interface PendingIncidentCursor {
  timestamp: string;
  id: string;
}

type PendingIncidentOrder = "first_seen_at" | "delivered_at";

/** The registry read: an endpoint joined to its preference row. */
export const SUBSCRIPTION_SELECT = `select e.id, e.transport, e.client, e.device_id, e.endpoint, e.p256dh, e.auth, e.label,
  e.user_agent, e.created_at, e.last_seen_at, e.gone_count, p.classes, p.quiet, p.delivery
  from push_endpoints e left join push_preferences p on p.endpoint_id = e.id`;

const parseJsonColumn = (value: unknown): unknown => {
  if (typeof value !== "string") return value;
  try {
    return JSON.parse(value);
  } catch {
    return null;
  }
};

/** Decode one registry row into the subscription shape. Stored values
 * are read through the intake merges, so an old or hand-edited row is
 * never reinterpreted (#3702 decision 3), and a legacy class id reads
 * as its class of record. */
export function readSubscriptionRow(row: Record<string, unknown>): PushSubscriptionRow {
  const classes = parseJsonColumn(row.classes);
  return {
    id: String(row.id),
    endpoint: String(row.endpoint),
    p256dh: String(row.p256dh),
    auth: String(row.auth),
    classes:
      classes && typeof classes === "object"
        ? normalizeClassMap(classes as Record<string, boolean>)
        : mergeClasses(undefined),
    quiet: mergeQuiet(parseJsonColumn(row.quiet)),
    user_agent: typeof row.user_agent === "string" ? row.user_agent : null,
    created_at: String(row.created_at),
    last_seen_at: String(row.last_seen_at),
    gone_count: Number(row.gone_count ?? 0),
    transport: "webpush",
    client: isPushClient(row.client) ? row.client : "browser",
    device_id: typeof row.device_id === "string" ? row.device_id : null,
    label: typeof row.label === "string" ? row.label : null,
    delivery: readDelivery(row.delivery),
  };
}

export class AlertsStore {
  // Each queue advances independently, even when a send fails and leaves
  // the incident eligible. The keyset cursor lives for this store's lifetime.
  private readonly deliveryCursor: { last: PendingIncidentCursor | null } = { last: null };
  private readonly escalationCursor: { last: PendingIncidentCursor | null } = { last: null };

  private stages: ItemStageStore | null = null;

  constructor(
    private readonly db: SqlClient,
    private readonly registryDb?: SqlClient,
  ) {}

  /** The push registry connection (CF-5e #15561): the D1 copy when the
   * backend switch names it, otherwise the ledger connection. Every
   * push_* table access goes through here; the alerts_* ledger tables
   * below keep using `db` directly. */
  private registry(): SqlClient {
    return this.registryDb ?? this.db;
  }

  /** The staged pipeline for answerable items (#14084 S1), over the
   * registry connection. */
  itemStages(): ItemStageStore {
    if (!this.stages) this.stages = new ItemStageStore(this.registry());
    return this.stages;
  }

  /** The registry connection behind the census and unified ledger. */
  registryClient(): SqlClient {
    return this.registry();
  }

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

  // --- Subscriptions: the one push registry (#14084 S1) ----------------------
  //
  // Endpoints and their preferences live in the registry tables
  // (push-registry.ts), not alerts.push_subscriptions. The row shape
  // every delivery path reads is unchanged; `/alerts/preferences`
  // behaves as before externally.

  private registryReady: Promise<void> | null = null;

  private ensureRegistry(): Promise<void> {
    if (!this.registryReady) {
      const pending = ensurePushRegistry(this.registry());
      this.registryReady = pending;
      pending.catch(() => {
        if (this.registryReady === pending) this.registryReady = null;
      });
    }
    return this.registryReady;
  }

  private async subscriptionByEndpoint(endpoint: string): Promise<PushSubscriptionRow | null> {
    const rows = await this.registry().query<Record<string, unknown>>(
      `${SUBSCRIPTION_SELECT} where e.endpoint = ${sqlStr(endpoint)}`,
    );
    return rows[0] ? readSubscriptionRow(rows[0]) : null;
  }

  async upsertSubscription(sub: NewSubscription): Promise<PushSubscriptionRow> {
    await this.ensureRegistry();
    const at = sqlStr(nowIso());
    const endpointRows = await this.registry().query<{ id: string }>(
      `insert into push_endpoints (id, transport, client, device_id, endpoint, p256dh, auth, label, user_agent, created_at, last_seen_at, gone_count)
       values (${sqlStr(crypto.randomUUID())}, ${sqlStr("webpush")}, ${sqlStr(sub.client ?? "browser")}, ${
         sub.deviceId ? sqlStr(sub.deviceId) : "null"
       }, ${sqlStr(sub.endpoint)}, ${sqlStr(sub.p256dh)}, ${sqlStr(sub.auth)}, ${sub.label ? sqlStr(sub.label) : "null"}, ${
         sub.userAgent ? sqlStr(sub.userAgent) : "null"
       }, ${at}, ${at}, 0)
       on conflict (endpoint) do update set
         p256dh = excluded.p256dh, auth = excluded.auth,
         client = ${sub.client ? "excluded.client" : "push_endpoints.client"},
         device_id = coalesce(excluded.device_id, push_endpoints.device_id),
         label = coalesce(excluded.label, push_endpoints.label),
         user_agent = excluded.user_agent, last_seen_at = excluded.last_seen_at, gone_count = 0
       returning id`,
    );
    const endpointId = endpointRows[0]?.id;
    if (!endpointId) throw new Error("push registry upsert returned no endpoint");
    await this.registry().query(
      `insert into push_preferences (endpoint_id, classes, quiet, delivery, updated_at)
       values (${sqlStr(endpointId)}, ${sqlJson(sub.classes)}, ${sqlJson(sub.quiet)}, ${sqlJson(
         sub.delivery ?? DEFAULT_DELIVERY,
       )}, ${at})
       on conflict (endpoint_id) do update set
         classes = json_patch(push_preferences.classes, ${sqlJson(sub.providedClasses ?? {})}),
         quiet = excluded.quiet,
         delivery = ${sub.delivery ? "excluded.delivery" : "push_preferences.delivery"},
         updated_at = excluded.updated_at`,
    );
    const row = await this.subscriptionByEndpoint(sub.endpoint);
    if (!row) throw new Error("push registry upsert did not read back");
    return row;
  }

  async deleteSubscription(endpoint: string): Promise<boolean> {
    await this.ensureRegistry();
    await this.registry().query(
      `delete from push_preferences where endpoint_id in (select id from push_endpoints where endpoint = ${sqlStr(endpoint)})`,
    );
    const rows = await this.registry().query<{ id: string }>(
      `delete from push_endpoints where endpoint = ${sqlStr(endpoint)} returning id`,
    );
    return rows.length > 0;
  }

  async listSubscriptions(): Promise<PushSubscriptionRow[]> {
    await this.ensureRegistry();
    const rows = await this.registry().query<Record<string, unknown>>(
      `${SUBSCRIPTION_SELECT} order by e.created_at, e.id`,
    );
    // #3702 decision 3: old stored prefs are not rewritten, but must not
    // be reinterpreted on read. Apply the same per-field fallbacks used at
    // intake so delivery and the prefs page see safe values.
    return rows.map(readSubscriptionRow);
  }

  /** Design §4: a 404/410 marks the subscription gone; two consecutive
   * gone responses prune it. A successful send resets the count (the
   * upsert path resets it too, on re-subscribe). */
  async noteSubscriptionGone(endpoint: string): Promise<"marked" | "pruned"> {
    await this.ensureRegistry();
    const rows = await this.registry().query<{ gone_count: number }>(
      `update push_endpoints set gone_count = gone_count + 1 where endpoint = ${sqlStr(endpoint)} returning gone_count`,
    );
    const count = rows[0]?.gone_count ?? 0;
    if (count >= 2) {
      await this.deleteSubscription(endpoint);
      return "pruned";
    }
    return "marked";
  }

  async noteSubscriptionAlive(endpoint: string): Promise<void> {
    await this.ensureRegistry();
    await this.registry().query(
      `update push_endpoints set gone_count = 0, last_seen_at = ${sqlStr(nowIso())} where endpoint = ${sqlStr(
        endpoint,
      )} and gone_count <> 0`,
    );
  }

  /**
   * Intake dedupe rides the partial unique index on (source,
   * incident_key) WHERE status <> 'closed': a fresh insert opens the
   * incident (+ `opened` event); a conflict bumps occurrences /
   * last_seen_at (+ `duplicate_suppressed` event only when a delivery
   * would otherwise repeat). `openedEventDetail` merges extra members
   * into the `opened` event's detail — the `healthHook` incident
   * record rides there (#12602: the ledger is the C3 record of
   * detail, so the down record lands on the event that opens it).
   */
  async postIncident(
    input: NewIncident,
    openedEventDetail?: Record<string, unknown>,
  ): Promise<{ incident: Incident; created: boolean }> {
    const inserted = await this.db.query<Incident>(
      `insert into alerts.incidents (source, incident_key, severity, title, body)
       values (${sqlStr(input.source)}, ${sqlStr(input.key)}, ${sqlStr(input.severity)}, ${sqlStr(input.title)}, ${
         input.body ? sqlStr(input.body) : "null"
       })
       on conflict (source, incident_key) where status <> 'closed' do nothing
       returning *`,
    );
    if (inserted[0]) {
      await this.addEvent(inserted[0].id, "opened", {
        source: input.source,
        key: input.key,
        severity: input.severity,
        ...openedEventDetail,
      });
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

  /** Read the append-only incident ledger from the relay's durable cursor. */
  async incidentEventsAfter(cursor: number, limit = 100): Promise<IncidentMirrorRow[]> {
    return this.db.query<IncidentMirrorRow>(
      `select e.id as mirror_event_id, e.at as mirror_event_at, e.kind as mirror_event_kind, i.*
       from alerts.incident_events e join alerts.incidents i on i.id = e.incident_id
       where e.id > ${sqlInt(cursor)} order by e.id limit ${sqlInt(limit)}`,
    );
  }

  async latestIncidentEventId(): Promise<number> {
    const rows = await this.db.query<{ id: number | null }>("select max(id) as id from alerts.incident_events");
    return rows[0]?.id ?? 0;
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
      // #14084 S2: the unified ledger's acted_on_at rides the same
      // exactly-once transition.
      await this.unifiedSends().markActedByIncident(id);
      return { incident: rows[0], transitioned: true };
    }
    return { incident: await this.getIncident(id), transitioned: false };
  }

  /** #8568: close the open incident for a (source, key) pair — the
   * sentinel's "up" transition resolves the incident its "down" post
   * opened. Reuses closeIncident for the transition + event.
   * `closedEventDetail` becomes the `closed` event's detail — the
   * `healthHook` recovery record rides there (#12602). */
  async closeOpenIncidentByKey(
    source: string,
    key: string,
    closedEventDetail?: Record<string, unknown>,
  ): Promise<{ incident: Incident | null; transitioned: boolean }> {
    const rows = await this.db.query<Incident>(
      `select * from alerts.incidents where source = ${sqlStr(source)} and incident_key = ${sqlStr(key)} and status <> 'closed' order by first_seen_at limit 1`,
    );
    const open = rows[0];
    if (!open) return { incident: null, transitioned: false };
    return this.closeIncident(open.id, closedEventDetail);
  }

  async closeIncident(
    id: string,
    closedEventDetail?: Record<string, unknown>,
  ): Promise<{ incident: Incident | null; transitioned: boolean }> {
    const rows = await this.db.query<Incident>(
      `update alerts.incidents set status = 'closed', closed_at = now()
       where id = ${sqlStr(id)} and status <> 'closed' returning *`,
    );
    if (rows[0]) {
      await this.addEvent(id, "closed", closedEventDetail ?? {});
      return { incident: rows[0], transitioned: true };
    }
    return { incident: await this.getIncident(id), transitioned: false };
  }

  /** Every ledger event for one incident, oldest first — the
   * per-incident read behind GET /api/alerts/:id, C6's detail path
   * for health-hook incident records (#12602). */
  async listIncidentEvents(incidentId: string): Promise<IncidentEvent[]> {
    return this.db.query<IncidentEvent>(
      `select * from alerts.incident_events where incident_id = ${sqlStr(incidentId)} order by id`,
    );
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
    /** #14419: injectable for tests; defaults to now. */
    sentAt?: string;
    /** Registry endpoint id, when the attempt targeted one endpoint. */
    endpointId?: string;
    /** Push-service detail for this attempt (status code / reason). */
    pushStatus?: string;
    /** The digest delivery a grouped send rode in, if any. */
    groupRef?: string;
  }): Promise<void> {
    // #14419: stamp sent_at explicitly in ISO-8601 UTC on every row.
    // Relying on the created_at column default left live rows in a
    // format the reader could not parse, so every send was excluded
    // from the measurement roll-up.
    const sentAt = entry.sentAt ?? new Date().toISOString();
    const cls = canonicalClass(entry.class);
    await this.db.query(
      `insert into alerts.notification_sends (class, object_ref, incident_id, outcome, sent_at)
       values (${sqlStr(cls)}, ${sqlStr(entry.objectRef)}, ${
         entry.incidentId ? sqlStr(entry.incidentId) : "null"
       }, ${sqlStr(entry.outcome)}, ${sqlStr(sentAt)})`,
    );
    // #14084 S2: dual-write the unified ledger. The legacy row above
    // stays byte-for-byte the shape #14061's backfill expects; the
    // unified row is the roll-up's input from cutover.
    await this.unifiedSends().log({
      endpointId: entry.endpointId,
      class: cls,
      objectRef: entry.objectRef,
      incidentId: entry.incidentId,
      outcome: entry.outcome,
      pushStatus: entry.pushStatus,
      sentAt,
      groupRef: entry.groupRef,
    });
  }

  /** The unified sends ledger (#14084 S2), over the registry connection. */
  unifiedSends(): UnifiedSendStore {
    return new UnifiedSendStore(this.registry());
  }

  /** One unified attempt outside the legacy dual-write (grouped sends:
   * each candidate in a digest delivery is its own attempt). */
  async logUnifiedSend(entry: UnifiedSendInput): Promise<void> {
    await this.unifiedSends().log(entry);
  }

  /** Every unified send-ledger row (#14084 S2 roll-up input). */
  async listUnifiedSends(): Promise<UnifiedSendRow[]> {
    return this.unifiedSends().list();
  }

  /** Open incidents for the dead-push sentinel (status + delivery state). */
  async listOpenIncidents(): Promise<{ status: string; delivered_at: string | null }[]> {
    return this.db.query<{ status: string; delivered_at: string | null }>(
      `select status, delivered_at from alerts.incidents where status = ${sqlStr("open")}`,
    );
  }

  /** Every send-ledger row (#3319 roll-up input). The ledger is one
   * row per attempt, so a full read stays small; the rolling window
   * is applied by the pure roll-up in src/notifications.ts. */
  async listSends(): Promise<NotificationSendRow[]> {
    return this.db.query<NotificationSendRow>(`select * from alerts.notification_sends`);
  }
}

/** Per-class test sends (#3318): the preferences page's send-test
 * button per class. The payload carries the class under test and
 * lands per the design §3 deep-link contract
 * (notificationTestLanding): a real candidate's own triage-capable
 * URL when one is supplied, otherwise the test item's class view —
 * never the origin root, and the digest test is the only
 * list-shaped landing (attention filtered to its refs). */
export function buildClassTestPayload(
  cls: string,
  origin: string,
  candidate?: { objectRef: string; title: string; url: string } | null,
): PushPayload {
  return {
    class: cls,
    objectRef: candidate?.objectRef ?? classTestObjectRef(cls),
    url: notificationTestLanding(cls, origin, candidate),
    title: `Test ${cls} — PM Stack`,
    body: candidate
      ? `Test ${cls} notification for ${candidate.title}. If you can read this, ${cls} delivery and its deep link work on this device.`
      : `Test ${cls} notification (test item ${classTestObjectRef(cls)}). If you can read this, ${cls} delivery and its deep link work on this device.`,
  };
}

export interface ClassTestPushSummary extends TestPushSummary {
  cls: string;
  /** Subscriptions the class toggle let through (all of them for
   * the digest test, which is a delivery shape, not a class). */
  eligible: number;
  /** The deep-link landing the test notification opens. */
  url: string;
}

/**
 * Send the per-class test push. Unlike the generic test, this one
 * honours the stored preference: a subscription whose class toggle
 * is off receives nothing — the same classEnabled check the engine
 * applies, so the test proves the toggle's effect. One sends-ledger
 * row per attempted subscription, under the tested class with the
 * test object ref. The sender is injectable, like sendTestPush.
 */
export async function sendClassTestPush(
  store: Pick<AlertsStore, "listSubscriptions" | "logSend" | "noteSubscriptionGone" | "noteSubscriptionAlive">,
  origin: string,
  cls: string,
  send: typeof sendPush = sendPush,
  candidate?: { objectRef: string; title: string; url: string } | null,
): Promise<ClassTestPushSummary> {
  const payload = buildClassTestPayload(cls, origin, candidate);
  const subs = await store.listSubscriptions();
  const targets = cls === "digest" ? subs : subs.filter((sub) => classEnabled(sub, cls));
  const summary: ClassTestPushSummary = {
    subscriptions: subs.length,
    eligible: targets.length,
    sent: 0,
    gone: 0,
    errors: 0,
    cls,
    url: payload.url,
  };
  for (const sub of targets) {
    const target: PushTarget = { endpoint: sub.endpoint, p256dh: sub.p256dh, auth: sub.auth };
    const outcome = await send(target, payload);
    if (outcome.ok) {
      summary.sent += 1;
      await store.logSend({ class: cls, objectRef: payload.objectRef, outcome: "sent", endpointId: sub.id });
      await store.noteSubscriptionAlive(sub.endpoint);
    } else if (outcome.gone) {
      summary.gone += 1;
      await store.logSend({ class: cls, objectRef: payload.objectRef, outcome: "gone", endpointId: sub.id });
      await store.noteSubscriptionGone(sub.endpoint);
    } else {
      summary.errors += 1;
      await store.logSend({ class: cls, objectRef: payload.objectRef, outcome: "error", endpointId: sub.id });
    }
  }
  return summary;
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
      await store.logSend({ class: "test", objectRef: TEST_OBJECT_REF, outcome: "sent", endpointId: sub.id });
      await store.noteSubscriptionAlive(sub.endpoint);
    } else if (outcome.gone) {
      summary.gone += 1;
      await store.logSend({ class: "test", objectRef: TEST_OBJECT_REF, outcome: "gone", endpointId: sub.id });
      await store.noteSubscriptionGone(sub.endpoint);
    } else {
      summary.errors += 1;
      await store.logSend({ class: "test", objectRef: TEST_OBJECT_REF, outcome: "error", endpointId: sub.id });
    }
  }
  return summary;
}

/**
 * Push registry backend switch (CF-5e #15561 slice 1). `d1` routes the
 * five push_* registry tables and the stage pipeline to the D1
 * fleet-platform copy through getD1StoreClient; anything else keeps the
 * Turso registry. The alerts_* ledger tables always stay on getClient.
 * Rollback is unsetting the variable. The D1 copy, refresh-at-flip and
 * live verification are later slices — do not set this in production
 * until they land.
 */
export const PUSH_REGISTRY_BACKEND_ENV = "PM_PUSH_REGISTRY_BACKEND";

export type PushRegistryBackend = "turso" | "d1";

export function pushRegistryBackend(env: NodeJS.ProcessEnv = process.env): PushRegistryBackend {
  return env[PUSH_REGISTRY_BACKEND_ENV]?.trim().toLowerCase() === "d1" ? "d1" : "turso";
}

/**
 * Create the shared store resolver. Keep the in-flight attempt and successful
 * store cached, but clear null/rejected attempts so a later call can recover.
 * Lazy-init contract (success-only memoization): README "Lazy-initialization contract" (#3879).
 *
 * Fail-closed: with the D1 registry backend requested but no D1 client
 * available, the getter rejects instead of silently serving the registry
 * from Turso — a split-brain write would diverge the two copies. The
 * rejection clears like any other, so a later call retries.
 */
export function createAlertsStoreGetter(
  getClient: () => Promise<SqlClient | null> = getTursoStoreClient,
  getRegistryClient: () => Promise<SqlClient | null> = getD1StoreClient,
  env: NodeJS.ProcessEnv = process.env,
): () => Promise<AlertsStore | null> {
  let storePromise: Promise<AlertsStore | null> | null = null;

  return function getAlertsStore(): Promise<AlertsStore | null> {
    if (!storePromise) {
      const pending = (async () => {
        const db = await getClient();
        if (!db) return null;
        if (pushRegistryBackend(env) === "d1") {
          const registry = await getRegistryClient();
          if (!registry) throw new Error("D1 push registry unavailable");
          return new AlertsStore(db, registry);
        }
        return new AlertsStore(db);
      })();
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
  };
}

/** The shared store; null when the Turso store client is unavailable. */
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
        endpointId: sub.id,
      });
      await store.noteSubscriptionAlive(sub.endpoint);
    } else if (outcome.gone) {
      summary.gone += 1;
      await store.logSend({
        class: "incident",
        objectRef: payload.objectRef,
        incidentId: incident.id,
        outcome: "gone",
        endpointId: sub.id,
      });
      await store.noteSubscriptionGone(sub.endpoint);
    } else {
      summary.errors += 1;
      await store.logSend({
        class: "incident",
        objectRef: payload.objectRef,
        incidentId: incident.id,
        outcome: "error",
        endpointId: sub.id,
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
  /** #4825: cost-anomaly evidence loader. When a caller injects one,
   * the tick runs the detector over the loaded evidence and posts
   * warning incidents to the same store, so they surface through the
   * ordinary ledger (Needs-you / brief) with ordinary dedupe. Absent
   * = the detector stays dormant; no production caller injects one
   * yet because the certified evidence sources are still unwired
   * (see COST-ANOMALY.md). */
  costAnomalyEvidence?: () => Promise<readonly CostEvidence[]>;
}

async function runDashboardNotificationTick(
  cfg: Config,
  store: AlertsStore,
  subscriptions: PushSubscriptionRow[],
  push: PushSender,
  options: AlertTickOptions,
): Promise<void> {
  if (!options.dashboardSnapshot) return;
  const now = options.now ?? Date.now();
  const historyStore = options.notificationHistory ?? new JsonlNotificationHistory(cfg.notificationsFile);
  const [snapshot, history] = await Promise.all([options.dashboardSnapshot(), historyStore.read()]);
  const candidates = buildNotificationCandidates(snapshot, now);
  await requestNotifyRelayCandidatesMirror(candidates);
  if (subscriptions.length === 0) return;

  const keyOf = (
    entry: Pick<NotificationHistoryEntry, "class" | "objectRef" | "stateFingerprint" | "subscriptionId">,
  ) => `${entry.subscriptionId ?? ""}\u0000${entry.class}\u0000${entry.objectRef}\u0000${entry.stateFingerprint}`;
  const pendingFor = (subId: string): Map<string, NotificationHistoryEntry> => {
    const pending = new Map<string, NotificationHistoryEntry>();
    for (const entry of history) {
      if (entry.subscriptionId !== subId) continue;
      const key = keyOf(entry);
      if (entry.outcome === "queued") pending.set(key, entry);
      else if (entry.outcome === "queue-cancelled" || entry.outcome === "sent" || entry.outcome === "gone")
        pending.delete(key);
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
      // #8568: a needs-Drew candidate pushes with tappable answers —
      // the payload url becomes the token-carrying answer page.
      if (candidate.needsDrew) {
        try {
          const item = await getNeedsDrew(cfg.needsDrewFile, candidate.needsDrew.ledgerId);
          if (item) {
            // #13469: with the edge relay configured, the answer page
            // and its POST live on the Worker, so a push answers even
            // while the laptop is away; the drain applies it here.
            const answerOrigin = cfg.notifyRelay.url ? cfg.notifyRelay.url.replace(/\/+$/, "") : cfg.publicOrigin;
            const answerUrl = `${answerOrigin}/answer/${encodeURIComponent(item.ledgerId)}?t=${item.token}`;
            payload.answer = { ledgerId: item.ledgerId, options: item.options, url: answerUrl };
            payload.url = answerUrl;
          }
        } catch (err) {
          console.error(
            `alerts engine: needs-drew answer payload lookup failed (${err instanceof Error ? err.message : "error"})`,
          );
        }
      }
      // #8766: a decision-set candidate pushes with the set page as
      // its URL — the per-set token rides the URL; the page itself is
      // the answer surface, so no `answer` payload is attached.
      if (candidate.decisionSet) {
        try {
          const set = await getDecisionSet(cfg.decisionsFile, candidate.decisionSet.setId);
          if (set) {
            payload.url = hostedDecisionPageUrl(set.setId);
          }
        } catch (err) {
          console.error(
            `alerts engine: decision-set payload lookup failed (${err instanceof Error ? err.message : "error"})`,
          );
        }
      }
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
      await store.logSend({ class: candidate.class, objectRef: candidate.objectRef, outcome, endpointId: sub.id });
      if (result.ok) await store.noteSubscriptionAlive(sub.endpoint);
      else if (result.gone) await store.noteSubscriptionGone(sub.endpoint);
    }

    const due = [...pending.entries()].filter(([, entry]) => Date.parse(entry.dueAt ?? "") <= now);
    const digestCandidates = due
      .map(([, entry]) => entry.candidate)
      .filter((candidate): candidate is NonNullable<typeof candidate> => !!candidate);
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
    await store.logSend({ class: "digest", objectRef: payload.objectRef, outcome, endpointId: sub.id });
    // #14084 S2: each grouped send is its own unified attempt under
    // the candidate's class (of record), tied to this digest delivery
    // by group_ref. The legacy ledger keeps the single digest row
    // above — its shape is #14061's territory, not this slice's. The
    // optional call keeps hand-rolled legacy-surface fakes working;
    // the real store always implements the unified ledger.
    for (const candidate of digestCandidates) {
      await store.logUnifiedSend?.({
        endpointId: sub.id,
        class: candidate.class,
        objectRef: candidate.objectRef,
        outcome,
        groupRef: payload.objectRef,
      });
    }
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
  let incidentCursor = notifyRelayIncidentEventCursor();
  if (incidentCursor === null) {
    incidentCursor = await store.latestIncidentEventId();
    await requestNotifyRelayIncidentEvents([], incidentCursor, cfg.publicOrigin);
  } else if (incidentCursor !== undefined) {
    while (true) {
      const rows = await store.incidentEventsAfter(incidentCursor, 100);
      if (rows.length === 0) break;
      const nextCursor = rows.at(-1)!.mirror_event_id;
      await requestNotifyRelayIncidentEvents(rows, nextCursor, cfg.publicOrigin);
      incidentCursor = nextCursor;
      if (rows.length < 100) break;
    }
  }
  for (const incident of await store.pendingDeliveries()) {
    if (targets.length > 0) {
      const summary = await deliverToAll(store, targets, incident, cfg.publicOrigin, false, push);
      if (summary.sent > 0) {
        await store.markDelivered(incident.id, { ...summary });
      } else {
        console.warn(
          `alerts engine: delivery not marked for incident ${incident.id} (sent=0, gone=${summary.gone}, errors=${summary.errors})`,
        );
      }
    }
  }
  if (targets.length > 0) {
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
  // #4825: the cost-anomaly step runs only when a caller injected an
  // evidence loader. Fail-soft like the rest of the tick: a loader or
  // store failure is logged and suppresses this tick's anomalies; it
  // never breaks delivery, escalation, or the dashboard tick.
  if (options.costAnomalyEvidence) {
    try {
      await postCostAnomalyTick({
        enabled: true,
        now: new Date(options.now ?? Date.now()),
        loadEvidence: options.costAnomalyEvidence,
        store,
      });
    } catch (err) {
      console.warn(`alerts engine: cost-anomaly step failed (${err instanceof Error ? err.message : "error"})`);
    }
  }
  await runDashboardNotificationTick(cfg, store, subs, push, options);
  // #14084 S2: the dead-push sentinel. Fail-soft by construction (the
  // tick helper never throws): a sentinel failure must not break
  // delivery, escalation, or the dashboard tick.
  await runPushSentinelTick(store, new Date(options.now ?? Date.now()));
}

let engineStarted = false;
let alertTickHook: (() => void) | null = null;

/** #8568: ask the running engine to tick promptly (e.g. a needs-Drew
 * question or a stack-health transition just landed at the intake).
 * The engine's own in-flight guard dedupes; no-op when not started. */
export function requestAlertTick(): void {
  try {
    alertTickHook?.();
  } catch {
    // A prompt tick is best-effort; the 60s loop remains the backstop.
  }
}

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
  alertTickHook = () => {
    void tick();
  };
  setTimeout(tick, 10_000);
  setInterval(tick, cfg.alerts.tickMs);
}
