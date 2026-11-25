/**
 * SSO relying-party front door, slice H1 (#14259, child of #9086).
 *
 * ONE shared front-door module consumed by all hosted surfaces
 * (design `doc/2026-11-24T04-50-00Z_design_sso-relying-party-9086/`
 * §4.1, §5 slice H1): validate (JWKS) → resolve (provider sub →
 * tenancy `account_id` via `identity_link`) → role-load (live
 * `role_grant` rows) → fail-closed. Every denial is a returned
 * `{ ok: false, reason }`, never a thrown error and never a partial
 * principal. Token values are never stored (E §2.3).
 *
 * Home: this file lives in `pm-service` because surface A (the PM
 * service dashboard) is the first consumer (H3 wires it in); H4's
 * surface-C integration imports the same module, and it is promoted
 * to a shared package when a second surface outside `pm-service`
 * wires in — no surface rolls its own auth (design §3, Alt 1).
 *
 * PROVISIONAL CLAIM SHAPE (design §8 open question 1): the exact
 * role-claim shape is still to be agreed with #9077 — the
 * claim-shape request was filed on #9077 on 2026-11-24 and is
 * unanswered. Per the design, H1 proceeds with this proposed shape,
 * recorded as provisional; the default applies if #9077 is still
 * unanswered on ~2026-12-01 (7 days):
 *
 *   - `binding_id: string` — the tenancy binding this token opens.
 *   - `roles: string[]` — role list, each in #1253's vocabulary
 *     (`owner | orchestrator | agent | read-only`).
 *   - Standard claims: `iss` (the #9077 provider), `aud` (the
 *     surface), `sub` (provider subject), `exp`, `iat`, `jti`.
 *   - One token opens exactly one binding (single-tenant, §4.5);
 *     multi-tenant representation, if #9077 answers differently, is
 *     a shape change here, not a second module.
 *
 * Decisions recorded (decide-and-proceed, FDR-0013):
 *
 *   - Live `role_grant` rows are authoritative. The token's role
 *     list is a snapshot bound into the session later (H3's cookie);
 *     the roles this function returns are the live rows. A token
 *     claiming a role the live rows do not grant gains nothing, and
 *     an account with no live roles is denied (`no_live_roles`).
 *   - `resolveAccountId` / `loadRoles` are interfaces: H1 ships
 *     with no database behind them. H2 wires them to the Turso
 *     central tenancy DB (`identity_link`, `role_grant`); tests
 *     supply in-memory doubles.
 *   - No crypto dependency: RS256 verification uses WebCrypto
 *     (`RSASSA-PKCS1-v1_5` + SHA-256), which runs in Bun and in
 *     Cloudflare Workers alike. `jose` or similar is not needed.
 */
export type Role = "owner" | "orchestrator" | "agent" | "read-only";

export const ROLES: ReadonlySet<string> = new Set(["owner", "orchestrator", "agent", "read-only"]);

export interface JwksKey {
  kty?: string;
  kid?: string;
  alg?: string;
  use?: string;
  n?: string;
  e?: string;
  [key: string]: unknown;
}

export interface JsonWebKeySet {
  keys: JwksKey[];
}

export interface ResolvedPrincipal {
  accountId: string;
  bindingId: string;
  roles: Role[];
  providerSub: string;
  tokenId: string;
}

export type DenyReason =
  | "malformed_token"
  | "bad_algorithm"
  | "unknown_key"
  | "bad_signature"
  | "bad_issuer"
  | "bad_audience"
  | "expired"
  | "not_yet_valid"
  | "bad_claim_shape"
  | "no_identity_link"
  | "no_live_roles";

export type FrontDoorResult = { ok: true; principal: ResolvedPrincipal } | { ok: false; reason: DenyReason };

export interface FrontDoorDeps {
  /** Expected `iss` — the #9077 provider. */
  issuer: string;
  /** Expected `aud` — this surface (e.g. `pm-service`). */
  audience: string;
  /** Provider JWKS (fetched by the caller; H3 refreshes it). */
  jwks: JsonWebKeySet;
  /** Seconds since epoch; defaults to `Date.now() / 1000`. */
  now?: () => number;
  /** `identity_link` lookup: provider sub → tenancy `account_id`. */
  resolveAccountId: (providerSub: string) => Promise<string | null>;
  /** Live `role_grant` rows for (account_id, binding_id). */
  loadRoles: (accountId: string, bindingId: string) => Promise<Role[]>;
}

