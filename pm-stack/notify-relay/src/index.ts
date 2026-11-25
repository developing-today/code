// pm-notify relay Worker (#8569 Phase 2 step 1): the Cloudflare
// half of the pm-stack dead-man's switch, per design record
// doc/2026-11-17_design_pm8569_notifications_phase2.md.
//
//   POST /mirror/subscriptions  (shared-secret auth, laptop -> edge)
//     Full-snapshot replace of the mirrored push subscriptions
//     under a monotonic seq (design §1). Step 1 of the split.
//   POST /heartbeat             (shared-secret auth, laptop -> edge)
//     The aggregator's ~60s beat {stackEpoch, seq, at}. Each beat
//     re-arms the Durable Object's alarm at last-beat + 150s; the
//     response carries the pending-answer count, the aggregator's
//     cue to drain (step 3).
//   GET /answers, POST /answers/ack  (shared-secret auth, laptop ->
//     edge) Step 3's drain: the answers taken at the edge, then an
//     ack by answer id that drops them (src/answers.ts).
//   GET /answer/:ledgerId?t=<token>  (token auth)
//   POST /api/needs-drew/:ledgerId/answer  (token auth in the body)
//     Step 3's answer intake: the needs-Drew answer page and its
//     POST, the same paths as the laptop's, so a push's answer link
//     works while the laptop is away. The edge stores only each
//     question's token hash, mirrored with the question (design §4).
//   GET /status                 (unauthenticated, deliberately
//     minimal: up/silent + since, snapshot seq, subscription
//     count, last delivery — never board data, never endpoints.)
//
// Everything else 404s — the route list is closed (design §2).
// Mirrored event rows are append-only, deduped by mirrorId, retained
// for 30 days, and deliberately never sent from the edge.
//
// The Durable Object (singleton, idFromName("pm-notify")) owns the
// episode state machine in src/relay.ts: an alarm fire with no
// intervening beat opens the silent episode and sends exactly one
// stack.silent push; the first beat after silence sends exactly
// one recovered push naming the downtime. The alarm is the only
// scheduler — no cron anywhere (design invariant 3).
import {
  ackAnswers,
  acceptAnswer,
  answeredFor,
  hashToken,
  lookupQuestion,
  renderAnsweredPage,
  renderAnswerPage,
  renderClosedPage,
  renderForbiddenPage,
  sameHash,
  validateAck,
  validateAnswerChoice,
  type AnswerableQuestion,
} from "./answers.ts";
import { sendPush } from "./push.ts";
import {
  applyAlarm,
  applyHeartbeat,
  applyMirrorEvent,
  applySnapshot,
  buildRecoveredPayload,
  buildSilentPayload,
  emptyRelayState,
  isAuthorized,
  takesSilentClass,
  validateHeartbeat,
  validateMirrorEvent,
  validateSnapshot,
  type DeliveryRecord,
  type RelayState,
} from "./relay.ts";

export interface Env {
  PM_NOTIFY: DurableObjectNamespace;
  /** Shared secret for the laptop -> edge POSTs (wrangler secret
   * put; Bitwarden name `pm-aggregator.notify-relay-shared-secret`). */
  RELAY_SHARED_SECRET?: string;
  /** The aggregator VAPID keypair's private key (wrangler secret
   * put; Bitwarden name `pm-aggregator.vapid-private-key`). */
  VAPID_PRIVATE_KEY?: string;
}

const SINGLETON = "pm-notify";
const STATE_KEY = "relay-state";

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

const ANSWER_PAGE_CSP =
  "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; " +
  "base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

const html = (body: string, status = 200): Response =>
  new Response(body, {
    status,
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
      "content-security-policy": ANSWER_PAGE_CSP,
      "referrer-policy": "no-referrer",
      "x-content-type-options": "nosniff",
    },
  });

