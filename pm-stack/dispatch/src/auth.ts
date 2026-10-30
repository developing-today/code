// Caller authentication for every endpoint except GET /health.
//
// Two accepted forms on `Authorization: Bearer <credential>`:
//  1. GitHub Actions OIDC — a JWT issued by
//     https://token.actions.githubusercontent.com (RS256, verified against
//     its JWKS), audience = cfg.oidcAudience. The token's `repository`
//     claim must be in the allowlist. Grants mesh: any allowlisted
//     repo may dispatch for any allowlisted repo (see assertCallerRepo).
//  2. A static bearer secret (DISPATCH_API_TOKEN), compared in constant
//     time — the shape a repo stores as a GitHub Actions secret.
//
// Signature verification is factored behind an injectable JWKS source so
// tests can stub it; claim checks are pure.
import { createHash, timingSafeEqual } from "node:crypto";

export type CallerIdentity =
  | { type: "bearer" }
  | {
      type: "oidc";
      repository: string;
      actor?: string;
      workflow_ref?: string;
      sub?: string;
    };

export class AuthError extends Error {
  status = 401;
}

export function timingSafeStringEqual(a: string, b: string): boolean {
  const ha = createHash("sha256").update(a).digest();
  const hb = createHash("sha256").update(b).digest();
  return timingSafeEqual(ha, hb);
}

// ---------------------------------------------------------------------------
// JWT plumbing
// ---------------------------------------------------------------------------

export interface JwtParts {
  header: Record<string, unknown>;
  payload: Record<string, unknown>;
  signingInput: string;
  signature: Uint8Array;
}

function b64urlDecode(s: string): Uint8Array {
  const pad = s.length % 4 === 0 ? "" : "=".repeat(4 - (s.length % 4));
  const b64 = s.replace(/-/g, "+").replace(/_/g, "/") + pad;
  return new Uint8Array(Buffer.from(b64, "base64"));
}

export function parseJwt(token: string): JwtParts | null {
  const parts = token.split(".");
  if (parts.length !== 3) return null;
  try {
    const header = JSON.parse(new TextDecoder().decode(b64urlDecode(parts[0]!)));
    const payload = JSON.parse(new TextDecoder().decode(b64urlDecode(parts[1]!)));
    return {
      header,
      payload,
      signingInput: `${parts[0]}.${parts[1]}`,
      signature: b64urlDecode(parts[2]!),
    };
  } catch {
    return null;
  }
}

export type Jwks = { keys: Array<Record<string, unknown>> };
export type JwksSource = () => Promise<Jwks>;

const GITHUB_JWKS_URL = "https://token.actions.githubusercontent.com/.well-known/jwks.json";
let jwksCache: { at: number; jwks: Jwks } | null = null;
const JWKS_TTL_MS = 60 * 60_000;

export const fetchGithubJwks: JwksSource = async () => {
  if (jwksCache && Date.now() - jwksCache.at < JWKS_TTL_MS) return jwksCache.jwks;
  const res = await fetch(GITHUB_JWKS_URL);
  if (!res.ok) throw new AuthError(`oidc: JWKS fetch failed (HTTP ${res.status})`);
  const jwks = (await res.json()) as Jwks;
  jwksCache = { at: Date.now(), jwks };
  return jwks;
};

export interface OidcVerifyOptions {
  audience: string;
  allowlist: string[];
  jwksSource?: JwksSource;
  /** Epoch ms "now" — injectable for tests. */
  now?: number;
}

/**
 * Verify a GitHub Actions OIDC token end-to-end (signature + claims) and
 * return the caller identity. Throws AuthError on any failure.
 */
