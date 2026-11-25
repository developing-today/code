// pm-notify relay Worker (#8569 Phase 2 step 1): the Cloudflare
// half of the pm-stack dead-man's switch, per design record
// doc/2026-11-17_design_pm8569_notifications_phase2.md.
//
//   POST /mirror/subscriptions  (shared-secret auth, laptop -> edge)
//     Full-snapshot replace of the mirrored push subscriptions
//     under a monotonic seq (design §1). Step 1 of the split.
//   POST /mirror/questions       (shared-secret auth, current open questions;
//     stores the per-question token hash only)
//   POST /heartbeat             (shared-secret auth, laptop -> edge)
//     The aggregator's ~60s beat {stackEpoch, seq, at}. Each beat
//     re-arms the Durable Object's alarm at last-beat + 150s; the
//     response carries the pending-answer count (step 3's drain —
//     counts pending answers until the aggregator ACKs each answer id).
//   GET /status                 (unauthenticated, deliberately
//     minimal: up/silent + since, snapshot seq, subscription
//     count, last delivery — never board data, never endpoints.)
//
//   GET /answer/:ledgerId and POST /api/needs-drew/:ledgerId/answer
//     authorize with the per-question token and queue at the edge.
//   GET /pending-answers + POST /pending-answers/:answerId/ack
//     are shared-secret authenticated aggregator mailbox routes.
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
import { sendPush } from "./push.ts";
import {
  applyAlarm,
  applyAnswerAck,
  applyEdgeAnswer,
  applyHeartbeat,
  applyMirrorEvent,
  applySnapshot,
  buildRecoveredPayload,
  buildSilentPayload,
  emptyRelayState,
  isAuthorized,
  isQuestionOpen,
  timingSafeEqual,
  takesSilentClass,
  validateHeartbeat,
  validateMirrorEvent,
  applyMirroredQuestions,
  validateMirroredQuestions,
  type EdgeAnswerInput,
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
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json", "cache-control": "no-store" } });

const sha256 = async (value: string): Promise<string> => {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
};

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    const stub = env.PM_NOTIFY.get(env.PM_NOTIFY.idFromName(SINGLETON));
    if (
      req.method === "POST" &&
      (url.pathname === "/mirror/subscriptions" || url.pathname === "/mirror/event" || url.pathname === "/mirror/questions" || url.pathname === "/heartbeat")
    ) {
      if (!isAuthorized(req.headers, env.RELAY_SHARED_SECRET)) return json({ error: "unauthorized" }, 401);
      // The DO learns the Worker's public origin from the request
      // itself (for the /status deep link in push payloads) — no
      // configured URL to drift.
      const forwarded = new Request(req);
      forwarded.headers.set("X-Relay-Origin", url.origin);
      return stub.fetch(forwarded);
    }
    if (req.method === "GET" && (url.pathname === "/status" || url.pathname === "/answer" || url.pathname.startsWith("/answer/"))) return stub.fetch(req);
    if (req.method === "POST" && /^\/api\/needs-drew\/[^/]+\/answer$/.test(url.pathname)) return stub.fetch(req);
    if (req.method === "GET" && url.pathname === "/pending-answers") {
      if (!isAuthorized(req.headers, env.RELAY_SHARED_SECRET)) return json({ error: "unauthorized" }, 401);
      return stub.fetch(req);
    }
    if (req.method === "POST" && /^\/pending-answers\/[^/]+\/ack$/.test(url.pathname)) {
      if (!isAuthorized(req.headers, env.RELAY_SHARED_SECRET)) return json({ error: "unauthorized" }, 401);
      return stub.fetch(req);
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
    return stored ? { ...emptyRelayState(), ...stored, mirrorEvents: stored.mirrorEvents ?? [], questions: stored.questions ?? [], pendingAnswers: stored.pendingAnswers ?? [] } : emptyRelayState();
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

    if (req.method === "POST" && url.pathname === "/mirror/questions") {
      const parsed = validateMirroredQuestions(await req.json().catch(() => null));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      await this.save(applyMirroredQuestions(state, parsed.value));
      return json({ ok: true, questions: parsed.value.length });
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
      // pendingAnswers is step 3's drain count; the field exists
      // from the first heartbeat so the client contract is stable.
      return json({ ok: true, silent: state.heartbeat.silentSince !== null, pendingAnswers: state.pendingAnswers.length });
    }

    if (req.method === "GET" && url.pathname === "/pending-answers") {
      return json({ answers: state.pendingAnswers });
    }

    const ackMatch = /^\/pending-answers\/([^/]+)\/ack$/.exec(url.pathname);
    if (req.method === "POST" && ackMatch) {
      const answerId = decodeURIComponent(ackMatch[1]!);
      const outcome = applyAnswerAck(state, answerId);
      if (outcome.acknowledged) await this.save(outcome.state);
      return json({ ok: true, acknowledged: outcome.acknowledged, answerId });
    }

    const answerPost = /^\/api\/needs-drew\/([^/]+)\/answer$/.exec(url.pathname);
    if (req.method === "POST" && answerPost) {
      const ledgerId = decodeURIComponent(answerPost[1]!);
      const body = await req.json().catch(() => null) as Record<string, unknown> | null;
      if (!body || typeof body.token !== "string") return json({ error: "token required" }, 400);
      const item = state.questions.find((question) => question.ledgerId === ledgerId);
      if (!item || !isQuestionOpen(item, Date.now())) return json({ error: "question expired or withdrawn" }, 410);
      const answerTokenHash = await sha256(body.token);
      if (!timingSafeEqual(answerTokenHash, item.answerTokenHash)) return json({ error: "invalid token" }, 403);
      if (item.acceptedAnswerId) {
        return json({ ok: true, outcome: "duplicate", duplicate: true, answerId: item.acceptedAnswerId });
      }
      const optionId = typeof body.optionId === "string" ? body.optionId : undefined;
      const freeText = typeof body.freeText === "string" ? body.freeText : undefined;
      if (optionId !== undefined && !item.options.some((option) => option.id === optionId)) return json({ error: "invalid optionId" }, 400);
      if (body.freeText !== undefined && freeText === undefined) return json({ error: "freeText must be a string" }, 400);
      if (optionId === undefined && !freeText?.trim()) return json({ error: "optionId or freeText required" }, 400);
      const answer: EdgeAnswerInput = {
        answerId: await sha256(JSON.stringify([ledgerId, answerTokenHash, optionId ?? null, freeText ?? null])),
        ledgerId, ...(optionId !== undefined ? { optionId } : {}), ...(freeText !== undefined ? { freeText } : {}),
        answeredAt: new Date().toISOString(), answerTokenHash,
      };
      const outcome = applyEdgeAnswer(state, answer, Date.now());
      if (outcome.outcome === "stored") await this.save(outcome.state);
      return json({ ok: true, outcome: outcome.outcome, answerId: answer.answerId });
    }

    const answerPage = /^\/answer\/([^/]+)$/.exec(url.pathname);
    if (req.method === "GET" && answerPage) {
      const ledgerId = decodeURIComponent(answerPage[1]!);
      const item = state.questions.find((question) => question.ledgerId === ledgerId);
      const token = url.searchParams.get("t") ?? "";
      if (!item || !isQuestionOpen(item, Date.now())) return new Response("Question expired or withdrawn", { status: 410 });
      if (!timingSafeEqual(await sha256(token), item.answerTokenHash)) return new Response("Invalid answer token", { status: 403 });
      if (item.acceptedAnswerId) return new Response("Question already answered", { status: 410 });
      const escape = (value: string) => value.replace(/[&<>\"']/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;", "'": "&#39;" })[char]!);
      const options = item.options.map((option) => `<label><input type="radio" name="optionId" value="${escape(option.id)}"> ${escape(option.label)}</label>`).join("<br>");
      const page = `<!doctype html><meta name="viewport" content="width=device-width"><title>Answer question</title><h1>${escape(item.question)}</h1>${item.context ? `<p>${escape(item.context)}</p>` : ""}${options || '<textarea name="freeText" required></textarea>'}<button id="submit">Submit</button><p id="result"></p><script>document.querySelector('#submit').onclick=async()=>{const optionId=document.querySelector('[name=optionId]:checked')?.value;const freeText=document.querySelector('[name=freeText]')?.value;const r=await fetch('/api/needs-drew/${encodeURIComponent(ledgerId)}/answer',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({token:${JSON.stringify(token)},...(optionId?{optionId}:{}),...(freeText?{freeText}:{})})});document.querySelector('#result').textContent=r.ok?'Answer saved':'Answer was not accepted';};</script>`;
      return new Response(page, { headers: { "content-type": "text/html; charset=utf-8" } });
    }

    if (req.method === "GET" && url.pathname === "/answer") return new Response("not found", { status: 404 });

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
        pendingAnswers: state.pendingAnswers.length,
      });
    }

    return new Response("not found", { status: 404 });
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