const ANSWER_CORS = {
  "access-control-allow-origin": "*",
  "access-control-allow-methods": "POST",
  "access-control-allow-headers": "content-type",
  "access-control-max-age": "86400",
};

const ANSWER_PAGE = /^\/answer\/([^/]+)$/;
const ANSWER_POST = /^\/api\/needs-drew\/([^/]+)\/answer$/;

/** The ledger id from a matched path segment, or null when it does
 * not decode to a plausible id. */
function ledgerIdFrom(match: RegExpMatchArray | null): string | null {
  if (!match) return null;
  try {
    const id = decodeURIComponent(match[1]);
    return id.length > 0 && id.length <= 200 ? id : null;
  } catch {
    return null;
  }
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    const stub = env.PM_NOTIFY.get(env.PM_NOTIFY.idFromName(SINGLETON));
    const laptopRoute =
      (req.method === "POST" &&
        (url.pathname === "/mirror/subscriptions" ||
          url.pathname === "/mirror/event" ||
          url.pathname === "/heartbeat" ||
          url.pathname === "/answers/ack")) ||
      (req.method === "GET" && url.pathname === "/answers");
    if (laptopRoute) {
      if (!isAuthorized(req.headers, env.RELAY_SHARED_SECRET)) return json({ error: "unauthorized" }, 401);
      // The DO learns the Worker's public origin from the request
      // itself (for the /status deep link in push payloads) — no
      // configured URL to drift.
      const forwarded = new Request(req);
      forwarded.headers.set("X-Relay-Origin", url.origin);
      return stub.fetch(forwarded);
    }
    // The push notification's answer buttons POST from the console's
    // service worker, another origin; the token in the body is the
    // only authority, so the answer POST is open to any origin.
    if (req.method === "OPTIONS" && ANSWER_POST.test(url.pathname)) {
      return new Response(null, { status: 204, headers: ANSWER_CORS });
    }
    if (req.method === "POST" && ANSWER_POST.test(url.pathname)) {
      const forwarded = new Request(req);
      forwarded.headers.delete("X-Relay-Origin");
      const res = await stub.fetch(forwarded);
      const headers = new Headers(res.headers);
      headers.set("access-control-allow-origin", "*");
      return new Response(res.body, { status: res.status, headers });
    }
    const publicRoute = req.method === "GET" && (url.pathname === "/status" || ANSWER_PAGE.test(url.pathname));
    if (publicRoute) {
      // Only the authed laptop routes may teach the DO its origin.
      const forwarded = new Request(req);
      forwarded.headers.delete("X-Relay-Origin");
      return stub.fetch(forwarded);
    }
    return new Response("not found", { status: 404 });
  },
};

export class PmNotify {
  private operation: Promise<void> = Promise.resolve();

  constructor(
    private state: DurableObjectState,
    private env: Env,
  ) {}

  private async load(): Promise<RelayState> {
    const stored = await this.state.storage.get<Partial<RelayState>>(STATE_KEY);
    return stored
      ? {
          ...emptyRelayState(),
          ...stored,
          mirrorEvents: stored.mirrorEvents ?? [],
          pendingAnswers: stored.pendingAnswers ?? [],
          answered: stored.answered ?? [],
        }
      : emptyRelayState();
  }

  private async save(state: RelayState): Promise<void> {
    await this.state.storage.put(STATE_KEY, state);
  }

  /** Serialize read-modify-write operations so overlapping event, beat,
   * and alarm work cannot overwrite each other's whole-state updates. */
  private async serialize<T>(work: () => Promise<T>): Promise<T> {
    const previous = this.operation;
    let release!: () => void;
    this.operation = new Promise<void>((resolve) => {
      release = resolve;
    });
    await previous;
    try {
      return await work();
    } finally {
      release();
    }
  }

