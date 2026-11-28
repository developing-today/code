/** IdP access-token verification and the #13799 server-side write-role gate. */
import { createPublicKey, verify as verifySignature } from "node:crypto";

export const IDP_WRITE_ROLES = ["owner", "orchestrator"] as const;
const CACHE_TTL_MS = 10 * 60 * 1000;

export interface VerifiedIdpToken {
  sub: string;
  roles: string[];
}

export type IdpWriteDecision =
  | { ok: true; configured: boolean; token?: VerifiedIdpToken }
  | { ok: false; status: 401 | 403; error: string };

type Jwk = JsonWebKey & { kid?: string; alg?: string; use?: string };
interface KeyCache {
  fetchedAt: number;
  keys: Jwk[];
  jwksUri: string;
}
const keyCache = new Map<string, KeyCache>();

export function clearIdpJwksCache(): void {
  keyCache.clear();
}

function configuredIssuer(env: Record<string, string | undefined>): string | null {
  const issuer = env.IDP_PROVIDER_ISSUER?.trim().replace(/\/$/, "");
  return issuer || null;
}

function decodePart(value: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(Buffer.from(value, "base64url").toString("utf8"));
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("JWT part is not an object");
  return parsed as Record<string, unknown>;
}

async function fetchKeys(issuer: string, fetchImpl: typeof fetch, refresh = false): Promise<KeyCache> {
  const cached = keyCache.get(issuer);
  if (!refresh && cached && Date.now() - cached.fetchedAt < CACHE_TTL_MS) return cached;
  const discoveryResponse = await fetchImpl(`${issuer}/.well-known/openid-configuration`, {
    signal: AbortSignal.timeout(5_000),
  });
  if (!discoveryResponse.ok) throw new Error(`OIDC discovery failed: HTTP ${discoveryResponse.status}`);
  const discovery = (await discoveryResponse.json()) as { issuer?: unknown; jwks_uri?: unknown };
  if (discovery.issuer !== issuer || typeof discovery.jwks_uri !== "string") {
    throw new Error("OIDC discovery issuer or jwks_uri is invalid");
  }
  const jwksResponse = await fetchImpl(discovery.jwks_uri, { signal: AbortSignal.timeout(5_000) });
  if (!jwksResponse.ok) throw new Error(`JWKS fetch failed: HTTP ${jwksResponse.status}`);
  const jwks = (await jwksResponse.json()) as { keys?: unknown };
  if (!Array.isArray(jwks.keys)) throw new Error("JWKS response carries no keys array");
  const entry = {
    fetchedAt: Date.now(),
    jwksUri: discovery.jwks_uri,
    keys: jwks.keys.filter((key): key is Jwk => !!key && typeof key === "object" && !Array.isArray(key)),
  };
  keyCache.set(issuer, entry);
  return entry;
}

export async function verifyIdpAccessToken(
  token: string,
  issuer: string,
  fetchImpl: typeof fetch = fetch,
): Promise<VerifiedIdpToken> {
  const parts = token.split(".");
  if (parts.length !== 3) throw new Error("token is not a compact JWS");
  const header = decodePart(parts[0]);
  if (header.alg !== "RS256" || typeof header.kid !== "string" || !header.kid) {
    throw new Error("unexpected jwt alg or missing kid");
  }
  let keySet = await fetchKeys(issuer, fetchImpl);
  let jwk = keySet.keys.find((key) => key.kid === header.kid && key.kty === "RSA" && key.use !== "enc");
  if (!jwk) {
    keySet = await fetchKeys(issuer, fetchImpl, true);
    jwk = keySet.keys.find((key) => key.kid === header.kid && key.kty === "RSA" && key.use !== "enc");
  }
  if (!jwk) throw new Error("unexpected jwt kid");
  const publicKey = createPublicKey({ key: jwk, format: "jwk" });
  const signed = Buffer.from(`${parts[0]}.${parts[1]}`);
  if (!verifySignature("RSA-SHA256", signed, publicKey, Buffer.from(parts[2], "base64url"))) {
    throw new Error("jwt signature verification failed");
  }
  const claims = decodePart(parts[1]);
  if (claims.iss !== issuer) throw new Error("unexpected jwt issuer");
  if (claims.token_use !== "access") throw new Error("not an access token");
  if (typeof claims.exp !== "number" || !Number.isFinite(claims.exp) || claims.exp <= Date.now() / 1000) {
    throw new Error("jwt expired or has no expiry");
  }
  if (typeof claims.sub !== "string" || claims.sub.trim() === "") throw new Error("jwt has no subject");
  const roles = Array.isArray(claims.roles)
    ? claims.roles.filter((role): role is string => typeof role === "string")
    : [];
  return { sub: claims.sub, roles };
}

/** With no configured IdP, preserve the current rollout behavior. */
export async function enforceIdpWriteRole(
  req: Request,
  env: Record<string, string | undefined> = process.env,
  fetchImpl: typeof fetch = fetch,
): Promise<IdpWriteDecision> {
  const issuer = configuredIssuer(env);
  if (!issuer) return { ok: true, configured: false };
  const authorization = req.headers.get("authorization") ?? "";
  const token = authorization.startsWith("Bearer ") ? authorization.slice("Bearer ".length).trim() : "";
  if (!token) return { ok: false, status: 401, error: "missing IdP bearer token" };
  let verified: VerifiedIdpToken;
  try {
    verified = await verifyIdpAccessToken(token, issuer, fetchImpl);
  } catch (error) {
    return {
      ok: false,
      status: 401,
      error: `invalid IdP access token: ${error instanceof Error ? error.message : String(error)}`,
    };
  }
  if (!verified.roles.some((role) => (IDP_WRITE_ROLES as readonly string[]).includes(role))) {
    return { ok: false, status: 403, error: "owner or orchestrator role required" };
  }
  return { ok: true, configured: true, token: verified };
}
