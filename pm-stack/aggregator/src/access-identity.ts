/**
 * Config-gated Cloudflare Access JWT author derivation (#1253, small-fix
 * slice queued as task 4964 v2).
 *
 * When the deployment configures Access (PM_AGG_ACCESS_TEAM_DOMAIN and
 * PM_AGG_ACCESS_AUD both set), aggregator write endpoints stop trusting
 * the caller-supplied DashAuthor and derive it from the verified
 * Cf-Access-Jwt-Assertion instead:
 *
 * - invalid or missing token            -> 401
 * - asserted kind "drew" for a verified identity that is NOT in the
 *   PM_AGG_OWNER_IDS allowlist         -> 403 (contradictory owner claim)
 * - Access configured but allowlist empty: fail closed — nobody may
 *   write as drew; other kinds derive from the verified JWT as agent
 * - verified owner identity             -> kind "drew"
 * - verified non-owner identity         -> kind "agent" (id = sub)
 *
 * When Access is NOT configured this module is inert: every helper
 * short-circuits to the legacy path, so unconfigured deployments behave
 * byte-identically to before (accepted risk, #941).
 */
import type { DashAuthor } from "./model.ts";

export const ACCESS_JWT_HEADER = "Cf-Access-Jwt-Assertion";

export interface AccessConfig {
  /** Cloudflare Access team domain, e.g. "team.cloudflareaccess.com". */
  teamDomain: string;
  /** Application AUD tag the JWT must carry. */
  aud: string;
  /** Verified-identity allowlist (sub or email values, comma-separated). */
  ownerIds: string[];
}

export function readAccessConfig(
  env: Record<string, string | undefined> = process.env,
): AccessConfig | null {
  const teamDomain = (env.PM_AGG_ACCESS_TEAM_DOMAIN ?? "").trim();
  const aud = (env.PM_AGG_ACCESS_AUD ?? "").trim();
  if (teamDomain === "" || aud === "") return null;
  const ownerIds = (env.PM_AGG_OWNER_IDS ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s !== "");
  return { teamDomain, aud, ownerIds };
}

export interface VerifiedAccessIdentity {
  sub: string;
  email: string;
}

export type AuthorDecision =
  | { ok: true; author: DashAuthor }
  | { ok: false; status: 401 | 403; error: string };