  /** Send one episode payload to every mirrored subscription that
   * takes the incident class, and record the outcome on the state.
   * A missing VAPID secret is recorded, never thrown — the episode
   * transition has already happened and must not wedge. */
  private async deliver(state: RelayState, kind: DeliveryRecord["kind"], payload: unknown): Promise<RelayState> {
    const targets = (state.snapshot?.subscriptions ?? []).filter(takesSilentClass);
    let record: DeliveryRecord;
    if (!this.env.VAPID_PRIVATE_KEY) {
      record = {
        kind,
        at: new Date().toISOString(),
        sent: 0,
        gone: 0,
        failed: 0,
        error: "VAPID_PRIVATE_KEY not configured",
      };
    } else {
      try {
        const summary = await sendPush(targets, payload, { vapidPrivateKey: this.env.VAPID_PRIVATE_KEY });
        record = { kind, at: new Date().toISOString(), sent: summary.sent, gone: summary.gone, failed: summary.failed };
      } catch (err) {
        record = {
          kind,
          at: new Date().toISOString(),
          sent: 0,
          gone: 0,
          failed: 0,
          error: err instanceof Error ? err.message : String(err),
        };
      }
    }
    return { ...state, lastDelivery: record };
  }

  async fetch(req: Request): Promise<Response> {
    return this.serialize(() => this.handleFetch(req));
  }

