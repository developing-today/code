// Bridge door (#14373; design §4.8). The PM stack's hosted path to
// the message bridge, over the same service binding and PM
// credential the console's edge Worker uses, so the PM service API
// and the console share one send path:
//
//   POST /v1/bridge/send     {to, body, kind?}: one message or
//                            command to agents, a group, or all,
//                            delivered by the bridge as
//                            service:pm-stack — no machine relays it.
//   GET  /v1/bridge/agents   the registered agents, read at the edge.
//   GET  /v1/bridge/messages ?limit=&order=: the fleet message log.
//
// The engine (the binding client, the request shapes) is
// src/bridge.ts. Every handler refuses 503 before parsing when the
// binding or the credential is unconfigured, like every door's
// missing dependency.

import type { AuthDeps, CallerContext } from "../auth.ts";
import { bridgeSurfaceFrom, parseBridgeMessagesQuery, parseBridgeSendRequest } from "../bridge.ts";
import { FailClosed } from "../errors.ts";
import type { RouteDef } from "../router.ts";

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function caller(ctx: CallerContext | null): CallerContext {
  if (!ctx) {
    throw new FailClosed(401, "missing_token", "bridge routes require an authenticated caller");
  }
  return ctx;
}

async function jsonBody(req: Request): Promise<unknown> {
  try {
    return await req.json();
  } catch {
    throw new FailClosed(400, "invalid_body", "request body is not valid JSON");
  }
}

async function send(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  caller(ctx);
  const bridge = bridgeSurfaceFrom(deps);
  const input = parseBridgeSendRequest(await jsonBody(req));
  return json(await bridge.send(input), 201);
}

async function agents(_req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  caller(ctx);
  return json(await bridgeSurfaceFrom(deps).agents());
}

async function messages(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  caller(ctx);
  const bridge = bridgeSurfaceFrom(deps);
  return json(await bridge.messages(parseBridgeMessagesQuery(new URL(req.url))));
}

export const routes: RouteDef[] = [
  { method: "POST", path: "/v1/bridge/send", door: "bridge", handler: send },
  { method: "GET", path: "/v1/bridge/agents", door: "bridge", handler: agents },
  { method: "GET", path: "/v1/bridge/messages", door: "bridge", handler: messages },
];
