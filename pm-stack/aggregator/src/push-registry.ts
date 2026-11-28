// The one push registry (#14084 S1; design of record
// doc/2026-11-24T04-45-43Z_design_pm-push-unification/, §3.1).
//
// Tables in the fleet-platform Turso store:
//
//   push_endpoints       one row per delivery endpoint. `transport` is
//                        `webpush` in v1 (the column is the reserved
//                        seat for a native transport); `client` is
//                        `browser` or `android-app`; `device_id` links
//                        an app endpoint to the console device registry.
//   push_preferences     one row per endpoint: the class map, quiet
//                        hours and delivery settings (Drew's ruling,
//                        #14084 comment 7124146989, point 4).
//   push_sends             the unified sends ledger (#14084 S2, §3.5):
//                        one row per send attempt, `sent_at` stamped
//                        by the writer as ISO-8601 UTC and
//                        schema-required (NOT NULL, no default — the
//                        #14061 lesson); `acted_on_at` stamped by the
//                        exactly-once Ack transition; `group_ref` ties
//                        a grouped send to its digest delivery.
//   push_item_stages     the staged pipeline for answerable items:
//                        everything enters `needs-review`, only what
//                        has to reach the user escalates to
//                        `needs-user`, and "send back" returns an item
//                        to review (points 2, 3 and 5 of the ruling).
//   push_item_stage_events  one row per stage transition.
//   push_registry_meta   one-shot markers (the legacy seed).
//
// Timestamps are written by the writer as ISO-8601 UTC, never by a
// column default (the #14061 lesson: a defaulted time in a translated
// store is a time nobody can read back). The DDL carries no string
// literal, because the Turso transport turns every literal into a
// bound parameter and SQLite rejects parameters in DDL.
//
// Endpoint key material (`p256dh`, `auth`) stays server side; HTTP
// surfaces project through `toPublicSubscription` in alerts.ts.
import type { SqlClient } from "./sql-client.ts";
import { sqlInt, sqlJson, sqlStr } from "./sql-client.ts";

/** Class ids renamed by a ruling, old id → class of record. #14084
 * (Drew, 2026-11-24, comment 7124146989): the answerable class is
 * `needs-user`, not `needs-drew`. A stored map or a caller that still
 * says `needs-drew` means `needs-user`; the old key is never stored
 * again and never reaches the engine. The registry owns stored class
 * ids, so the map lives here; alerts.ts re-exports it. */
export const LEGACY_CLASS_ALIASES: Readonly<Record<string, string>> = { "needs-drew": "needs-user" };

export const canonicalClass = (cls: string): string =>
  Object.hasOwn(LEGACY_CLASS_ALIASES, cls) ? LEGACY_CLASS_ALIASES[cls] : cls;

/** Rewrite a class map onto the classes of record. Where a map holds
 * both the old and the new key, the new key's choice wins. */
export function normalizeClassMap<T>(input: Record<string, T>): Record<string, T> {
  const out: Record<string, T> = {};
  for (const [k, v] of Object.entries(input)) {
    const canonical = canonicalClass(k);
    if (canonical !== k && Object.hasOwn(input, canonical)) continue;
    out[canonical] = v;
  }
  return out;
}

