// RP-token validation for fleet-hosted (#12583; design §4.1).
//
// Callers present #9077 relying-party tokens: OIDC ID/access tokens the
// devtoday provider issues as compact JWS (JWT) bearer tokens. This
// module validates them the standard relying-party way, with WebCrypto
// only (no dependencies): JWS signature against the provider's JWKS,
// then iss / aud / exp / nbf claim checks. The provider itself is a
// design-stage dependency (#9077), so issuer, audience and JWKS URI
// are configuration (src/config.ts) and the JWKS fetch is injectable —
// production fetches the provider's JWKS endpoint; tests supply a
// locally minted key set. Validation never calls back to the caller
// (design §4.1: "a bearer token the service can validate without a
// callback to the caller").

import { FailClosed } from "./errors.ts";

export interface Jwk {
  kty: string;
  kid?: string;
  alg?: string;
  crv?: string;
  n?: string;
  e?: string;
  x?: string;
  y?: string;
  [key: string]: unknown;
}

export interface Jwks {
  keys: Jwk[];
}

export type JwksFetcher = (jwksUri: string) => Promise<Jwks>;

export interface RpValidationConfig {
  issuer: string;
  audience: string;
  jwksUri: string;
}

export interface RpTokenClaims {
  /** The provider subject (`sub`). Tenancy keying input only — never
   * recorded as identity downstream; src/tenancy.ts maps it to a
   * binding_id (design §4.1, component E keying rule). */
  subject: string;
  /** Role names from the token (#1253 role model: owner /
   * orchestrator / agent / read-only). */
  roles: string[];
  /** Agent-role claims (#1253): the lane the credential names. */
  lane?: string;
  /** Agent-role claims (#1253): the session the credential names. */
  sessionId?: string;
  raw: Record<string, unknown>;
}

const SUPPORTED_ALGS = ["RS256", "ES256"] as const;

const jwksCache = new Map<string, { jwks: Jwks; fetchedAt: number }>();
const JWKS_TTL_MS = 5 * 60 * 1000;

export async function fetchJwksDefault(jwksUri: string): Promise<Jwks> {
  const res = await fetch(jwksUri);
  if (!res.ok) {
    throw new FailClosed(503, "jwks_unavailable", `provider JWKS fetch failed: HTTP ${res.status}`);
  }
  return (await res.json()) as Jwks;
}

async function getJwks(jwksUri: string, fetcher: JwksFetcher, now: number): Promise<Jwks> {
  const hit = jwksCache.get(jwksUri);
  if (hit && now - hit.fetchedAt < JWKS_TTL_MS) return hit.jwks;
  const jwks = await fetcher(jwksUri);
  if (!jwks || !Array.isArray(jwks.keys)) {
    throw new FailClosed(503, "jwks_unavailable", "provider JWKS document has no keys array");
  }
  jwksCache.set(jwksUri, { jwks, fetchedAt: now });
  return jwks;
}

function b64urlToBytes(text: string): Uint8Array {
  const b64 = text.replace(/-/g, "+").replace(/_/g, "/");
  const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
  const bin = atob(padded);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function b64urlToJson(text: string): Record<string, unknown> {
  const bytes = b64urlToBytes(text);
  const parsed: unknown = JSON.parse(new TextDecoder().decode(bytes));
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new FailClosed(401, "malformed_token", "token segment is not a JSON object");
  }
  return parsed as Record<string, unknown>;
}

export interface ParsedJwt {
  header: Record<string, unknown>;
  payload: Record<string, unknown>;
  signature: Uint8Array;
  signingInput: string;
}

/** True when the token has the three-segment compact-JWS shape. */
export function looksLikeJwt(token: string): boolean {
  return token.split(".").length === 3;
}

export function parseJwt(token: string): ParsedJwt {
  const parts = token.split(".");
  if (parts.length !== 3 || parts.some((p) => p === "")) {
    throw new FailClosed(401, "malformed_token", "token is not a compact JWS");
  }
  try {
    return {
      header: b64urlToJson(parts[0]),
      payload: b64urlToJson(parts[1]),
      signature: b64urlToBytes(parts[2]),
      signingInput: `${parts[0]}.${parts[1]}`,
    };
  } catch (e) {
    if (e instanceof FailClosed) throw e;
    throw new FailClosed(401, "malformed_token", "token segments do not decode");
  }
}

