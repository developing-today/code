// Notify-relay client (#8569 Phase 2 step 1): the laptop half of
// the pm-notify dead-man's switch, per design record
// doc/2026-11-17_design_pm8569_notifications_phase2.md §1.
//
// Two flows out of the aggregator, both authenticated with the
// relay shared secret (Bitwarden by name, resolved at send time
// through the same secrets layer as the VAPID key):
//
//   - Heartbeat: POST {stackEpoch, seq, at} every ~60s. The
//     stackEpoch is minted per aggregator start; the edge treats
//     an epoch change after silence as the recovery boundary.
//     The response carries the pending-answer count (step 3's
//     drain; this client only reads it, the drain itself is
//     step 3).
//   - Subscription mirror: on any subscription-store change (the
//     server routes call requestNotifyRelaySubscriptionSync) the
//     COMPLETE mirrored set is POSTed as one snapshot under a
//     monotonically increasing snapshotSeq persisted in the
//     relay state file. The edge keeps only the highest seq; a
//     failed POST is retried with the same seq on the next beat,
//     and a 409 (our counter fell behind the edge's, e.g. the
//     state file was lost) re-adopts the edge's seq from /status.
//
// Everything here is fail-soft: the relay is a mirror of the
// throat, never a dependency of it. A down edge, a missing
// secret, or an unavailable alerts store must never break the
// aggregator's own sends — every public entry point swallows its
// errors after recording them on the client for /status-style
// inspection.
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { AlertsStore, PushSubscriptionRow } from "./alerts.ts";
import type { Config } from "./config.ts";
import type { NotificationCandidate } from "./notifications.ts";
import { resolveBwsSecret } from "./secrets.ts";

/** Bitwarden name of the throat<->Worker shared secret (design
 * §4: by name on the laptop side; the Worker holds the same value
 * as its RELAY_SHARED_SECRET). */
export const NOTIFY_RELAY_SECRET_NAME = "pm-aggregator.notify-relay-shared-secret";
export const NOTIFY_RELAY_HEARTBEAT_MS = 60_000;

export interface MirroredEvent {
  mirrorId: string;
  objectRef: string;
  transition: string;
  raisedAt: string;
  payload: Record<string, unknown>;
}

/** Stable across alert ticks and retries for one candidate state. */
export function notificationMirrorId(event: Pick<MirroredEvent, "objectRef" | "transition" | "payload">): string {
  return createHash("sha256")
    .update(JSON.stringify([event.objectRef, event.transition, event.payload.stateFingerprint ?? null]))
    .digest("hex");
}

export function notificationMirrorEvent(candidate: NotificationCandidate): MirroredEvent {
  const transition = "raised";
  const payload = {
    class: candidate.class,
    title: candidate.title,
    body: candidate.body,
    url: candidate.url,
    stateFingerprint: candidate.stateFingerprint,
    ...(candidate.needsDrew ? { needsDrew: candidate.needsDrew } : {}),
  };
  return {
    mirrorId: notificationMirrorId({ objectRef: candidate.objectRef, transition, payload }),
    objectRef: candidate.objectRef,
    transition,
    raisedAt: candidate.observedAt,
    payload,
  };
}

export interface MirroredSubscription {
  endpoint: string;
  p256dh: string;
  auth: string;
  classes: Record<string, boolean>;
  quiet: PushSubscriptionRow["quiet"];
}

/** The design §1 snapshot body: the complete mirrored set
 * (endpoints, keys, class map, quiet hours) under one seq. Only
 * delivery-relevant row fields cross — the store's bookkeeping
 * (ids, user agent, gone counts) stays home. */
export function buildSnapshotBody(rows: PushSubscriptionRow[], snapshotSeq: number) {
  return {
    snapshotSeq,
    subscriptions: rows.map(
      (row): MirroredSubscription => ({
        endpoint: row.endpoint,
        p256dh: row.p256dh,
        auth: row.auth,
        classes: row.classes,
        quiet: row.quiet,
      }),
    ),
  };
}

export function buildHeartbeatBody(stackEpoch: string, seq: number, at: string) {
  return { stackEpoch, seq, at };
}

export interface NotifyRelayDeps {
  url: string;
  stateFile: string;
  getStore: () => Promise<Pick<AlertsStore, "listSubscriptions"> | null>;
  resolveSecret?: (name: string) => Promise<string | null>;
  fetchImpl?: typeof fetch;
  now?: () => number;
  mintEpoch?: () => string;
}