export const PUSH_REGISTRY_DDL: readonly string[] = [
  `CREATE TABLE IF NOT EXISTS push_endpoints (
     id TEXT PRIMARY KEY,
     transport TEXT NOT NULL,
     client TEXT NOT NULL,
     device_id TEXT,
     endpoint TEXT NOT NULL UNIQUE,
     p256dh TEXT NOT NULL,
     auth TEXT NOT NULL,
     label TEXT,
     user_agent TEXT,
     created_at TEXT NOT NULL,
     last_seen_at TEXT NOT NULL,
     gone_count INTEGER NOT NULL DEFAULT 0
   )`,
  `CREATE INDEX IF NOT EXISTS push_endpoints_transport_client ON push_endpoints (transport, client)`,
  `CREATE TABLE IF NOT EXISTS push_sends (
      id INTEGER PRIMARY KEY,
      endpoint_id TEXT,
      class TEXT NOT NULL,
      object_ref TEXT NOT NULL,
      incident_id TEXT,
      outcome TEXT NOT NULL,
      push_status TEXT,
      sent_at TEXT NOT NULL,
      acted_on_at TEXT,
      group_ref TEXT
    )`,
  `CREATE INDEX IF NOT EXISTS push_sends_endpoint ON push_sends (endpoint_id)`,
  `CREATE INDEX IF NOT EXISTS push_sends_sent ON push_sends (sent_at)`,
  `CREATE TABLE IF NOT EXISTS push_preferences (
     endpoint_id TEXT PRIMARY KEY,
     classes TEXT NOT NULL,
     quiet TEXT NOT NULL,
     delivery TEXT NOT NULL,
     updated_at TEXT NOT NULL
   )`,
  `CREATE TABLE IF NOT EXISTS push_item_stages (
     item_ref TEXT PRIMARY KEY,
     stage TEXT NOT NULL,
     priority INTEGER,
     blocked_by_count INTEGER NOT NULL DEFAULT 0,
     entered_stage_at TEXT NOT NULL,
     updated_at TEXT NOT NULL
   )`,
  `CREATE INDEX IF NOT EXISTS push_item_stages_stage ON push_item_stages (stage, priority, blocked_by_count)`,
  `CREATE TABLE IF NOT EXISTS push_item_stage_events (
     id INTEGER PRIMARY KEY,
     item_ref TEXT NOT NULL,
     from_stage TEXT,
     to_stage TEXT NOT NULL,
     reason TEXT,
     at TEXT NOT NULL
   )`,
  `CREATE INDEX IF NOT EXISTS push_item_stage_events_item ON push_item_stage_events (item_ref, id)`,
  `CREATE TABLE IF NOT EXISTS push_registry_meta (
     key TEXT PRIMARY KEY,
     value TEXT NOT NULL,
     at TEXT NOT NULL
   )`,
];

/** Every registry table, in backup/export order. */
export const PUSH_REGISTRY_TABLES: Record<string, string> = {
  push_endpoints: "id",
  push_preferences: "endpoint_id",
  push_sends: "id",
  push_item_stages: "item_ref",
  push_item_stage_events: "id",
  push_registry_meta: "key",
};

export const LEGACY_SEED_MARKER = "legacy-alerts-push-subscriptions-seeded";

/** The writer-stamped time format: ISO-8601 UTC with milliseconds. */
export const nowIso = (now: Date = new Date()): string => now.toISOString();

/**
 * Create the registry tables (idempotent) and, once per store, copy
 * any rows still in the legacy `alerts_push_subscriptions` table into
 * the registry. The design verified that table empty (0 rows,
 * 2026-11-23), so in practice the seed moves nothing; it exists so
 * the adoption can never silently drop a subscription that arrived in
 * between. The marker row makes it one-shot: an endpoint unsubscribed
 * from the registry is never resurrected from the frozen legacy table
 * by a later restart.
 */
export async function ensurePushRegistry(db: SqlClient, now: Date = new Date()): Promise<void> {
  for (const statement of PUSH_REGISTRY_DDL) await db.query(statement);
  const marker = await db.query<{ key: string }>(
    `select key from push_registry_meta where key = ${sqlStr(LEGACY_SEED_MARKER)}`,
  );
  if (marker.length > 0) return;
  const legacyTable = await db.query<{ name: string }>(
    `select name from sqlite_master where type = ${sqlStr("table")} and name = ${sqlStr("alerts_push_subscriptions")}`,
  );
  // No legacy table (a fresh store, a test database): nothing to seed.
  // A failed query throws and leaves the marker unwritten, so the seed
  // is retried on the next start rather than skipped forever.
  const seeded = legacyTable.length > 0 ? await seedFromLegacy(db, now) : 0;
  await db.query(
    `insert into push_registry_meta (key, value, at) values (${sqlStr(LEGACY_SEED_MARKER)}, ${sqlStr(
      String(seeded),
    )}, ${sqlStr(nowIso(now))}) on conflict (key) do nothing`,
  );
}