function deny(reason: DenyReason): FrontDoorResult {
  return { ok: false, reason };
}

function base64urlDecode(input: string): Uint8Array | null {
  if (!/^[A-Za-z0-9_-]*$/.test(input)) return null;
  try {
    const binary = atob(input.replace(/-/g, "+").replace(/_/g, "/"));
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  } catch {
    return null;
  }
}

function decodeJsonPart(input: string): Record<string, unknown> | null {
  const bytes = base64urlDecode(input);
  if (bytes === null) return null;
  try {
    const parsed: unknown = JSON.parse(new TextDecoder().decode(bytes));
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return null;
    return parsed as Record<string, unknown>;
  } catch {
    return null;
  }
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function audienceMatches(aud: unknown, expected: string): boolean {
  if (aud === expected) return true;
  return Array.isArray(aud) && aud.includes(expected);
}

/**
 * Validate → resolve → role-load → fail-closed.
 *
 * Any malformed input, failed check, missing link, or empty live
 * role set denies with a stable reason code. This function never
 * throws for bad tokens; unexpected internal errors deny as
 * `malformed_token` rather than leaking a partial principal.
 */
export async function resolveFrontDoor(token: string, deps: FrontDoorDeps): Promise<FrontDoorResult> {
  try {
    const nowSec = Math.floor((deps.now ?? (() => Date.now() / 1000))());
    const parts = token.split(".");
    if (parts.length !== 3) return deny("malformed_token");
    const [encodedHeader, encodedPayload, encodedSignature] = parts as [string, string, string];

    const header = decodeJsonPart(encodedHeader);
    const claims = decodeJsonPart(encodedPayload);
    const signature = base64urlDecode(encodedSignature);
    if (header === null || claims === null || signature === null) return deny("malformed_token");

    if (header.alg !== "RS256") return deny("bad_algorithm");
    if (!isNonEmptyString(header.kid)) return deny("unknown_key");
    const jwk = deps.jwks.keys.find((key) => key.kid === header.kid);
    if (jwk === undefined) return deny("unknown_key");

    let key: CryptoKey;
    try {
      // The JWKS key is provider data: cast to the DOM JsonWebKey
      // (now unshadowed) and let importKey validate the material;
      // a bad key denies as unknown_key below.
      key = await crypto.subtle.importKey(
        "jwk",
        jwk as unknown as JsonWebKey,
        { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
        false,
        ["verify"],
      );
    } catch {
      return deny("unknown_key");
    }
    const signed = new TextEncoder().encode(`${encodedHeader}.${encodedPayload}`);
    const valid = await crypto.subtle.verify("RSASSA-PKCS1-v1_5", key, signature, signed);
    if (!valid) return deny("bad_signature");

    if (claims.iss !== deps.issuer) return deny("bad_issuer");
    if (!audienceMatches(claims.aud, deps.audience)) return deny("bad_audience");
    if (typeof claims.exp !== "number" || claims.exp <= nowSec) return deny("expired");
    if (typeof claims.nbf === "number" && claims.nbf > nowSec) return deny("not_yet_valid");
    if (!isNonEmptyString(claims.sub)) return deny("bad_claim_shape");
    if (!isNonEmptyString(claims.jti)) return deny("bad_claim_shape");
    // Provisional shape (see header): binding_id + role list as claims.
    if (!isNonEmptyString(claims.binding_id)) return deny("bad_claim_shape");
    if (
      !Array.isArray(claims.roles) ||
      claims.roles.length === 0 ||
      !claims.roles.every((role) => typeof role === "string" && ROLES.has(role))
    ) {
      return deny("bad_claim_shape");
    }

    const providerSub = claims.sub;
    const bindingId = claims.binding_id;
    const accountId = await deps.resolveAccountId(providerSub);
    if (accountId === null) return deny("no_identity_link");

    // Live role_grant rows are authoritative (see header).
    const roles = await deps.loadRoles(accountId, bindingId);
    if (roles.length === 0) return deny("no_live_roles");

    return {
      ok: true,
      principal: { accountId, bindingId, roles, providerSub, tokenId: claims.jti },
    };
  } catch {
    return deny("malformed_token");
  }
}