export class NotifyRelayClient {
  readonly stackEpoch: string;
  private readonly resolveSecret: (name: string) => Promise<string | null>;
  private readonly fetchImpl: typeof fetch;
  private readonly now: () => number;
  private beatSeq = 0;
  /** Highest snapshot seq this process has assigned (persisted). */
  private snapshotSeq = 0;
  /** The snapshot content awaiting an ack, with the seq it was
   * assigned. A retry reuses both — the edge acks an equal seq
   * as a duplicate. */
  private pending: { seq: number; json: string } | null = null;
  /** The last snapshot content the edge acked. */
  private ackedJson: string | null = null;
  private syncDirty = true;
  private syncInFlight: Promise<void> | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;
  private pendingEvents: MirroredEvent[] = [];
  private ackedEvents = new Map<string, string>();
  private persistInFlight: Promise<void> = Promise.resolve();
  private eventFlushInFlight: Promise<void> | null = null;
  lastError: string | null = null;
  lastHeartbeatAt: string | null = null;
  lastPendingAnswers: number | null = null;

  constructor(private readonly deps: NotifyRelayDeps) {
    this.stackEpoch = (deps.mintEpoch ?? (() => crypto.randomUUID()))();
    this.resolveSecret = deps.resolveSecret ?? resolveBwsSecret;
    this.fetchImpl = deps.fetchImpl ?? fetch;
    this.now = deps.now ?? Date.now;
  }