async function seedFromLegacy(db: SqlClient, now: Date): Promise<number> {
  const stamp = sqlStr(nowIso(now));
  const legacy = await db.query<{ endpoint: string }>(`select endpoint from alerts_push_subscriptions`);
  if (legacy.length === 0) return 0;
  await db.query(
    `insert into push_endpoints (id, transport, client, device_id, endpoint, p256dh, auth, label, user_agent, created_at, last_seen_at, gone_count)
     select lower(hex(randomblob(16))), ${sqlStr("webpush")}, ${sqlStr("browser")}, null, endpoint, p256dh, auth, null, user_agent,
            coalesce(created_at, ${stamp}), coalesce(last_seen_at, created_at, ${stamp}), coalesce(gone_count, 0)
     from alerts_push_subscriptions where true
     on conflict (endpoint) do nothing`,
  );
  await db.query(
    `insert into push_preferences (endpoint_id, classes, quiet, delivery, updated_at)
     select e.id, coalesce(s.classes, ${sqlStr("{}")}), coalesce(s.quiet, ${sqlStr("{}")}), ${sqlJson(
       DEFAULT_DELIVERY,
     )}, ${stamp}
     from push_endpoints e join alerts_push_subscriptions s on s.endpoint = e.endpoint where true
     on conflict (endpoint_id) do nothing`,
  );
  return legacy.length;
}

// --- Endpoints ----------------------------------------------------------------

export type PushTransport = "webpush";
export type PushClient = "browser" | "android-app";
export const PUSH_CLIENTS: readonly PushClient[] = ["browser", "android-app"];

export function isPushClient(value: unknown): value is PushClient {
  return typeof value === "string" && (PUSH_CLIENTS as readonly string[]).includes(value);
}

/** Longest accepted endpoint label / device id. */
export const MAX_LABEL_LENGTH = 80;
export const MAX_DEVICE_ID_LENGTH = 128;

// --- Delivery settings (ruling point 4) -------------------------------------

/**
 * Per-endpoint delivery settings. Pushes send immediately by default;
 * the user can set a rate cap (at most `max` pushes per `perMinutes`),
 * grouping (items bundled into grouped pushes rather than one each),
 * and reminders for outstanding groups. These are stored per endpoint
 * as first-class settings; the one sender (S3) applies them.
 */
export interface DeliveryPrefs {
  immediate: boolean;
  rateCap: { max: number; perMinutes: number } | null;
  grouping: boolean;
  groupReminderMinutes: number | null;
}

export const DEFAULT_DELIVERY: DeliveryPrefs = {
  immediate: true,
  rateCap: null,
  grouping: false,
  groupReminderMinutes: null,
};

export const RATE_CAP_MAX = 1000;
/** One week, in minutes: the longest rate-cap window or reminder interval. */
export const MAX_WINDOW_MINUTES = 7 * 24 * 60;
export const MIN_GROUP_REMINDER_MINUTES = 5;

const boundedInt = (v: unknown, min: number, max: number): number | null =>
  typeof v === "number" && Number.isInteger(v) && v >= min && v <= max ? v : null;

/** Validate delivery settings as values, per field: an invalid field
 * falls back to its default, never stored as sent (the #3702 rule the
 * quiet-hours merge follows). */
export function mergeDelivery(input: unknown): DeliveryPrefs {
  const out: DeliveryPrefs = { ...DEFAULT_DELIVERY };
  if (!input || typeof input !== "object") return out;
  const d = input as Record<string, unknown>;
  if (typeof d.immediate === "boolean") out.immediate = d.immediate;
  if (typeof d.grouping === "boolean") out.grouping = d.grouping;
  if (d.rateCap && typeof d.rateCap === "object") {
    const cap = d.rateCap as Record<string, unknown>;
    const max = boundedInt(cap.max, 1, RATE_CAP_MAX);
    const perMinutes = boundedInt(cap.perMinutes, 1, MAX_WINDOW_MINUTES);
    if (max !== null && perMinutes !== null) out.rateCap = { max, perMinutes };
  }
  const reminder = boundedInt(d.groupReminderMinutes, MIN_GROUP_REMINDER_MINUTES, MAX_WINDOW_MINUTES);
  if (reminder !== null) out.groupReminderMinutes = reminder;
  return out;
}

