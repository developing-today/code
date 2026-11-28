// Session registry door (#12584; design §4.2). Open / resume /
// close / resolve over the shared auth plumbing, like every door —
// plus the registry itself (src/sessions.ts). The record is written
// against the caller's credential-resolved binding_id only: a body
// field or header asserting a tenancy or a session id is never
// honoured (the router already refuses a disagreeing tenancy header;
// body fields named binding_id / session_id on open are ignored —
// the service mints the id and owns the binding).
//
// resolve is the stamp path's question — "may this session be
// cited?" — and refuses unknown or closed ids with named errors
// (session_unknown / session_closed), so #12585's create path can
// never stamp a session the service did not open. resume is the
// CLI's re-attach: the same registry check, returning the record the
// CLI recovered its id for.

import type { AuthDeps, CallerContext } from "../auth.ts";
import { readJsonBody } from "../body.ts";
import { FailClosed } from "../errors.ts";
import type { RouteDef } from "../router.ts";
import {
  checkedSessionId,
  getOwnedSession,
  mintSessionId,
  requireOpenSession,
  sessionRecordJson,
  type SessionRecord,
  type SessionRegistry,
} from "../sessions.ts";

function registryFrom(deps: AuthDeps): SessionRegistry {
  if (!deps.sessions) {
    throw new FailClosed(
      503,
      "session_registry_not_configured",
      "this deployment has no session registry configured; failing closed",
    );
  }
  return deps.sessions;
}

function caller(ctx: CallerContext | null): CallerContext {
  if (!ctx) {
    throw new FailClosed(401, "missing_token", "session routes require an authenticated caller");
  }
  return ctx;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function jsonBody(req: Request): Promise<Record<string, unknown>> {
  const parsed: unknown = await readJsonBody(req);
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new FailClosed(400, "invalid_body", "request body must be a JSON object");
  }
  return parsed as Record<string, unknown>;
}

function optionalString(body: Record<string, unknown>, field: string): string | null {
  const value = body[field];
  if (value === undefined || value === null) return null;
  if (typeof value !== "string" || value.trim() === "") {
    throw new FailClosed(400, "invalid_body", `${field} must be a non-empty string when present`);
  }
  return value;
}

function laneStartIssues(body: Record<string, unknown>): string[] {
  const value = body["lane_start_issues"];
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value) || value.some((v) => typeof v !== "string" || v.trim() === "")) {
    throw new FailClosed(
      400,
      "invalid_body",
      "lane_start_issues must be an array of issue references (non-empty strings)",
    );
  }
  return [...(value as string[])];
}

function bodySessionId(body: Record<string, unknown>): string {
  if (!("session_id" in body)) {
    throw new FailClosed(400, "invalid_body", "request body must carry session_id");
  }
  return checkedSessionId(body["session_id"]);
}

function nowIso(deps: AuthDeps): string {
  return new Date(deps.now?.() ?? Date.now()).toISOString();
}

async function openSession(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  const me = caller(ctx);
  const registry = registryFrom(deps);
  const body = await jsonBody(req);

  // An agent credential's lane is the ceiling of what the caller may
  // assert (design §4.1): a declared lane must match it; an omitted
  // lane inherits it. Other caller kinds declare freely.
  let lane = optionalString(body, "lane");
  if (me.kind === "agent" && me.lane) {
    if (lane !== null && lane !== me.lane) {
      throw new FailClosed(403, "lane_beyond_credential", "declared lane differs from the agent credential's lane");
    }
    lane = me.lane;
  }

  // The service mints the id — never the client. Mint until unique
  // (a collision is a 128-bit accident, but the registry is the
  // authority, so ask it).
  let sessionId = mintSessionId();
  while ((await registry.get(sessionId)) !== null) {
    sessionId = mintSessionId();
  }

  const record: SessionRecord = {
    sessionId,
    bindingId: me.bindingId,
    lane,
    model: optionalString(body, "model"),
    openedAt: nowIso(deps),
    closedAt: null,
    laneStartIssues: laneStartIssues(body),
  };
  await registry.insert(record);
  return json(sessionRecordJson(record), 201);
}

async function resumeSession(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  const me = caller(ctx);
  const registry = registryFrom(deps);
  const sessionId = bodySessionId(await jsonBody(req));
  const record = await requireOpenSession(registry, me.bindingId, sessionId);
  return json(sessionRecordJson(record));
}

async function resolveSession(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  const me = caller(ctx);
  const registry = registryFrom(deps);
  const sessionId = bodySessionId(await jsonBody(req));
  const record = await requireOpenSession(registry, me.bindingId, sessionId);
  return json(sessionRecordJson(record));
}

async function closeSession(req: Request, ctx: CallerContext | null, deps: AuthDeps): Promise<Response> {
  const me = caller(ctx);
  const registry = registryFrom(deps);
  const sessionId = bodySessionId(await jsonBody(req));
  const record = await getOwnedSession(registry, me.bindingId, sessionId);
  if (record.closedAt !== null) {
    throw new FailClosed(409, "session_closed", `session ${sessionId} is already closed`);
  }
  const closed = await registry.markClosed(sessionId, nowIso(deps));
  if (!closed) {
    throw new FailClosed(404, "session_unknown", "session id was not minted by this service for this account");
  }
  return json(sessionRecordJson(closed));
}

export const routes: RouteDef[] = [
  { method: "POST", path: "/v1/sessions", door: "sessions", handler: openSession },
  { method: "POST", path: "/v1/sessions/resume", door: "sessions", handler: resumeSession },
  { method: "POST", path: "/v1/sessions/resolve", door: "sessions", handler: resolveSession },
  { method: "POST", path: "/v1/sessions/close", door: "sessions", handler: closeSession },
];