  private async handleFetch(req: Request): Promise<Response> {
    const url = new URL(req.url);
    let state = await this.load();
    const origin = req.headers.get("X-Relay-Origin");
    if (origin && origin !== state.origin) {
      state = { ...state, origin };
      await this.save(state);
    }

    if (req.method === "POST" && url.pathname === "/mirror/subscriptions") {
      const parsed = validateSnapshot(await req.json().catch(() => null));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      const applied = applySnapshot(state, parsed.value);
      if (applied.outcome === "regression") {
        return json(
          { error: `snapshotSeq ${parsed.value.snapshotSeq} regresses the stored seq ${state.snapshot?.snapshotSeq}` },
          409,
        );
      }
      await this.save(applied.state);
      return json({
        ok: true,
        outcome: applied.outcome,
        snapshotSeq: parsed.value.snapshotSeq,
        subscriptions: parsed.value.subscriptions.length,
      });
    }

    if (req.method === "POST" && url.pathname === "/mirror/event") {
      const parsed = validateMirrorEvent(await req.json().catch(() => null));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      const applied = applyMirrorEvent(state, parsed.value, Date.now());
      await this.save(applied.state);
      return json({ ok: true, outcome: applied.outcome, mirrorId: parsed.value.mirrorId });
    }

    if (req.method === "POST" && url.pathname === "/heartbeat") {
      const parsed = validateHeartbeat(await req.json().catch(() => null));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      const outcome = applyHeartbeat(state, parsed.value, Date.now());
      state = outcome.state;
      if (outcome.recovered) {
        state = await this.deliver(state, "recovered", buildRecoveredPayload(state, outcome.recovered.downtimeMs));
      }
      await this.save(state);
      await this.state.storage.setAlarm(outcome.alarmAt);
      // pendingAnswers is the aggregator's cue to drain (step 3).
      return json({
        ok: true,
        silent: state.heartbeat.silentSince !== null,
        pendingAnswers: state.pendingAnswers.length,
      });
    }

    if (req.method === "GET" && url.pathname === "/answers") {
      return json({ ok: true, answers: state.pendingAnswers });
    }

    if (req.method === "POST" && url.pathname === "/answers/ack") {
      const parsed = validateAck(await req.json().catch(() => null));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      const acked = ackAnswers(state, parsed.value);
      await this.save({ ...state, ...acked.state });
      return json({ ok: true, acked: acked.acked, pendingAnswers: acked.state.pendingAnswers.length });
    }

    const pageMatch = req.method === "GET" ? url.pathname.match(ANSWER_PAGE) : null;
    if (pageMatch) {
      const ledgerId = ledgerIdFrom(pageMatch);
      const token = url.searchParams.get("t");
      const found = ledgerId && token ? await this.authorizeQuestion(state, ledgerId, token) : null;
      if (!found) return html(renderForbiddenPage(), 403);
      const record = answeredFor(state, found.question.ledgerId, Date.now());
      if (record) return html(renderAnsweredPage(found.question, record));
      if (found.kind !== "open") return html(renderClosedPage(found.question, found.kind), 410);
      return html(renderAnswerPage(found.question, token as string));
    }

    const postMatch = req.method === "POST" ? url.pathname.match(ANSWER_POST) : null;
    if (postMatch) {
      const ledgerId = ledgerIdFrom(postMatch);
      const body = await req.json().catch(() => null);
      const token = body && typeof body === "object" && !Array.isArray(body) ? (body as Record<string, unknown>).token : undefined;
      const found =
        ledgerId && typeof token === "string" && token ? await this.authorizeQuestion(state, ledgerId, token) : null;
      if (!found) return json({ error: "unknown question or invalid token" }, 403);
      const now = Date.now();
      const record = answeredFor(state, found.question.ledgerId, now);
      if (record) {
        return json({ ok: true, duplicate: true, answerId: record.answerId, answeredAt: record.answeredAt });
      }
      if (found.kind !== "open") return json({ error: `question ${found.kind}`, state: found.kind }, 410);
      const choice = validateAnswerChoice(found.question, body as Record<string, unknown>);
      if (!choice.ok) return json({ error: choice.error }, 400);
      const accepted = acceptAnswer(state, found.question, choice.value, now, crypto.randomUUID());
      if (accepted.outcome === "full") return json({ error: "answer mailbox full; try again later" }, 503);
      await this.save({ ...state, ...accepted.state });
      if (accepted.outcome === "duplicate") {
        return json({ ok: true, duplicate: true, answerId: accepted.answerId, answeredAt: accepted.answeredAt });
      }
      return json({
        ok: true,
        duplicate: false,
        answerId: accepted.answer.answerId,
        answeredAt: accepted.answer.answeredAt,
      });
    }

    if (req.method === "GET" && url.pathname === "/status") {
      const hb = state.heartbeat;
      return json({
        up: hb.lastBeatAt !== null && hb.silentSince === null,
        silent: hb.silentSince !== null,
        silentSince: hb.silentSince ? new Date(hb.silentSince).toISOString() : null,
        lastBeatAt: hb.lastBeatAt ? new Date(hb.lastBeatAt).toISOString() : null,
        stackEpoch: hb.stackEpoch,
        snapshotSeq: state.snapshot?.snapshotSeq ?? null,
        subscriptions: state.snapshot?.subscriptions.length ?? 0,
        lastDelivery: state.lastDelivery,
      });
    }

    return new Response("not found", { status: 404 });
  }

  /** The mirrored question for a ledger id, when the token matches
   * its mirrored hash. Unknown questions and wrong tokens look the
   * same to the caller: null. */
  private async authorizeQuestion(
    state: RelayState,
    ledgerId: string,
    token: string,
  ): Promise<{ kind: "open" | "withdrawn" | "expired"; question: AnswerableQuestion } | null> {
    const found = lookupQuestion(state.mirrorEvents, ledgerId, Date.now());
    if (found.kind === "unknown") return null;
    if (token.length > 512 || !sameHash(await hashToken(token), found.question.tokenHash)) return null;
    return found;
  }

  /** The dead-man's switch itself: no beat inside the window =>
   * open the episode and send the one stack.silent push. */
  async alarm(): Promise<void> {
    return this.serialize(async () => {
      await this.handleAlarm();
    });
  }

  private async handleAlarm(): Promise<void> {
    const loaded = await this.load();
    const outcome = applyAlarm(loaded, Date.now());
    if (!outcome.opened) return;
    const delivered = await this.deliver(outcome.state, "silent", buildSilentPayload(outcome.state));
    await this.save(delivered);
  }
}