/** Decode a stored delivery column (JSON text from Turso, an object
 * from a decoded row, or null on a row with no preference yet). */
export function readDelivery(value: unknown): DeliveryPrefs {
  if (typeof value === "string") {
    try {
      return mergeDelivery(JSON.parse(value));
    } catch {
      return { ...DEFAULT_DELIVERY };
    }
  }
  return mergeDelivery(value);
}

// --- The staged pipeline (ruling points 2, 3, 5) -----------------------------

export type ItemStage = "needs-review" | "needs-user" | "resolved";
export const ITEM_STAGES: readonly ItemStage[] = ["needs-review", "needs-user", "resolved"];

/** Item priority: 1 is the highest, 5 the lowest; null is unset. */
export const MIN_PRIORITY = 1;
export const MAX_PRIORITY = 5;

/** The standard answer every needs-user item carries: return the item
 * to the review stage, to be decided there (ruling point 3). */
export const SEND_BACK_OPTION = { id: "send-back", label: "Send back" } as const;

export interface ItemStageRow {
  item_ref: string;
  stage: ItemStage;
  priority: number | null;
  blocked_by_count: number;
  entered_stage_at: string;
  updated_at: string;
}

export interface ItemStageEvent {
  id: number;
  item_ref: string;
  from_stage: ItemStage | null;
  to_stage: ItemStage;
  reason: string | null;
  at: string;
}

export interface StageTransition {
  item: ItemStageRow | null;
  transitioned: boolean;
}

const MAX_ITEM_REF_LENGTH = 256;
const MAX_REASON_LENGTH = 2000;

function checkItemRef(itemRef: string): void {
  if (typeof itemRef !== "string" || !itemRef.trim() || itemRef.length > MAX_ITEM_REF_LENGTH) {
    throw new Error("item ref must be a non-empty string of at most 256 characters");
  }
}

const reasonSql = (reason?: string | null): string =>
  reason && reason.trim() ? sqlStr(reason.trim().slice(0, MAX_REASON_LENGTH)) : "null";

/**
 * The stage store. Every transition is one conditional UPDATE (the
 * row moves only from the stage the transition names), so concurrent
 * writers cannot both apply it; a transition that moved a row records
 * exactly one event.
 */
export class ItemStageStore {
  private ready: Promise<void> | null = null;

  constructor(
    private readonly db: SqlClient,
    private readonly clock: () => Date = () => new Date(),
  ) {}

  private ensure(): Promise<void> {
    if (!this.ready) {
      const pending = ensurePushRegistry(this.db, this.clock());
      this.ready = pending;
      pending.catch(() => {
        if (this.ready === pending) this.ready = null;
      });
    }
    return this.ready;
  }

  async get(itemRef: string): Promise<ItemStageRow | null> {
    checkItemRef(itemRef);
    await this.ensure();
    const rows = await this.db.query<ItemStageRow>(
      `select * from push_item_stages where item_ref = ${sqlStr(itemRef)}`,
    );
    return rows[0] ?? null;
  }

  private async event(
    itemRef: string,
    from: ItemStage | null,
    to: ItemStage,
    reason: string | null | undefined,
    at: string,
  ) {
    await this.db.query(
      `insert into push_item_stage_events (item_ref, from_stage, to_stage, reason, at) values (${sqlStr(itemRef)}, ${
        from ? sqlStr(from) : "null"
      }, ${sqlStr(to)}, ${reasonSql(reason)}, ${sqlStr(at)})`,
    );
  }

