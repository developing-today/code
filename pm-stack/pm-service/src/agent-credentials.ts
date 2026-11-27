/**
 * Agent credential issuance, revocation, and the surface C
 * client-credentials exchange, slice H4 (#14262, child of #9086,
 * design §§4.2/4.5).
 *
 * Issuance is minted from the H2 owner session: the caller passes
 * the front-door-resolved principal of an authenticated session and
 * this module enforces the §4.5 minting rules —
 *
 *   - only `owner` or `orchestrator` sessions may mint (§4.5, per
 *     #1253's permission matrix; the open question in design §8
 *     defaults to allowing both until ruled otherwise);
 *   - `role=agent` exactly, scoped to exactly one tenant: the
 *     tenant must be the session's own `binding_id` and is never
 *     `'*'` at issuance (cross-tenant agents hold one credential
 *     per tenant);
 *   - TTL is at most 30 days; rotation is by re-issuance;
 *   - the secret is generated here, shown exactly once in the
 *     issuance result, and never stored — only its SHA-256 digest
 *     reaches the metadata store (see
 *     `./agent-credential-store.ts`).
 *
 * Revocation is by `jti`, owner-scoped, and effective within one
 * request because both consumers (the exchange below and
 * `resolveAgentFrontDoor`) read the store live.
 *
 * The exchange is the surface C flow (§4.2): present
 * `jti` + secret, receive an audience-bound access token with a
 * 15-minute TTL and no refresh token — callers re-authenticate with
 * the credential. Signing is injected (`mintAccessToken`): the
 * provider owns the keys; this module owns the policy and the
 * claims shape H1/H4 consume (`binding_id`, `scope === binding_id`,
 * `roles: ["agent"]`, `jti`, `iat`, `exp`).
 */
import type { AgentCredentialMetadata } from "./agent-front-door.ts";
import type { AgentCredentialStore, StoredAgentCredential } from "./agent-credential-store.ts";
import type { ResolvedPrincipal, Role } from "./front-door.ts";

export const AGENT_CREDENTIAL_MAX_TTL_SEC = 30 * 86400;
export const AGENT_ACCESS_TOKEN_TTL_SEC = 15 * 60;
const DEFAULT_TTL_SEC = 86400;

export type IssueDenyReason =
  | "not_minting_role"
  | "invalid_tenant"
  | "invalid_audience"
  | "invalid_ttl"
  | "issuance_failed";

export type IssuedAgentCredential =
  | { ok: true; credential: AgentCredentialMetadata /** Shown exactly once; never stored. */; secret: string }
  | { ok: false; reason: IssueDenyReason };

export interface AgentCredentialDeps {
  store: AgentCredentialStore;
  /** Seconds since epoch; defaults to `Date.now() / 1000`. */
  now?: () => number;
  /** Credential handle generator; defaults to `crypto.randomUUID()`. */
  generateJti?: () => string;
  /** Secret generator; defaults to 32 random bytes, base64url. */
  generateSecret?: () => string;
  /** Audited account event for issuance/revocation (§4.8); awaited, fail-closed. */
  auditLifecycle: (event: {
    kind: "issued" | "revoked";
    jti: string;
    actorTokenId: string;
    accountId: string;
    bindingId: string;
    tenantId: string;
    at: number;
  }) => Promise<void>;
}

