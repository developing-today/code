// decision-events Worker (#9335 Track B): the Cloudflare half of the
// decision-set event pipeline.
//
//   POST /events  (alias POST /webhook)
//     Authenticated webhook (shared secret, Worker secret
//     WEBHOOK_SECRET — vaulted in Bitwarden by name
//     `cloudflare.decision-events.webhook-secret`, set with
//     `wrangler secret put`; never in this repo). Accepts a batch of
//     decision-set events ({events: [...]}, a bare array, or one
//     event), stores them in D1 (INSERT OR IGNORE on event_id —
//     redelivery is a no-op), then hands each set's NEW events to that
//     set's Durable Object for per-set fan-out.
//   GET /sets/<setId>/events?limit=<n>&after=<cursor>
//     Authenticated read-back of a set's events from D1, in
//     (ts, event_id) order. `after` is the composite cursor the
//     previous page returned as `nextAfter` (<ts>|<eventId>); a bare
//     <ts> is still accepted with its legacy meaning (ts > after).
//     The response's `nextAfter` is the cursor for the following
//     page, or null when the page is empty.
//   GET /sets/<setId>/state
//     Authenticated: the set's Durable Object state (events fanned
//     out, last event, live subscriber count) merged with D1's count.
//   GET /sets/<setId>/stream
//     Authenticated WebSocket upgrade, routed to the set's Durable
//     Object: subscribers receive every new event for that set as it
//     lands ({type:"decision-events", setId, events:[...]}).
//   GET /health — unauthenticated liveness only.
//
// The Durable Object is keyed by setId (idFromName(setId)): one object
// per decision set coordinates that set's fan-out — it counts the
// events it has broadcast, remembers the latest, and pushes new
// batches to the set's WebSocket subscribers (hibernatable sockets,
// so idle sets cost nothing). D1 stays the durable store; the object
// holds coordination state only.
import {
  encodeEventsCursor,
  groupBySet,
  isAuthorized,
  parseEventBatch,
  parseEventsCursor,
  type DecisionSetEvent,
} from "./events.ts";

export interface Env {
  DB: D1Database;
  SET_EVENTS: DurableObjectNamespace;
  WEBHOOK_SECRET?: string;
}

interface SetState {
  setId: string;
  fannedOut: number;
  lastEventId: string | null;
  lastType: string | null;
  lastTs: string | null;
  updatedAt: string | null;
}

const emptyState = (setId: string): SetState => ({
  setId,
  fannedOut: 0,
  lastEventId: null,
  lastType: null,
  lastTs: null,
  updatedAt: null,
});

export class DecisionSetEvents {
  private state: DurableObjectState;
  private setId = "";

  constructor(state: DurableObjectState) {
    this.state = state;
  }

  private async loadState(): Promise<SetState> {
    const stored = await this.state.storage.get<SetState>("state");
    if (stored) {
      this.setId = stored.setId;
      return stored;
    }
    return emptyState(this.setId);
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname === "/subscribe" && request.headers.get("Upgrade") === "websocket") {
      const pair = new WebSocketPair();
      const [client, server] = Object.values(pair) as [WebSocket, WebSocket];
      this.state.acceptWebSocket(server);
      const state = await this.loadState();
      server.send(JSON.stringify({ type: "decision-set-state", ...state }));
      return new Response(null, { status: 101, webSocket: client });
    }
    if (url.pathname === "/ingest" && request.method === "POST") {
      const body = (await request.json().catch(() => null)) as { setId?: string; events?: DecisionSetEvent[] } | null;
      if (!body || typeof body.setId !== "string" || !Array.isArray(body.events)) {
        return Response.json({ error: "expected {setId, events}" }, { status: 400 });
      }
      this.setId = body.setId;
      const state = await this.loadState();
      state.setId = body.setId;
      for (const e of body.events) {
        state.fannedOut += 1;
        state.lastEventId = e.eventId;
        state.lastType = e.type;
        state.lastTs = e.ts;
      }
      state.updatedAt = new Date().toISOString();
      await this.state.storage.put("state", state);
      const message = JSON.stringify({ type: "decision-events", setId: body.setId, events: body.events });
      let delivered = 0;
      for (const ws of this.state.getWebSockets()) {
        try {
          ws.send(message);
          delivered += 1;
        } catch {
          // A dead socket drops out of getWebSockets on its own.
        }
      }
      return Response.json({ ...state, subscribers: this.state.getWebSockets().length, delivered });
    }
    if (url.pathname === "/state") {
      const state = await this.loadState();
      return Response.json({ ...state, subscribers: this.state.getWebSockets().length });
    }
    return Response.json({ error: "not found" }, { status: 404 });
  }

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): Promise<void> {
    if (typeof message === "string" && message.trim() === "ping") {
      ws.send(JSON.stringify({ type: "pong", at: new Date().toISOString() }));
      return;
    }
    if (typeof message === "string" && message.trim() === "state") {
      const state = await this.loadState();
      ws.send(JSON.stringify({ type: "decision-set-state", ...state }));
    }
  }

  async webSocketClose(ws: WebSocket): Promise<void> {
    try {
      ws.close();
    } catch {
      // Already closed.
    }
  }
}

function jsonError(status: number, error: string): Response {
  return Response.json({ error }, { status });
}