  /** Every item enters at needs-review (ruling point 2). Idempotent:
   * an item already in the pipeline is returned as it stands. */
  async enterReview(itemRef: string, reason?: string | null): Promise<StageTransition> {
    checkItemRef(itemRef);
    await this.ensure();
    const at = nowIso(this.clock());
    const rows = await this.db.query<ItemStageRow>(
      `insert into push_item_stages (item_ref, stage, priority, blocked_by_count, entered_stage_at, updated_at)
       values (${sqlStr(itemRef)}, ${sqlStr("needs-review")}, null, 0, ${sqlStr(at)}, ${sqlStr(at)})
       on conflict (item_ref) do nothing returning *`,
    );
    if (rows.length > 0) {
      await this.event(itemRef, null, "needs-review", reason, at);
      return { item: rows[0], transitioned: true };
    }
    return { item: await this.get(itemRef), transitioned: false };
  }

  private async move(
    itemRef: string,
    from: readonly ItemStage[],
    to: ItemStage,
    reason?: string | null,
  ): Promise<StageTransition> {
    checkItemRef(itemRef);
    await this.ensure();
    const before = await this.get(itemRef);
    if (!before || !from.includes(before.stage)) return { item: before, transitioned: false };
    const at = nowIso(this.clock());
    const rows = await this.db.query<ItemStageRow>(
      `update push_item_stages set stage = ${sqlStr(to)}, entered_stage_at = ${sqlStr(at)}, updated_at = ${sqlStr(at)}
       where item_ref = ${sqlStr(itemRef)} and stage = ${sqlStr(before.stage)} returning *`,
    );
    if (rows.length === 0) return { item: await this.get(itemRef), transitioned: false };
    await this.event(itemRef, before.stage, to, reason, at);
    return { item: rows[0], transitioned: true };
  }

  /** Escalate a reviewed item to the user. An item not yet in the
   * pipeline enters review first, so every escalation has a review
   * stage on its record. */
  async escalate(itemRef: string, reason?: string | null): Promise<StageTransition> {
    await this.enterReview(itemRef);
    return this.move(itemRef, ["needs-review"], "needs-user", reason);
  }

  /** "Send back" (ruling point 3): return a needs-user item to the
   * review stage, to be decided there. Only a needs-user item moves. */
  async sendBack(itemRef: string, reason?: string | null): Promise<StageTransition> {
    return this.move(itemRef, ["needs-user"], "needs-review", reason);
  }

  /** The item is decided (answered, handled in review, or dropped). */
  async resolve(itemRef: string, reason?: string | null): Promise<StageTransition> {
    return this.move(itemRef, ["needs-review", "needs-user"], "resolved", reason);
  }

  /** Re-open a resolved item at review. */
  async reopen(itemRef: string, reason?: string | null): Promise<StageTransition> {
    return this.move(itemRef, ["resolved"], "needs-review", reason);
  }

  /** Set (or clear, with null) an item's priority (ruling point 5). */
  async setPriority(itemRef: string, priority: number | null): Promise<ItemStageRow | null> {
    checkItemRef(itemRef);
    if (priority !== null && boundedInt(priority, MIN_PRIORITY, MAX_PRIORITY) === null) {
      throw new Error(`priority must be an integer ${MIN_PRIORITY}..${MAX_PRIORITY} or null`);
    }
    await this.ensure();
    const rows = await this.db.query<ItemStageRow>(
      `update push_item_stages set priority = ${priority === null ? "null" : sqlInt(priority)}, updated_at = ${sqlStr(
        nowIso(this.clock()),
      )} where item_ref = ${sqlStr(itemRef)} returning *`,
    );
    return rows[0] ?? null;
  }

  /** Record how many issues wait on this item's answer (ruling point 5). */
  async setBlockedByCount(itemRef: string, count: number): Promise<ItemStageRow | null> {
    checkItemRef(itemRef);
    if (boundedInt(count, 0, 1_000_000) === null) throw new Error("blocked-by count must be a non-negative integer");
    await this.ensure();
    const rows = await this.db.query<ItemStageRow>(
      `update push_item_stages set blocked_by_count = ${sqlInt(count)}, updated_at = ${sqlStr(
        nowIso(this.clock()),
      )} where item_ref = ${sqlStr(itemRef)} returning *`,
    );
    return rows[0] ?? null;
  }