function defaultSecret(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  // base64url without Buffer: this module typechecks against
  // Workers types only (see tsconfig), same as front-door.ts.
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export async function digestSecret(secret: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(secret));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** Constant-work comparison for equal-length digest strings. */
function digestsEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

const MINTING_ROLES: ReadonlySet<Role> = new Set(["owner", "orchestrator"]);

export async function issueAgentCredential(
  session: ResolvedPrincipal,
  input: { tenantId: string; audience: string; ttlSec?: number },
  deps: AgentCredentialDeps,
): Promise<IssuedAgentCredential> {
  try {
    if (typeof deps.auditLifecycle !== "function") return { ok: false, reason: "issuance_failed" };
    if (!session.roles.some((role) => MINTING_ROLES.has(role))) return { ok: false, reason: "not_minting_role" };
    // Single tenant, the session's own binding, never '*'.
    if (
      typeof input.tenantId !== "string" ||
      input.tenantId.length === 0 ||
      input.tenantId === "*" ||
      input.tenantId !== session.bindingId
    )
      return { ok: false, reason: "invalid_tenant" };
    if (typeof input.audience !== "string" || input.audience.length === 0)
      return { ok: false, reason: "invalid_audience" };
    const ttlSec = input.ttlSec ?? DEFAULT_TTL_SEC;
    if (!Number.isSafeInteger(ttlSec) || ttlSec <= 0 || ttlSec > AGENT_CREDENTIAL_MAX_TTL_SEC)
      return { ok: false, reason: "invalid_ttl" };

    const now = Math.floor((deps.now ?? (() => Date.now() / 1000))());
    if (!Number.isSafeInteger(now)) return { ok: false, reason: "issuance_failed" };
    const jti = (deps.generateJti ?? (() => crypto.randomUUID()))();
    const secret = (deps.generateSecret ?? defaultSecret)();
    const metadata: AgentCredentialMetadata = {
      jti,
      accountId: session.accountId,
      bindingId: session.bindingId,
      tenantId: input.tenantId,
      audience: input.audience,
      role: "agent",
      issuedAt: now,
      expiresAt: now + ttlSec,
      revokedAt: null,
    };
    const stored: StoredAgentCredential = { ...metadata, secretDigest: await digestSecret(secret) };
    await deps.store.put(stored);
    try {
      await deps.auditLifecycle({
        kind: "issued",
        jti,
        accountId: session.accountId,
        bindingId: session.bindingId,
        tenantId: input.tenantId,
        actorTokenId: session.tokenId,
        at: now,
      });
    } catch {
      // Issuance without its audit event must not stand (§4.8).
      await deps.store.revoke(jti, now).catch(() => false);
      return { ok: false, reason: "issuance_failed" };
    }
    return { ok: true, credential: metadata, secret };
  } catch {
    return { ok: false, reason: "issuance_failed" };
  }
}

export async function revokeAgentCredential(
  session: ResolvedPrincipal,
  jti: string,
  deps: AgentCredentialDeps,
): Promise<{ ok: true } | { ok: false; reason: "not_minting_role" | "not_found" | "not_owner" | "audit_unavailable" }> {
  if (!session.roles.some((role) => MINTING_ROLES.has(role))) return { ok: false, reason: "not_minting_role" };
  if (typeof deps.auditLifecycle !== "function") return { ok: false, reason: "audit_unavailable" };
  const record = await deps.store.getByJti(jti);
  if (record === null) return { ok: false, reason: "not_found" };
  if (record.accountId !== session.accountId || record.bindingId !== session.bindingId)
    return { ok: false, reason: "not_owner" };
  const now = Math.floor((deps.now ?? (() => Date.now() / 1000))());
  const revoked = await deps.store.revoke(jti, now);
  if (revoked) {
    await deps.auditLifecycle({
      kind: "revoked",
      jti,
      accountId: record.accountId,
      bindingId: record.bindingId,
      tenantId: record.tenantId,
      actorTokenId: session.tokenId,
      at: now,
    });
  }
  return { ok: true };
}

export interface AccessTokenClaims {
  jti: string;
  accountId: string;
  bindingId: string;
  tenantId: string;
  audience: string;
  roles: ["agent"];
  issuedAt: number;
  expiresAt: number;
}

export type ExchangeResult =
  | { ok: true; accessToken: string; tokenType: "Bearer"; expiresIn: number; claims: AccessTokenClaims }
  | { ok: false; reason: "invalid_client" };

export interface ExchangeDeps extends AgentCredentialDeps {
  /** Provider signing seam: claims in, signed token out. */
  mintAccessToken: (claims: AccessTokenClaims) => Promise<string>;
}

/**
 * Surface C client-credentials exchange. Every failure is the same
 * `invalid_client` — the endpoint is not an oracle for which check
 * failed. No refresh token is issued (§4.2).
 */
export async function exchangeClientCredentials(
  input: { credentialId: string; secret: string; audience: string },
  deps: ExchangeDeps,
): Promise<ExchangeResult> {
  const deny: ExchangeResult = { ok: false, reason: "invalid_client" };
  try {
    if (!input.credentialId || !input.secret || !input.audience) return deny;
    const now = Math.floor((deps.now ?? (() => Date.now() / 1000))());
    // Live read: a credential revoked since issuance never exchanges.
    const record = await deps.store.getByJti(input.credentialId);
    if (
      record === null ||
      record.revokedAt !== null ||
      record.expiresAt <= now ||
      record.issuedAt > now ||
      record.audience !== input.audience ||
      record.role !== "agent" ||
      record.tenantId !== record.bindingId ||
      record.tenantId === "*"
    )
      return deny;
    const presented = await digestSecret(input.secret);
    if (!digestsEqual(presented, record.secretDigest)) return deny;
    const claims: AccessTokenClaims = {
      jti: record.jti,
      accountId: record.accountId,
      bindingId: record.bindingId,
      tenantId: record.tenantId,
      audience: record.audience,
      roles: ["agent"],
      issuedAt: now,
      expiresAt: now + AGENT_ACCESS_TOKEN_TTL_SEC,
    };
    const accessToken = await deps.mintAccessToken(claims);
    return { ok: true, accessToken, tokenType: "Bearer", expiresIn: AGENT_ACCESS_TOKEN_TTL_SEC, claims };
  } catch {
    return deny;
  }
}
