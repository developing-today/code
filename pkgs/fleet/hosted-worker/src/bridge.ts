// Bridge door engine (#14373; design §4.8): the hosted door's path
// to the message bridge, shared with the PM console's edge Worker.
//
// The PM service API (this door) and the console's edge both speak to
// the bridge Worker over a service binding, presenting the PM stack's
// own bridge credential — minted on the bridge for the purpose, held
// as a Worker secret by name (BRIDGE_PM_TOKEN), never in code or
// config. Sends go out as service:pm-stack, so the bridge's log tells
// PM-originated rows apart from owner-sent (from_addr null) and
// agent-sent (agent:<name>) ones; the agents list and the message log
// are read with the credential's read scope. Revoking the credential
// on the bridge stops this door's sends at the bridge: the door
// answers 502 naming the refusal, and nothing falls back to a
// machine's fleet binary — the laptop-local services keep their own
// route, untouched by this door.

import type { AuthDeps } from "./auth.ts";
import { FailClosed } from "./errors.ts";

/** The bridge surface this door calls: the three operations the PM
 * stack's hosted path uses. Behind an interface so tests stand in
 * their own bridge, as with every door's dependency. */
export interface BridgeSurface {
  /** One message or command to agents, a group, or all. Returns the
   * bridge's delivery record (the rows it stored). */
  send(input: BridgeSendInput): Promise<unknown>;
  /** The registered agents, their groups, last seen, unread counts. */
  agents(): Promise<unknown>;
  /** The fleet message log, newest or oldest first. */
  messages(query: BridgeMessagesQuery): Promise<unknown>;
}

export interface BridgeSendInput {
  to: string[];
  body: string;
  kind: "message" | "command";
}

export interface BridgeMessagesQuery {
  limit?: number;
  order?: "asc" | "desc";
}

/** The name the PM stack's bridge credential is bound to: the bridge
 * fixes a credential's sends to its own name, and this door's sends
 * go out under it, as service:pm-stack. */
export const BRIDGE_AGENT = "pm-stack";
/** Where the bridge is asked, over its service binding: the binding
 * answers for the bridge Worker whatever host the address names. */
const BRIDGE_BASE = "https://bridge.internal";

/** A service binding's fetch shape (a Fetcher in the Workers
 * runtime; a stand-in in tests). */
export interface BridgeBinding {
  fetch(request: Request): Promise<Response>;
}

/** The bridge over its service binding, presenting the PM credential
 * in the path (the bridge's own way: /fleet/<credential>/...). */
export class BindingBridgeSurface implements BridgeSurface {
  constructor(
    private binding: BridgeBinding,
    private token: string,
  ) {}

  private async call(method: string, path: string, body?: unknown): Promise<unknown> {
    let res: Response;
    try {
      res = await this.binding.fetch(
        new Request(`${BRIDGE_BASE}${path}`, {
          method,
          headers: {
            "user-agent": "fleet-hosted",
            ...(body === undefined ? {} : { "content-type": "application/json" }),
          },
          ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        }),
      );
    } catch (e) {
      throw new FailClosed(502, "bridge_unreachable", `the bridge could not be reached: ${(e as Error).message}`);
    }
    if (res.status === 404) {
      // The bridge's one answer to a token it does not know: the PM
      // credential is unknown or revoked. Said plainly, because that
      // is the error the operator acts on (mint a new one).
      throw new FailClosed(
        502,
        "bridge_credential_refused",
        "the bridge turned the PM credential away (unknown or revoked); nothing was sent",
      );
    }
    if (!res.ok) {
      throw new FailClosed(502, "bridge_failed", `the bridge answered http ${res.status}`);
    }
    try {
      return await res.json();
    } catch {
      throw new FailClosed(502, "bridge_failed", "the bridge answered with no JSON body");
    }
  }

  async send(input: BridgeSendInput): Promise<unknown> {
    return this.call("POST", `/fleet/${this.token}/send`, {
      agent: BRIDGE_AGENT,
      to: input.to,
      body: input.body,
      kind: input.kind,
    });
  }

  async agents(): Promise<unknown> {
    return this.call("GET", `/fleet/${this.token}/agents`);
  }

  async messages(query: BridgeMessagesQuery): Promise<unknown> {
    const params = new URLSearchParams();
    if (query.limit !== undefined) params.set("limit", String(query.limit));
    if (query.order !== undefined) params.set("order", query.order);
    const qs = params.toString();
    return this.call("GET", `/fleet/${this.token}/messages${qs === "" ? "" : `?${qs}`}`);
  }
}

/** The door's bridge, or the door fails closed: no binding or no
 * credential configured is a 503, like every door's missing
 * dependency. */
export function bridgeSurfaceFrom(deps: AuthDeps): BridgeSurface {
  if (!deps.bridge) {
    throw new FailClosed(
      503,
      "bridge_unconfigured",
      "the bridge door is not configured (no BRIDGE service binding or BRIDGE_PM_TOKEN)",
    );
  }
  return deps.bridge;
}

const TARGET = /^(all|(agent:|group:)?[A-Za-z0-9][A-Za-z0-9._-]{0,63})$/;

/** A send as the console's own validation shapes it: one to fifty
 * targets, a text of at most 16,000 characters, a kind of message
 * (the default) or command. */
export function parseBridgeSendRequest(raw: unknown): BridgeSendInput {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    throw new FailClosed(400, "invalid_body", "send body must be a JSON object");
  }
  const { to, body, kind } = raw as { to?: unknown; body?: unknown; kind?: unknown };
  if (!Array.isArray(to) || to.length === 0 || to.length > 50) {
    throw new FailClosed(400, "invalid_body", "to: one to fifty targets");
  }
  for (const t of to) {
    if (typeof t !== "string" || !TARGET.test(t)) {
      throw new FailClosed(400, "invalid_body", `to: not a target: ${JSON.stringify(t)?.slice(0, 80)}`);
    }
  }
  if (typeof body !== "string" || body.trim() === "" || body.length > 16_000) {
    throw new FailClosed(400, "invalid_body", "body: required, at most 16000 characters");
  }
  if (kind !== undefined && kind !== "message" && kind !== "command") {
    throw new FailClosed(400, "invalid_body", "kind: message or command");
  }
  return { to: to as string[], body, kind: kind ?? "message" };
}

/** The log read's query: a limit of at most 200 (the bridge's own
 * cap), an order of asc (its default) or desc. */
export function parseBridgeMessagesQuery(url: URL): BridgeMessagesQuery {
  const out: BridgeMessagesQuery = {};
  const limit = url.searchParams.get("limit");
  if (limit !== null && limit !== "") {
    const n = Number(limit);
    if (!Number.isInteger(n) || n < 1) throw new FailClosed(400, "invalid_query", "limit: a positive whole number");
    out.limit = Math.min(n, 200);
  }
  const order = url.searchParams.get("order");
  if (order !== null && order !== "") {
    if (order !== "asc" && order !== "desc") throw new FailClosed(400, "invalid_query", "order: asc or desc");
    out.order = order;
  }
  return out;
}
