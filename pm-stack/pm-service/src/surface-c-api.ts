/**
 * Surface C HTTP integration, slice H4 (#14262, child of #9086,
 * design §4.2 surface C + §4.5).
 *
 * Routes:
 *
 *   - `POST /api/agent-credentials` — issuance from the H2 owner
 *     session. The session is resolved by the injected
 *     `resolveOwnerSession` (H1 front door + H3 cookie in the wired
 *     service; a stub in tests). Body: `{tenantId, audience,
 *     ttlSec?}`. The 201 body carries the credential metadata and
 *     the secret — the one and only response that ever shows it.
 *   - `DELETE /api/agent-credentials/<jti>` — owner-scoped
 *     revocation by `jti`; effective within one request because
 *     every consumer reads the metadata store live.
 *   - `POST /oauth/token` — the client-credentials exchange
 *     (`grant_type=client_credentials`, `client_id=<jti>`,
 *     `client_secret`, `audience`): an audience-bound Bearer token,
 *     15-minute TTL, no refresh token.
 *   - `GET /api/agent/whoami` — a protected surface C resource: the
 *     Bearer token is accepted only through
 *     `resolveAgentFrontDoor` (live credential metadata, live
 *     roles, awaited audit attribution), with the tenant named by
 *     the `x-tenant-id` header. This is the end-to-end path the H4
 *     acceptance exercises: issue → exchange → act → revoke → the
 *     very next request denies.
 */
import { resolveAgentFrontDoor, type AgentAudit, type AgentFrontDoorDeps } from "./agent-front-door.ts";
import type { AgentCredentialStore } from "./agent-credential-store.ts";
import {
  exchangeClientCredentials,
  issueAgentCredential,
  revokeAgentCredential,
  type AccessTokenClaims,
  type AgentCredentialDeps,
} from "./agent-credentials.ts";
import type { ResolvedPrincipal } from "./front-door.ts";

export interface SurfaceCDeps {
  store: AgentCredentialStore;
  /** H2 owner session resolution for the minting routes. */
  resolveOwnerSession: (req: Request) => Promise<ResolvedPrincipal | null>;
  /** H1 deps minus the credential loader, which the store supplies. */
  frontDoor: Omit<AgentFrontDoorDeps, "loadCredential">;
  mintAccessToken: (claims: AccessTokenClaims) => Promise<string>;
  now?: () => number;
  generateJti?: () => string;
  generateSecret?: () => string;
  auditLifecycle?: AgentCredentialDeps["auditLifecycle"];
}

const json = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

async function readBody(req: Request): Promise<Record<string, unknown>> {
  const contentType = req.headers.get("content-type") ?? "";
  try {
    if (contentType.includes("application/x-www-form-urlencoded")) {
      const form = new URLSearchParams(await req.text());
      return Object.fromEntries(form.entries());
    }
    const parsed: unknown = await req.json();
    if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed))
      return parsed as Record<string, unknown>;
  } catch { /* fall through to empty */ }
  return {};
}

const asString = (value: unknown): string => (typeof value === "string" ? value : "");

export function createSurfaceCHandler(deps: SurfaceCDeps): (req: Request) => Promise<Response> {
  const credentialDeps = {
    store: deps.store,
    now: deps.now,
    generateJti: deps.generateJti,
    generateSecret: deps.generateSecret,
    auditLifecycle: deps.auditLifecycle,
  };
  return async (req: Request): Promise<Response> => {
    const url = new URL(req.url);
    const path = url.pathname;

    if (req.method === "POST" && path === "/api/agent-credentials") {
      const session = await deps.resolveOwnerSession(req);
      if (session === null) return json({ error: "unauthenticated" }, 401);
      const body = await readBody(req);
      const result = await issueAgentCredential(
        session,
        {
          tenantId: asString(body.tenantId),
          audience: asString(body.audience),
          ttlSec: typeof body.ttlSec === "number" ? body.ttlSec : undefined,
        },
        credentialDeps,
      );
      if (!result.ok) return json({ error: result.reason }, 403);
      // The secret appears in this response only — shown exactly once.
      return json({ credential: result.credential, secret: result.secret }, 201);
    }

    const revokeMatch = /^\/api\/agent-credentials\/(.+)$/.exec(path);
    if (req.method === "DELETE" && revokeMatch) {
      const session = await deps.resolveOwnerSession(req);
      if (session === null) return json({ error: "unauthenticated" }, 401);
      const jti = decodeURIComponent(revokeMatch[1] as string);
      const result = await revokeAgentCredential(session, jti, credentialDeps);
      if (!result.ok) return json({ error: result.reason }, result.reason === "not_found" ? 404 : 403);
      return json({ revoked: true, jti });
    }

    if (req.method === "POST" && path === "/oauth/token") {
      const body = await readBody(req);
      if (asString(body.grant_type) !== "client_credentials")
        return json({ error: "unsupported_grant_type" }, 400);
      const result = await exchangeClientCredentials(
        {
          credentialId: asString(body.client_id),
          secret: asString(body.client_secret),
          audience: asString(body.audience),
        },
        { ...credentialDeps, mintAccessToken: deps.mintAccessToken },
      );
      if (!result.ok) return json({ error: result.reason }, 401);
      return json({
        access_token: result.accessToken,
        token_type: result.tokenType,
        expires_in: result.expiresIn,
      });
    }

    if (req.method === "GET" && path === "/api/agent/whoami") {
      const authorization = req.headers.get("authorization") ?? "";
      const token = authorization.startsWith("Bearer ") ? authorization.slice("Bearer ".length) : "";
      const tenantId = req.headers.get("x-tenant-id") ?? "";
      if (!token || !tenantId) return json({ error: "unauthenticated" }, 401);
      const audits: AgentAudit[] = [];
      const result = await resolveAgentFrontDoor(token, tenantId, {
        ...deps.frontDoor,
        loadCredential: (jti) => deps.store.getByJti(jti),
        audit: async (actor) => {
          audits.push(actor);
          await deps.frontDoor.audit(actor);
        },
      });
      if (!result.ok) return json({ error: result.reason }, 401);
      return json({ principal: result.principal, audit: audits[0] ?? null });
    }

    return json({ error: "not_found" }, 404);
  };
}