  /** Items at one stage, in answering order: set priority first
   * (1 before 5, unset last), then the most issues unblocked by the
   * answer, then the longest waiting. */
  async list(stage: ItemStage, limit = 200): Promise<ItemStageRow[]> {
    await this.ensure();
    const rows = await this.db.query<ItemStageRow>(
      `select * from push_item_stages where stage = ${sqlStr(stage)} limit ${sqlInt(Math.max(1, Math.min(limit, 1000)))}`,
    );
    return sortForAnswering(rows);
  }

  async events(itemRef: string): Promise<ItemStageEvent[]> {
    checkItemRef(itemRef);
    await this.ensure();
    return this.db.query<ItemStageEvent>(
      `select * from push_item_stage_events where item_ref = ${sqlStr(itemRef)} order by id`,
    );
  }
}

/** Answering order for a stage list (ruling point 5), applied in code
 * so it does not depend on the store's NULL ordering. */
export function sortForAnswering(rows: ItemStageRow[]): ItemStageRow[] {
  return [...rows].sort((a, b) => {
    const pa = a.priority ?? Number.POSITIVE_INFINITY;
    const pb = b.priority ?? Number.POSITIVE_INFINITY;
    if (pa !== pb) return pa - pb;
    if (a.blocked_by_count !== b.blocked_by_count) return b.blocked_by_count - a.blocked_by_count;
    return a.entered_stage_at.localeCompare(b.entered_stage_at) || a.item_ref.localeCompare(b.item_ref);
  });
}

// --- The unified sends ledger (S2, §3.5) -----------------------------------

/** Outcome of one unified send attempt. `sent` is the only outcome
 * that reaches a device; every other outcome is an attempt the
 * roll-up must not count as delivery. `push_status` carries the
 * push-service detail (e.g. the Web Push status code) beside it. */
export type UnifiedSendOutcome = "sent" | "suppressed-quiet" | "suppressed-dedupe" | "gone" | "error";

/** One `push_sends` row: one attempt, dated by its writer. */
export interface UnifiedSendRow {
  id: number;
  endpoint_id: string | null;
  class: string;
  object_ref: string;
  incident_id: string | null;
  outcome: string;
  push_status: string | null;
  sent_at: string;
  acted_on_at: string | null;
  group_ref: string | null;
}

export interface UnifiedSendInput {
  endpointId?: string | null;
  class: string;
  objectRef: string;
  incidentId?: string | null;
  outcome: UnifiedSendOutcome;
  /** Push-service detail for this attempt (status code / reason). */
  pushStatus?: string | null;
  /** Injectable for tests; defaults to now. Always stored as ISO-8601 UTC. */
  sentAt?: string;
  /** The digest delivery this grouped send rode in, if any. */
  groupRef?: string | null;
}

/** The unified sends ledger. Every write stamps `sent_at` in
 * ISO-8601 UTC — the schema requires it (NOT NULL, no default), so
 * an undated row is a data defect, never a quiet default. */
export class UnifiedSendStore {
  private ready: Promise<void> | null = null;

  constructor(
    private readonly db: SqlClient,
    private readonly clock: () => Date = () => new Date(),
  ) {}

  private ensure(): Promise<void> {
    if (!this.ready) {
      const pending = ensurePushRegistry(this.db, this.clock());
      this.ready = pending;
      pending.catch(() => {
        if (this.ready === pending) this.ready = null;
      });
    }
    return this.ready;
  }