export async function verifyOidcToken(token: string, opts: OidcVerifyOptions): Promise<CallerIdentity> {
  const jwt = parseJwt(token);
  if (!jwt) throw new AuthError("oidc: malformed JWT");
  if (jwt.header.alg !== "RS256") throw new AuthError("oidc: unexpected alg (want RS256)");
  const kid = jwt.header.kid;
  if (typeof kid !== "string" || !kid) throw new AuthError("oidc: missing kid");

  const jwks = await (opts.jwksSource ?? fetchGithubJwks)();
  const jwk = jwks.keys.find((k) => k.kid === kid);
  if (!jwk) throw new AuthError("oidc: signing key not found in JWKS");

  let key: CryptoKey;
  try {
    key = await crypto.subtle.importKey(
      "jwk",
      jwk as JsonWebKey,
      { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
      false,
      ["verify"],
    );
  } catch {
    throw new AuthError("oidc: unusable signing key");
  }
  const ok = await crypto.subtle.verify(
    "RSASSA-PKCS1-v1_5",
    key,
    jwt.signature as BufferSource,
    new TextEncoder().encode(jwt.signingInput),
  );
  if (!ok) throw new AuthError("oidc: signature verification failed");

  const c = jwt.payload;
  const nowSec = Math.floor((opts.now ?? Date.now()) / 1000);
  if (c.iss !== "https://token.actions.githubusercontent.com") {
    throw new AuthError("oidc: unexpected issuer");
  }
  if (typeof c.exp !== "number" || c.exp <= nowSec) throw new AuthError("oidc: token expired");
  if (typeof c.nbf === "number" && c.nbf > nowSec + 60) throw new AuthError("oidc: token not yet valid");
  const aud = c.aud;
  const audOk = aud === opts.audience || (Array.isArray(aud) && aud.includes(opts.audience));
  if (!audOk) throw new AuthError("oidc: unexpected audience");
  const repository = c.repository;
  if (typeof repository !== "string" || !repository) throw new AuthError("oidc: missing repository claim");
  if (!opts.allowlist.includes(repository)) {
    throw new AuthError(`oidc: repository ${repository} is not in the dispatch allowlist`);
  }
  return {
    type: "oidc",
    repository,
    actor: typeof c.actor === "string" ? c.actor : undefined,
    workflow_ref: typeof c.workflow_ref === "string" ? c.workflow_ref : undefined,
    sub: typeof c.sub === "string" ? c.sub : undefined,
  };
}

// ---------------------------------------------------------------------------
// Request-level auth
// ---------------------------------------------------------------------------

export interface AuthDeps {
  apiToken: string | null;
  oidcAudience: string;
  allowlist: string[];
  jwksSource?: JwksSource;
  now?: number;
}

/** Authenticate a request. Returns the caller identity or throws AuthError. */
export async function authenticate(req: Request, deps: AuthDeps): Promise<CallerIdentity> {
  const header = req.headers.get("authorization") ?? "";
  const m = /^Bearer\s+(.+)$/i.exec(header.trim());
  if (!m) throw new AuthError("missing bearer credential");
  const credential = m[1]!.trim();

  // Static bearer first (cheap, constant-time). A JWT never equals it.
  if (deps.apiToken && timingSafeStringEqual(credential, deps.apiToken)) {
    return { type: "bearer" };
  }
  // Anything JWT-shaped is tried as OIDC; verification failure is final.
  if (credential.split(".").length === 3) {
    return verifyOidcToken(credential, {
      audience: deps.oidcAudience,
      allowlist: deps.allowlist,
      jwksSource: deps.jwksSource,
      now: deps.now,
    });
  }
  throw new AuthError("unrecognized credential");
}

/**
 * Repo authorization — the grants mesh.
 *
 * Grants: every allowlisted repo may target every allowlisted repo.
 * Repos outside the allowlist can neither call nor be targeted.
 *
 * The target repo must be in the allowlist for every caller. An OIDC
 * caller's repository claim must also be in the allowlist
 * (verifyOidcToken already enforces this; it is re-checked here so the
 * grant rule is complete at this layer), but it need NOT equal the
 * target: any allowlisted caller may dispatch for any allowlisted
 * target. Bearer callers are bound only by the allowlist on the target.
 */
export function assertCallerRepo(caller: CallerIdentity, repo: string, allowlist: string[]): void {
  if (!allowlist.includes(repo)) {
    throw new AuthError(`repo ${repo} is not in the dispatch allowlist`);
  }
  if (caller.type === "oidc" && !allowlist.includes(caller.repository)) {
    throw new AuthError(
      `oidc: token repository ${caller.repository} is not in the dispatch allowlist`,
    );
  }
}