function base64UrlToBytes(s: string): Uint8Array {
  const padded = s.replace(/-/g, "+").replace(/_/g, "/");
  const bin = atob(padded);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

interface JwksCacheEntry {
  fetchedAt: number;
  keys: Array<Record<string, unknown>>;
}

const jwksCache = new Map<string, JwksCacheEntry>();
const JWKS_TTL_MS = 10 * 60 * 1000;

export function clearAccessJwksCache(): void {
  jwksCache.clear();
}

async function fetchJwks(
  teamDomain: string,
  fetchImpl: typeof fetch = fetch,
  refresh = false,
): Promise<Array<Record<string, unknown>>> {
  const now = Date.now();
  const cached = jwksCache.get(teamDomain);
  if (!refresh && cached && now - cached.fetchedAt < JWKS_TTL_MS) return cached.keys;
  const res = await fetchImpl(`https://${teamDomain}/cdn-cgi/access/certs`, {
    signal: AbortSignal.timeout(5_000),
  });
  if (!res.ok) throw new Error(`JWKS fetch failed: HTTP ${res.status}`);
  const body = (await res.json()) as { keys?: Array<Record<string, unknown>> };
  if (!Array.isArray(body.keys)) throw new Error("JWKS response carries no keys array");
  jwksCache.set(teamDomain, { fetchedAt: now, keys: body.keys });
  return body.keys;
}

/**
 * Verify a Cloudflare Access JWT (RS256, team JWKS) and return the
 * identity claims. Throws on any failure — callers map to 401.
 */
export async function verifyAccessJwt(
  token: string,
  config: AccessConfig,
  fetchImpl: typeof fetch = fetch,
): Promise<VerifiedAccessIdentity> {
  const parts = token.split(".");
  if (parts.length !== 3) throw new Error("malformed JWT");
  let header: Record<string, unknown>;
  let payload: Record<string, unknown>;
  try {
    header = JSON.parse(new TextDecoder().decode(base64UrlToBytes(parts[0])));
    payload = JSON.parse(new TextDecoder().decode(base64UrlToBytes(parts[1])));
  } catch {
    throw new Error("malformed JWT claims");
  }
  if (header.alg !== "RS256") throw new Error(`unexpected JWT alg: ${String(header.alg)}`);
  if (typeof header.kid !== "string" || header.kid === "") throw new Error("JWT carries no kid");
  let keys = await fetchJwks(config.teamDomain, fetchImpl);
  let jwk = keys.find((k) => k.kid === header.kid && (k.kty === undefined || k.kty === "RSA"));
  // Cloudflare can rotate signing keys before the local cache TTL ends.
  // An unknown kid triggers one forced refresh before rejecting the token.
  if (!jwk) {
    keys = await fetchJwks(config.teamDomain, fetchImpl, true);
    jwk = keys.find((k) => k.kid === header.kid && (k.kty === undefined || k.kty === "RSA"));
  }
  if (!jwk) throw new Error("no JWKS key matches JWT kid");
  const key = await crypto.subtle.importKey(
    "jwk",
    jwk as JsonWebKey,
    { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
    false,
    ["verify"],
  );
  const data = new TextEncoder().encode(`${parts[0]}.${parts[1]}`);
  const valid = await crypto.subtle.verify(
    "RSASSA-PKCS1-v1_5",
    key,
    base64UrlToBytes(parts[2]) as unknown as BufferSource,
    data as unknown as BufferSource,
  );
  if (!valid) throw new Error("JWT signature invalid");
  // Audience: string or list containing the configured AUD.
  const aud = payload.aud;
  const audOk = typeof aud === "string" ? aud === config.aud : Array.isArray(aud) && aud.includes(config.aud);
  if (!audOk) throw new Error("JWT audience mismatch");
  // Issuer: the team's Access origin.
  const iss = payload.iss;
  if (typeof iss !== "string" || iss !== `https://${config.teamDomain}`) {
    throw new Error("JWT issuer mismatch");
  }
  // Expiry, with 60 s clock skew tolerance.
  if (typeof payload.exp !== "number" || payload.exp * 1000 < Date.now() - 60_000) {
    throw new Error("JWT expired");
  }
  if (payload.nbf !== undefined && (typeof payload.nbf !== "number" || payload.nbf * 1000 > Date.now() + 60_000)) {
    throw new Error("JWT not yet valid");
  }
  const sub = typeof payload.sub === "string" ? payload.sub : "";
  const email = typeof payload.email === "string" ? payload.email : "";
  if (sub === "" && email === "") throw new Error("JWT carries no sub or email identity");
  return { sub, email };
}

/** Map a verified identity to its DashAuthor kind via the owner allowlist. */
export function deriveAuthorFromAccess(
  verified: VerifiedAccessIdentity,
  asserted: DashAuthor | null,
  ownerIds: string[],
): AuthorDecision {
  const identity = verified.email !== "" ? verified.email : verified.sub;
  const isOwner =
    (verified.sub !== "" && ownerIds.includes(verified.sub)) ||
    (verified.email !== "" && ownerIds.includes(verified.email));
  if (isOwner) {
    // The verified owner IS the standing drew identity (registry.ts:
    // { kind: "drew", id: "drew" }). Deriving id = email/sub instead
    // would re-key every pre-Access drew-authored artifact on the day
    // Access is enabled — saved views above all: ViewsStore owner
    // operations match ownerKey ("drew:drew") exactly, so the owner
    // would lose rename/remove/setDefault over their own views
    // (Codex P2, PR #8645). Map the verified owner to the standing
    // identity; the allowlist check above is what authorizes it,
    // never the caller-supplied author.
    return { ok: true, author: { kind: "drew", id: "drew", label: "Drew" } };
  }
  // Not the owner: asserting drew is a contradiction -> 403. This also
  // covers the fail-closed case (empty allowlist: NOBODY is an owner).
  if (asserted?.kind === "drew") {
    return {
      ok: false,
      status: 403,
      error: "asserted drew author contradicts the verified Access identity",
    };
  }
  return {
    ok: true,
    author: {
      kind: "agent",
      id: verified.sub.slice(0, 120) || identity.slice(0, 120),
      label: identity.slice(0, 120),
    },
  };
}

type AuthorEnforcement =
  | { ok: true; author: DashAuthor }
  | { ok: false; status: 401 | 403; error: string };

/**
 * Build the Access author enforcer for ONE HTTP request. The JWT is
 * verified at most once per enforcer: the verified identity — or the
 * 401 failure — is memoized and reused for every call, while author
 * derivation (including the per-item contradictory-drew 403) still
 * runs per call. Batch endpoints MUST use one enforcer for the whole
 * batch: verifying per item repeats the JWKS fetch per action
 * whenever the cache is cold or the certs endpoint is failing (a
 * 100-action batch could stack ~100 sequential 5 s timeouts just to
 * report the same 401 — Codex P2, PR #8645).
 */
export function createAccessAuthorEnforcer(
  req: Request,
  env: Record<string, string | undefined> = process.env,
  fetchImpl: typeof fetch = fetch,
): (asserted: DashAuthor) => Promise<AuthorEnforcement> {
  const config = readAccessConfig(env);
  let verification: Promise<
    { ok: true; verified: VerifiedAccessIdentity } | { ok: false; status: 401; error: string }
  > | null = null;
  return async (asserted: DashAuthor): Promise<AuthorEnforcement> => {
    if (!config) return { ok: true, author: asserted };
    if (!verification) {
      verification = (async () => {
        const token = req.headers.get(ACCESS_JWT_HEADER) ?? "";
        if (token === "") {
          return { ok: false, status: 401, error: "missing Cf-Access-Jwt-Assertion" } as const;
        }
        try {
          const verified = await verifyAccessJwt(token, config, fetchImpl);
          return { ok: true, verified } as const;
        } catch (err) {
          return {
            ok: false,
            status: 401,
            error: `invalid Access JWT: ${err instanceof Error ? err.message : err}`,
          } as const;
        }
      })();
    }
    const outcome = await verification;
    if (!outcome.ok) return outcome;
    return deriveAuthorFromAccess(outcome.verified, asserted, config.ownerIds);
  };
}

/**
 * Enforce Access-derived authorship for one aggregator write. Returns the
 * author to persist, or an HTTP error. When Access is unconfigured the
 * asserted (legacy-validated) author passes through untouched.
 */
export async function enforceAccessAuthor(
  req: Request,
  asserted: DashAuthor,
  env: Record<string, string | undefined> = process.env,
  fetchImpl: typeof fetch = fetch,
): Promise<AuthorEnforcement> {
  return createAccessAuthorEnforcer(req, env, fetchImpl)(asserted);
}