async function handlePostEvents(request: Request, env: Env): Promise<Response> {
  if (!env.WEBHOOK_SECRET) return jsonError(503, "webhook secret not configured");
  if (!isAuthorized(request.headers, env.WEBHOOK_SECRET)) return jsonError(401, "unauthorized");
  const body: unknown = await request.json().catch(() => null);
  if (body === null) return jsonError(400, "body must be JSON");
  const parsed = parseEventBatch(body);
  if (!parsed.ok) return jsonError(400, parsed.error);
  const { events, errors } = parsed.value;
  if (events.length === 0) {
    return Response.json({ accepted: 0, inserted: 0, duplicates: 0, errors });
  }

  // Store first (D1, idempotent), fan out only what was actually new.
  const statements = events.map((e) =>
    env.DB.prepare(
      "INSERT OR IGNORE INTO decision_set_events (event_id, set_id, question_ref, type, payload, ts) VALUES (?, ?, ?, ?, ?, ?)",
    ).bind(e.eventId, e.setId, e.questionRef, e.type, JSON.stringify(e.payload), e.ts),
  );
  const results = await env.DB.batch(statements);
  const insertedEvents: DecisionSetEvent[] = [];
  let inserted = 0;
  results.forEach((r, i) => {
    const changes = r.meta?.changes ?? 0;
    inserted += changes;
    if (changes > 0) insertedEvents.push(events[i] as DecisionSetEvent);
  });

  const fanout: Record<string, unknown> = {};
  for (const [setId, setEvents] of groupBySet(insertedEvents)) {
    const stub = env.SET_EVENTS.get(env.SET_EVENTS.idFromName(setId));
    const res = await stub.fetch("https://do/ingest", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ setId, events: setEvents }),
    });
    fanout[setId] = res.ok ? await res.json().catch(() => null) : { error: `do ingest HTTP ${res.status}` };
  }

  return Response.json({
    accepted: events.length,
    inserted,
    duplicates: events.length - inserted,
    errors,
    fanout,
  });
}

async function handleGetEvents(url: URL, env: Env, setId: string): Promise<Response> {
  const limitRaw = Number(url.searchParams.get("limit") ?? "200");
  const limit = Number.isFinite(limitRaw) ? Math.min(Math.max(Math.trunc(limitRaw), 1), 1000) : 200;
  const afterRaw = url.searchParams.get("after");
  const select = "SELECT event_id, set_id, question_ref, type, payload, ts FROM decision_set_events WHERE set_id = ?";
  let query = `${select} ORDER BY ts, event_id LIMIT ?`;
  let bound = env.DB.prepare(query).bind(setId, limit);
  if (afterRaw) {
    const parsed = parseEventsCursor(afterRaw);
    if (!parsed.ok) return jsonError(400, parsed.error);
    const { ts, eventId } = parsed.value;
    if (eventId !== null) {
      // Composite cursor (#9614): resume strictly after the (ts,
      // event_id) pair, so events sharing the page-boundary ts are
      // not skipped.
      query = `${select} AND (ts > ? OR (ts = ? AND event_id > ?)) ORDER BY ts, event_id LIMIT ?`;
      bound = env.DB.prepare(query).bind(setId, ts, ts, eventId, limit);
    } else {
      // Legacy bare-ts cursor: original meaning, ts strictly after.
      query = `${select} AND ts > ? ORDER BY ts, event_id LIMIT ?`;
      bound = env.DB.prepare(query).bind(setId, ts, limit);
    }
  }
  const { results } = await bound.all<Record<string, unknown>>();
  const events = (results ?? []).map((row) => ({
    eventId: row.event_id,
    setId: row.set_id,
    questionRef: row.question_ref,
    type: row.type,
    payload: typeof row.payload === "string" ? JSON.parse(row.payload) : row.payload,
    ts: row.ts,
  }));
  const last = events[events.length - 1];
  const nextAfter = last ? encodeEventsCursor({ ts: last.ts as string, eventId: last.eventId as string }) : null;
  return Response.json({ setId, count: events.length, events, nextAfter });
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname === "/health") {
      return Response.json({ ok: true, service: "decision-events", webhookConfigured: Boolean(env.WEBHOOK_SECRET) });
    }
    if ((url.pathname === "/events" || url.pathname === "/webhook") && request.method === "POST") {
      return handlePostEvents(request, env);
    }
    const setMatch = /^\/sets\/([^/]+)\/(events|state|stream)$/.exec(url.pathname);
    if (setMatch && request.method === "GET") {
      if (!env.WEBHOOK_SECRET) return jsonError(503, "webhook secret not configured");
      if (!isAuthorized(request.headers, env.WEBHOOK_SECRET)) return jsonError(401, "unauthorized");
      const setId = decodeURIComponent(setMatch[1] as string);
      const kind = setMatch[2] as string;
      if (kind === "events") return handleGetEvents(url, env, setId);
      const stub = env.SET_EVENTS.get(env.SET_EVENTS.idFromName(setId));
      if (kind === "stream") {
        if (request.headers.get("Upgrade") !== "websocket") return jsonError(426, "websocket upgrade required");
        return stub.fetch("https://do/subscribe", { headers: request.headers });
      }
      const doState = await stub.fetch("https://do/state").then((r) => r.json().catch(() => null));
      const countRow = await env.DB.prepare("SELECT COUNT(*) AS n FROM decision_set_events WHERE set_id = ?")
        .bind(setId)
        .first<{ n: number }>()
        .catch(() => null);
      return Response.json({ setId, d1Events: countRow?.n ?? null, object: doState });
    }
    return jsonError(404, "not found");
  },
};