  /** Record one attempt. The class is canonicalized on write, so the
   * ledger only ever holds classes of record (`needs-user`, never
   * `needs-drew`). */
  async log(entry: UnifiedSendInput): Promise<UnifiedSendRow> {
    if (!entry.objectRef.trim()) throw new Error("unified send requires a non-empty object ref");
    await this.ensure();
    const sentAt = entry.sentAt ?? nowIso(this.clock());
    const rows = await this.db.query<UnifiedSendRow>(
      `insert into push_sends (endpoint_id, class, object_ref, incident_id, outcome, push_status, sent_at, acted_on_at, group_ref)
       values (${entry.endpointId ? sqlStr(entry.endpointId) : "null"}, ${sqlStr(
         canonicalClass(entry.class),
       )}, ${sqlStr(entry.objectRef)}, ${entry.incidentId ? sqlStr(entry.incidentId) : "null"}, ${sqlStr(
         entry.outcome,
       )}, ${entry.pushStatus ? sqlStr(entry.pushStatus) : "null"}, ${sqlStr(sentAt)}, null, ${
         entry.groupRef ? sqlStr(entry.groupRef) : "null"
       }) returning *`,
    );
    if (!rows[0]) throw new Error("unified send log did not return the row");
    return rows[0];
  }

  /** Every unified attempt, oldest first — the roll-up's input. */
  async list(limit = 10000): Promise<UnifiedSendRow[]> {
    await this.ensure();
    return this.db.query<UnifiedSendRow>(
      `select * from push_sends order by id limit ${sqlInt(Math.max(1, Math.min(limit, 100000)))}`,
    );
  }

  /** Stamp `acted_on_at` for one incident's sends (the exactly-once
   * Ack transition calls this beside the legacy stamp). Returns the
   * rows stamped. */
  async markActedByIncident(incidentId: string, at?: string): Promise<number> {
    await this.ensure();
    const rows = await this.db.query<{ id: number }>(
      `update push_sends set acted_on_at = ${sqlStr(at ?? nowIso(this.clock()))}
       where incident_id = ${sqlStr(incidentId)} and acted_on_at is null returning id`,
    );
    return rows.length;
  }

  /** The newest `sent_at` among outcome=`sent` rows, or null when
   * nothing was ever successfully sent. ISO-8601 strings order
   * chronologically, so the maximum is the latest. */
  async lastSuccessfulSendAt(): Promise<string | null> {
    await this.ensure();
    const rows = await this.db.query<{ sent_at: string | null }>(
      `select max(sent_at) as sent_at from push_sends where outcome = ${sqlStr("sent")}`,
    );
    return rows[0]?.sent_at ?? null;
  }

  /** Live endpoints by transport/client — the census's denominator. */
  async countByTransportClient(): Promise<{ transport: string; client: string; count: number }[]> {
    await this.ensure();
    return this.db.query<{ transport: string; client: string; count: number }>(
      `select transport, client, count(*) as count from push_endpoints group by transport, client order by transport, client`,
    );
  }
}

// --- The census (S2, §3.5) ---------------------------------------------------

export interface PushCensusTransportClient {
  transport: string;
  client: string;
  count: number;
}

export interface PushCensus {
  available: true;
  asOf: string;
  endpoints: { total: number; byTransportClient: PushCensusTransportClient[] };
  lastSuccessfulSendAt: string | null;
  needsUser: { open: number; totalBlockedBy: number };
}

/** One read that answers "how many live endpoints exist and what is
 * waiting on the user": endpoint counts by transport/client, the last
 * successful send, and the needs-user backlog with its total
 * blocked-by count. Served on the aggregator's status surfaces; the
 * console's `GET /push`-style status carries the same shape (its live
 * loader wires in with S4's registry read path). */
export async function readPushCensus(
  registry: SqlClient,
  stages: ItemStageStore,
  now: Date = new Date(),
): Promise<PushCensus> {
  const sends = new UnifiedSendStore(registry, () => now);
  const [counts, lastSuccessfulSendAt, backlog] = await Promise.all([
    sends.countByTransportClient(),
    sends.lastSuccessfulSendAt(),
    stages.list("needs-user", 1000),
  ]);
  return {
    available: true,
    asOf: nowIso(now),
    endpoints: {
      total: counts.reduce((n, c) => n + c.count, 0),
      byTransportClient: counts,
    },
    lastSuccessfulSendAt,
    needsUser: {
      open: backlog.length,
      totalBlockedBy: backlog.reduce((n, r) => n + r.blocked_by_count, 0),
    },
  };
}
