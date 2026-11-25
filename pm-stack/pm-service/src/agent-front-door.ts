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
  return resolveFrontDoor(token, deps);
}
