// Queue door (#12589; design §6): the broker protocol of the one
// fleet queue over hosted transports — a second, peer front door
// onto the existing fleet-queue backend, not a second queue. One
// POST route per broker verb; the response body IS the broker
// envelope ({ok:true, ...} / {ok:false, exit, error}) at HTTP 200
// for every verb outcome, exactly as the native broker answers —
// the envelope is the protocol, and callers branch on it, not on
// the transport status. Plumbing refusals keep this worker's idiom
// instead: a malformed body is a named 400, an unconfigured store
// a 503, before any verb runs.
//
// The deliberate divergence (design §6): this door requires
// authentication and refuses anonymous claims — the router's
// pipeline has already authenticated the caller before any handler
// here runs — while the native door keeps its local trust model.
// Claim provenance is stamped server-side: the caller's
// credential-resolved binding_id rides in every event detail the
// engine writes, and an agent caller's lane/session assertions are
// capped by its credential (design §4.1), like the session door.
//
// The fallback-claim protocol (the queue backend is down; an
// issue's comments carry the claim) is served under
// /v1/queue/fallback/* over the GitHub comment surface, with the
// native CLI's result JSON: success at 200, state refusals at 409,
// usage refusals at 400, transport failures at 502.

import type { AuthDeps, CallerContext } from "../auth.ts";
import { FailClosed } from "../errors.ts";
import { NO_DEP_PROBE, type QueueStore } from "../queue-store.ts";
import { executeQueueVerb, QUEUE_VERBS, type QueueEnv } from "../queue.ts";
import {
  fallbackClaim,
  fallbackHeartbeat,
  fallbackRelease,
  fallbackStatus,
  type FallbackCtx,
  type FallbackResult,
} from "../queue-fallback.ts";
import type { JsonObject } from "../queue-validate.ts";
import type { RouteDef } from "../router.ts";

function storeFrom(deps: AuthDeps): QueueStore {
  if (!deps.queueStore) {
    throw new FailClosed(
      503,
      "queue_store_not_configured",
      "this deployment has no queue store configured; failing closed",
    );
  }
  return deps.queueStore;
}

function caller(ctx: CallerContext | null): CallerContext {
  if (!ctx) {
    throw new FailClosed(401, "missing_token", "queue routes require an authenticated caller");
  }
  return ctx;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function jsonBody(req: Request): Promise<JsonObject> {
  let parsed: unknown;
  try {
    parsed = await req.json();
  } catch {
    throw new FailClosed(400, "invalid_body", "request body is not valid JSON");
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new FailClosed(400, "invalid_body", "request body must be a JSON object");
  }
  return parsed as JsonObject;
}

/** Agent credentials cap what the caller may assert (design §4.1):
 * a queue request naming a lane or session other than the
 * credential's is refused, never silently rewritten. */
function assertAgentCeiling(me: CallerContext, body: JsonObject): void {
  if (me.kind !== "agent") return;
  const lane = body["lane"];
  if (typeof lane === "string" && me.lane && lane !== me.lane) {
    throw new FailClosed(
      403,
      "lane_beyond_credential",
      "request lane differs from the agent credential's lane",
    );
  }
  const sessionId = body["session_id"];
  if (typeof sessionId === "string" && me.sessionId && sessionId !== me.sessionId) {
    throw new FailClosed(
      403,
      "session_beyond_credential",
      "request session differs from the agent credential's session",
    );
  }
}

function verbHandler(verb: string): RouteDef["handler"] {
  return async (req, ctx, deps) => {
    const me = caller(ctx);
    const store = storeFrom(deps);
    const body = await jsonBody(req);
    assertAgentCeiling(me, body);
    const env: QueueEnv = {
      nowEpoch: () => Math.floor((deps.now?.() ?? Date.now()) / 1000),
      depProbe: deps.queueDepProbe ?? NO_DEP_PROBE,
      bindingId: me.bindingId,
    };
    return json(executeQueueVerb(store, verb, body, env));
  };
}

function fallbackHandler(
  run: (ctx: FallbackCtx, req: JsonObject) => Promise<FallbackResult>,
): RouteDef["handler"] {
  return async (req, ctx, deps) => {
    caller(ctx);
    if (!deps.githubComments) {
      throw new FailClosed(
        503,
        "github_comments_not_configured",
        "this deployment has no GitHub comment surface configured; failing closed",
      );
    }
    const body = await jsonBody(req);
    const result = await run(
      {
        surface: deps.githubComments,
        nowEpoch: () => Math.floor((deps.now?.() ?? Date.now()) / 1000),
      },
      body,
    );
    return json(result.body, result.status);
  };
}

export const routes: RouteDef[] = [
  ...QUEUE_VERBS.map((verb) => ({
    method: "POST",
    path: `/v1/queue/${verb}`,
    door: "queue",
    handler: verbHandler(verb),
  })),
  {
    method: "POST",
    path: "/v1/queue/fallback/status",
    door: "queue",
    handler: fallbackHandler(fallbackStatus),
  },
  {
    method: "POST",
    path: "/v1/queue/fallback/claim",
    door: "queue",
    handler: fallbackHandler(fallbackClaim),
  },
  {
    method: "POST",
    path: "/v1/queue/fallback/heartbeat",
    door: "queue",
    handler: fallbackHandler(fallbackHeartbeat),
  },
  {
    method: "POST",
    path: "/v1/queue/fallback/release",
    door: "queue",
    handler: fallbackHandler(fallbackRelease),
  },
];
