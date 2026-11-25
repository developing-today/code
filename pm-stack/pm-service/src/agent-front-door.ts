/**
 * Surface C agent front door, slice H4 (#14262, child of #9086).
 *
 * Narrow acceptance seam over the shared H1 front door
 * (`./front-door.ts`, design §4.1): signature validation stays in H1;
 * this module adds the H4 policy (§4.5) for surface C agent tokens —
 * `role=agent` exactly, one tenant (`scope === binding_id`, never
 * `'*'`), audience-bound, 15-minute token lifetime, live
 * credential-metadata match with a 30-day maximum, revocation by
 * `jti` read live every request (active → revoked denies the very
 * next request), and an audit event carrying `jti` plus the owning
 * `account_id`/`binding_id` on every accepted request (§4.8).
 */
import { resolveFrontDoor, type FrontDoorDeps, type FrontDoorResult } from "./front-door.ts";

export interface AgentCredentialMetadata {
  jti: string;
  accountId: string;
  bindingId: string;
  tenantId: string;
  audience: string;
  role: "agent";
  issuedAt: number;
  expiresAt: number;
  revokedAt: number | null;
}

export interface AgentAudit {
  jti: string;
  accountId: string;
  bindingId: string;
  tenantId: string;
}

export interface AgentFrontDoorDeps extends FrontDoorDeps {
  loadCredential: (jti: string) => Promise<AgentCredentialMetadata | null>;
  audit: (actor: AgentAudit) => Promise<void>;
}

export async function resolveAgentFrontDoor(
  token: string,
  tenantId: string,
  deps: AgentFrontDoorDeps,
): Promise<FrontDoorResult> {
  try {
    // Read one clock value for a consistent request-wide policy decision.
    const now = Math.floor((deps.now ?? (() => Date.now() / 1000))());
    let tokenIssuedAt = 0;
    let tokenExpiresAt = 0;
    const result = await resolveFrontDoor(token, {
      ...deps,
      now: () => now,
      validateClaims: (claims) => {
        if (deps.validateClaims !== undefined && !deps.validateClaims(claims)) return false;
        if (
          !Number.isSafeInteger(now) ||
          claims.aud !== deps.audience || claims.scope !== claims.binding_id ||
          !Array.isArray(claims.roles) || claims.roles.length !== 1 || claims.roles[0] !== "agent" ||
          typeof claims.iat !== "number" || !Number.isSafeInteger(claims.iat) || claims.iat > now ||
          typeof claims.exp !== "number" || !Number.isSafeInteger(claims.exp) ||
          claims.exp <= now || claims.exp - claims.iat > 15 * 60
        ) return false;
        tokenIssuedAt = claims.iat;
        tokenExpiresAt = claims.exp;
        return true;
      },
    });
    if (!result.ok) return result;
    const principal = result.principal;
    if (tenantId !== principal.bindingId || tenantId === "*" || !principal.roles.includes("agent")) {
      return { ok: false, reason: "invalid_agent_credential" };
    }
    // Never cache: a jti revoked between requests must deny the next one.
    const metadata = await deps.loadCredential(principal.tokenId);
    if (
      metadata === null || metadata.jti !== principal.tokenId ||
      metadata.accountId !== principal.accountId || metadata.bindingId !== principal.bindingId ||
      metadata.tenantId !== principal.bindingId || metadata.tenantId !== tenantId ||
      metadata.audience !== deps.audience || metadata.role !== "agent" ||
      !Number.isSafeInteger(metadata.issuedAt) || !Number.isSafeInteger(metadata.expiresAt) ||
      metadata.issuedAt > tokenIssuedAt || metadata.expiresAt < tokenExpiresAt ||
      metadata.expiresAt <= metadata.issuedAt || metadata.expiresAt - metadata.issuedAt > 30 * 86400 ||
      metadata.revokedAt !== null
    ) return { ok: false, reason: "invalid_agent_credential" };

    // Await attribution before returning authority; audit outage fails closed.
    await deps.audit({ jti: principal.tokenId, accountId: principal.accountId, bindingId: principal.bindingId, tenantId });
    return { ok: true, principal: { ...principal, roles: ["agent"] } };
  } catch {
    return { ok: false, reason: "invalid_agent_credential" };
  }
}