  async start(): Promise<void> {
    await this.loadState();
    await this.beat();
    if (!this.timer) {
      this.timer = setInterval(() => void this.beat(), NOTIFY_RELAY_HEARTBEAT_MS);
      this.timer.unref?.();
    }
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  /** Called by the subscription routes after a store mutation:
   * mark the mirror stale and kick a sync now (the beat retries
   * until it lands). */
  requestSubscriptionSync(): void {
    this.syncDirty = true;
    void this.syncSubscriptions();
  }

  /** One contact: sync the mirror first if it owes one (so a
   * recovery beat doesn't close an episode before the snapshot
   * that authorizes its push has landed), then heartbeat. */
  async beat(): Promise<void> {
    try {
      if (this.syncDirty || this.pending) await this.syncSubscriptions();
      await this.flushMirrorEvents();
      await this.postHeartbeat();
    } catch (err) {
      this.lastError = err instanceof Error ? err.message : String(err);
    }
  }

  /** Persist before sending; an ambiguous network failure retries the
   * same mirrorId until the DO acknowledges it. */
  async mirrorEvent(event: MirroredEvent): Promise<void> {
    if (this.ackedEvents.has(event.mirrorId)) return;
    if (this.pendingEvents.some((pending) => pending.mirrorId === event.mirrorId)) return;
    this.pendingEvents.push(event);
    try {
      await this.persistState();
      void this.flushMirrorEvents();
    } catch (err) {
      this.lastError = err instanceof Error ? err.message : String(err);
    }
  }

  private async flushMirrorEvents(): Promise<void> {
    if (this.eventFlushInFlight) return this.eventFlushInFlight;
    const run = this.flushMirrorEventsOnce();
    this.eventFlushInFlight = run;
    try {
      await run;
    } finally {
      this.eventFlushInFlight = null;
    }
  }

  private async flushMirrorEventsOnce(): Promise<void> {
    while (this.pendingEvents.length > 0) {
      const event = this.pendingEvents[0]!;
      try {
        const secret = await this.resolveSecret(NOTIFY_RELAY_SECRET_NAME);
        if (!secret) throw new Error(`relay shared secret unavailable (bws: ${NOTIFY_RELAY_SECRET_NAME})`);
        const res = await this.fetchImpl(`${this.deps.url}/mirror/event`, {
          method: "POST",
          headers: { "content-type": "application/json", authorization: `Bearer ${secret}` },
          body: JSON.stringify(event),
          signal: AbortSignal.timeout(10_000),
        });
        if (!res.ok) throw new Error(`relay event POST failed: HTTP ${res.status}`);
        this.pendingEvents = this.pendingEvents.filter((pending) => pending.mirrorId !== event.mirrorId);
        this.ackedEvents.set(event.mirrorId, event.raisedAt);
        const cutoff = this.now() - 30 * 24 * 60 * 60_000;
        for (const [id, raisedAt] of this.ackedEvents) {
          if (!Number.isFinite(Date.parse(raisedAt)) || Date.parse(raisedAt) < cutoff) this.ackedEvents.delete(id);
        }
        await this.persistState();
      } catch (err) {
        this.lastError = err instanceof Error ? err.message : String(err);
        return; // keep the durable head item and retry on the next beat
      }
    }
  }

  async syncSubscriptions(): Promise<void> {
    if (this.syncInFlight) return this.syncInFlight;
    const run = (async () => {
      try {
        await this.syncOnce();
        this.lastError = null;
      } catch (err) {
        this.lastError = err instanceof Error ? err.message : String(err);
      } finally {
        this.syncInFlight = null;
      }
    })();
    this.syncInFlight = run;
    return run;
  }

  private async syncOnce(): Promise<void> {
    const store = await this.deps.getStore();
    if (!store) return; // alerts store unavailable (fixtures): nothing to mirror yet
    const rows = await store.listSubscriptions();
    const contentJson = JSON.stringify(buildSnapshotBody(rows, 0).subscriptions);
    if (!this.pending && contentJson === this.ackedJson) {
      this.syncDirty = false;
      return;
    }
    if (!this.pending || this.pending.json !== contentJson) {
      const seq = this.snapshotSeq + 1;
      await this.saveSnapshotSeq(seq);
      this.snapshotSeq = seq;
      this.pending = { seq, json: contentJson };
    }
    const secret = await this.resolveSecret(NOTIFY_RELAY_SECRET_NAME);
    if (!secret) throw new Error(`relay shared secret unavailable (bws: ${NOTIFY_RELAY_SECRET_NAME})`);
    const res = await this.fetchImpl(`${this.deps.url}/mirror/subscriptions`, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${secret}` },
      body: JSON.stringify({ snapshotSeq: this.pending.seq, subscriptions: JSON.parse(this.pending.json) }),
      signal: AbortSignal.timeout(10_000),
    });
    if (res.status === 409) {
      // Our counter fell behind the edge's (state file lost or
      // restored from backup): adopt the edge's seq from /status
      // and let the next attempt re-assign above it.
      await this.adoptEdgeSnapshotSeq();
      this.pending = null;
      this.syncDirty = true;
      return;
    }
    if (!res.ok) throw new Error(`relay snapshot POST failed: HTTP ${res.status}`);
    this.ackedJson = this.pending.json;
    this.pending = null;
    this.syncDirty = false;
  }

  private async adoptEdgeSnapshotSeq(): Promise<void> {
    const res = await this.fetchImpl(`${this.deps.url}/status`, { signal: AbortSignal.timeout(10_000) });
    if (!res.ok) throw new Error(`relay /status failed: HTTP ${res.status}`);
    const status = (await res.json()) as { snapshotSeq?: unknown };
    if (typeof status.snapshotSeq === "number" && status.snapshotSeq > this.snapshotSeq) {
      this.snapshotSeq = status.snapshotSeq;
      await this.saveSnapshotSeq(status.snapshotSeq);
    }
  }

  private async postHeartbeat(): Promise<void> {
    const secret = await this.resolveSecret(NOTIFY_RELAY_SECRET_NAME);
    if (!secret) throw new Error(`relay shared secret unavailable (bws: ${NOTIFY_RELAY_SECRET_NAME})`);
    this.beatSeq += 1;
    const res = await this.fetchImpl(`${this.deps.url}/heartbeat`, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${secret}` },
      body: JSON.stringify(buildHeartbeatBody(this.stackEpoch, this.beatSeq, new Date(this.now()).toISOString())),
      signal: AbortSignal.timeout(10_000),
    });
    if (!res.ok) throw new Error(`relay heartbeat POST failed: HTTP ${res.status}`);
    const body = (await res.json().catch(() => null)) as { pendingAnswers?: unknown } | null;
    this.lastHeartbeatAt = new Date(this.now()).toISOString();
    this.lastPendingAnswers = typeof body?.pendingAnswers === "number" ? body.pendingAnswers : null;
    if (this.pendingEvents.length === 0) this.lastError = null;
  }

  private async loadState(): Promise<void> {
    try {
      const parsed = JSON.parse(await readFile(this.deps.stateFile, "utf8")) as {
        snapshotSeq?: unknown;
        pendingEvents?: unknown;
        ackedEvents?: unknown;
      };
      if (
        typeof parsed.snapshotSeq === "number" &&
        Number.isSafeInteger(parsed.snapshotSeq) &&
        parsed.snapshotSeq > 0
      ) {
        this.snapshotSeq = parsed.snapshotSeq;
      }
      if (Array.isArray(parsed.pendingEvents)) {
        this.pendingEvents = parsed.pendingEvents.filter(
          (event): event is MirroredEvent =>
            !!event &&
            typeof event === "object" &&
            typeof (event as MirroredEvent).mirrorId === "string" &&
            typeof (event as MirroredEvent).objectRef === "string" &&
            typeof (event as MirroredEvent).transition === "string" &&
            typeof (event as MirroredEvent).raisedAt === "string" &&
            !!(event as MirroredEvent).payload &&
            typeof (event as MirroredEvent).payload === "object",
        );
      }
      if (Array.isArray(parsed.ackedEvents)) {
        this.ackedEvents = new Map(
          parsed.ackedEvents
            .filter(
              (event): event is { mirrorId: string; raisedAt: string } =>
                !!event &&
                typeof event === "object" &&
                typeof (event as { mirrorId?: unknown }).mirrorId === "string" &&
                typeof (event as { raisedAt?: unknown }).raisedAt === "string",
            )
            .map((event) => [event.mirrorId, event.raisedAt]),
        );
        const cutoff = this.now() - 30 * 24 * 60 * 60_000;
        for (const [id, raisedAt] of this.ackedEvents) {
          if (!Number.isFinite(Date.parse(raisedAt)) || Date.parse(raisedAt) < cutoff) this.ackedEvents.delete(id);
        }
      }
    } catch {
      // No state file yet (first run) or unreadable: start at 0;
      // a collision with the edge's stored seq surfaces as a 409
      // and re-adopts (see syncOnce).
    }
  }

  private async saveSnapshotSeq(seq: number): Promise<void> {
    this.snapshotSeq = seq;
    await this.persistState();
  }

  private async persistState(): Promise<void> {
    const write = this.persistInFlight.then(async () => {
      await mkdir(dirname(this.deps.stateFile), { recursive: true });
      await writeFile(
        this.deps.stateFile,
        JSON.stringify({
          snapshotSeq: this.snapshotSeq,
          pendingEvents: this.pendingEvents,
          ackedEvents: [...this.ackedEvents].map(([mirrorId, raisedAt]) => ({ mirrorId, raisedAt })),
        }),
        "utf8",
      );
    });
    this.persistInFlight = write.catch(() => {});
    return write;
  }
}