async function importKey(jwk: Jwk, alg: string): Promise<CryptoKey> {
  if (alg === "RS256") {
    if (jwk.kty !== "RSA") {
      throw new FailClosed(401, "invalid_token", "JWKS key type does not match token alg");
    }
    return crypto.subtle.importKey(
      "jwk",
      jwk,
      { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
      false,
      ["verify"],
    );
  }
  if (jwk.kty !== "EC" || jwk.crv !== "P-256") {
    throw new FailClosed(401, "invalid_token", "JWKS key type does not match token alg");
  }
  return crypto.subtle.importKey("jwk", jwk, { name: "ECDSA", namedCurve: "P-256" }, false, [
    "verify",
  ]);
}

function verifyParams(alg: string): AlgorithmIdentifier | EcdsaParams {
  return alg === "RS256"
    ? { name: "RSASSA-PKCS1-v1_5" }
    : { name: "ECDSA", hash: "SHA-256" };
}

function stringClaim(payload: Record<string, unknown>, name: string): string | undefined {
  const v = payload[name];
  return typeof v === "string" && v !== "" ? v : undefined;
}

function rolesClaim(payload: Record<string, unknown>): string[] {
  const roles = payload["roles"];
  if (Array.isArray(roles)) return roles.filter((r): r is string => typeof r === "string");
  const role = payload["role"];
  return typeof role === "string" && role !== "" ? [role] : [];
}

/**
 * Validate an RP token end to end. Throws FailClosed(401) on any
 * defect — signature, issuer, audience, lifetime, shape. Returns the
 * claims the auth layer maps into a caller context.
 */
export async function validateRpToken(
  token: string,
  cfg: RpValidationConfig,
  fetcher: JwksFetcher = fetchJwksDefault,
  now: number = Date.now(),
): Promise<RpTokenClaims> {
  const parsed = parseJwt(token);
  const alg = parsed.header["alg"];
  if (typeof alg !== "string" || !(SUPPORTED_ALGS as readonly string[]).includes(alg)) {
    throw new FailClosed(401, "invalid_token", "token alg is not supported");
  }
  const kid = parsed.header["kid"];
  if (typeof kid !== "string" || kid === "") {
    throw new FailClosed(401, "invalid_token", "token has no kid");
  }
  const jwks = await getJwks(cfg.jwksUri, fetcher, now);
  const jwk = jwks.keys.find((k) => k.kid === kid);
  if (!jwk) {
    throw new FailClosed(401, "invalid_token", "token kid is not in the provider JWKS");
  }
  const key = await importKey(jwk, alg);
  const ok = await crypto.subtle.verify(
    verifyParams(alg),
    key,
    parsed.signature as BufferSource,
    new TextEncoder().encode(parsed.signingInput),
  );
  if (!ok) {
    throw new FailClosed(401, "invalid_signature", "token signature does not verify");
  }

  const p = parsed.payload;
  if (p["iss"] !== cfg.issuer) {
    throw new FailClosed(401, "invalid_issuer", "token issuer is not the relying-party provider");
  }
  const aud = p["aud"];
  const audiences = Array.isArray(aud) ? aud : [aud];
  if (!audiences.includes(cfg.audience)) {
    throw new FailClosed(401, "invalid_audience", "token audience does not name this service");
  }
  const nowSec = Math.floor(now / 1000);
  if (typeof p["exp"] !== "number" || p["exp"] <= nowSec) {
    throw new FailClosed(401, "token_expired", "token is expired or carries no exp");
  }
  if (typeof p["nbf"] === "number" && p["nbf"] > nowSec) {
    throw new FailClosed(401, "token_not_yet_valid", "token nbf is in the future");
  }
  const subject = stringClaim(p, "sub");
  if (!subject) {
    throw new FailClosed(401, "invalid_token", "token carries no subject");
  }
  return {
    subject,
    roles: rolesClaim(p),
    lane: stringClaim(p, "lane"),
    sessionId: stringClaim(p, "session_id") ?? stringClaim(p, "session"),
    raw: p,
  };
}