// ------------------------------------------------------ singleton wiring

let singleton: NotifyRelayClient | null = null;

/** Start the relay client if (and only if) a relay URL is
 * configured. Returns the client, or null when the relay is not
 * deployed/configured yet — the aggregator runs exactly as
 * before in that case. Called once from server.ts startup,
 * beside startAlertEngine. */
export function startNotifyRelay(
  cfg: Config,
  getStore: () => Promise<Pick<AlertsStore, "listSubscriptions"> | null>,
): NotifyRelayClient | null {
  if (!cfg.notifyRelay.url) return null;
  if (singleton) return singleton;
  singleton = new NotifyRelayClient({ url: cfg.notifyRelay.url, stateFile: cfg.notifyRelay.stateFile, getStore });
  void singleton.start().catch(() => {
    // start() records its own errors; the catch only keeps an
    // early rejection from surfacing as an unhandled rejection.
  });
  return singleton;
}

/** The subscription routes' hook: after any successful store
 * mutation, ask the relay to re-mirror. A no-op until the relay
 * is configured and started. */
export function requestNotifyRelaySubscriptionSync(): void {
  singleton?.requestSubscriptionSync();
}

/** Queue one push-worthy event for the silent edge mirror. */
export function requestNotifyRelayEventMirror(event: MirroredEvent): Promise<void> {
  return singleton?.mirrorEvent(event) ?? Promise.resolve();
}

/** Mirror only the Phase-2 step-2 notification transitions. */
export function requestNotifyRelayCandidateMirror(candidate: NotificationCandidate): Promise<void> {
  if (candidate.class !== "needs-drew" && candidate.class !== "quota-wall") return Promise.resolve();
  return requestNotifyRelayEventMirror(notificationMirrorEvent(candidate));
}
