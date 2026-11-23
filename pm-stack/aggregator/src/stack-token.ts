/**
 * Short-lived identity tokens for brokered writes (#9605, #1253 split 3).
 *
 * The aggregator is the only minting authority. Consumers verify the HS256
 * signature with the configured stack secret, the target audience and the
 * expiry before accepting a write. Identity and role are carried in the
 * signed claims; request bodies are never consulted for either value.
 */

import { createHmac, timingSafeEqual } from "node:crypto";

export const STACK_TOKEN_TTL_SECONDS = 300;
export const STACK_TOKEN_ALGORITHM = "HS256";
const STACK_TOKEN_TYPE = "JWT";

export type StackTokenRole = "owner" | "orchestrator" | "agent" | "read-only";

export interface StackTokenClaims {
  sub: string;
  role: StackTokenRole;
  aud: string;
  act: string;
  iat?: number;
  exp?: number;
}

export interface VerifiedStackToken extends StackTokenClaims {
  iat: number;
  exp: number;
}

function encode(value: unknown): string {
  return Buffer.from(JSON.stringify(value), "utf8").toString("base64url");
}

function decode<T>(value: string, label: string): T {
  try {
    return JSON.parse(Buffer.from(value, "base64url").toString("utf8")) as T;
  } catch {
    throw new Error(`malformed stack token ${label}`);
  }
}

function signature(input: string, secret: string): string {
  return createHmac("sha256", secret).update(input).digest("base64url");
}

function requireLabel(value: unknown, field: string): string {
  if (typeof value !== "string" || value.trim() === "" || value.length > 200) {
    throw new Error(`invalid stack token ${field}`);
  }
  return value;
}

function requireSecret(secret: string): void {
  if (typeof secret !== "string" || secret.length < 16) {
    throw new Error("stack token secret must be at least 16 characters");
  }
}

function normalizeClaims(input: StackTokenClaims, nowSeconds: number): VerifiedStackToken {
  const sub = requireLabel(input.sub, "sub");
  const aud = requireLabel(input.aud, "aud");
  const act = requireLabel(input.act, "act");
  const roles: StackTokenRole[] = ["owner", "orchestrator", "agent", "read-only"];
  if (!roles.includes(input.role)) throw new Error("invalid stack token role");
  const iat = input.iat ?? nowSeconds;
  const exp = input.exp ?? iat + STACK_TOKEN_TTL_SECONDS;
  if (!Number.isSafeInteger(iat) || !Number.isSafeInteger(exp) || exp <= iat) {
    throw new Error("invalid stack token timestamps");
  }
  if (exp - iat > STACK_TOKEN_TTL_SECONDS) throw new Error("stack token ttl exceeds 5 minutes");
  return { sub, role: input.role, aud, act, iat, exp };
}

/** Mint a signed token; only the aggregator should call this function. */
export function mintStackToken(input: StackTokenClaims, secret: string, nowMs = Date.now()): string {
  requireSecret(secret);
  const nowSeconds = Math.floor(nowMs / 1000);
  const claims = normalizeClaims(input, nowSeconds);
  const header = encode({ alg: STACK_TOKEN_ALGORITHM, typ: STACK_TOKEN_TYPE });
  const payload = encode(claims);
  const body = `${header}.${payload}`;
  return `${body}.${signature(body, secret)}`;
}

/** Verify signature, audience, role and the five-minute expiry bound. */
export function verifyStackToken(
  token: string,
  secret: string,
  audience: string,
  nowMs = Date.now(),
): VerifiedStackToken {
  requireSecret(secret);
  const parts = token.split(".");
  if (parts.length !== 3) throw new Error("malformed stack token");
  const [encodedHeader, encodedPayload, encodedSignature] = parts;
  const body = `${encodedHeader}.${encodedPayload}`;
  const expected = Buffer.from(signature(body, secret), "base64url");
  const actual = Buffer.from(encodedSignature, "base64url");
  if (expected.length !== actual.length || !timingSafeEqual(expected, actual)) {
    throw new Error("stack token signature invalid");
  }
  const header = decode<{ alg?: unknown; typ?: unknown }>(encodedHeader, "header");
  if (header.alg !== STACK_TOKEN_ALGORITHM || header.typ !== STACK_TOKEN_TYPE) {
    throw new Error("unsupported stack token header");
  }
  const raw = decode<Record<string, unknown>>(encodedPayload, "payload");
  // Verification requires explicit timestamps: normalizeClaims substitutes
  // minting defaults for absent iat/exp, which would let a correctly signed
  // timestamp-less token verify indefinitely instead of expiring.
  if (typeof raw.iat !== "number" || typeof raw.exp !== "number") {
    throw new Error("invalid stack token timestamps");
  }
  const claims = normalizeClaims(
    {
      sub: raw.sub as string,
      role: raw.role as StackTokenRole,
      aud: raw.aud as string,
      act: raw.act as string,
      iat: raw.iat as number,
      exp: raw.exp as number,
    },
    Math.floor(nowMs / 1000),
  );
  if (claims.aud !== audience) throw new Error("stack token audience mismatch");
  const nowSeconds = Math.floor(nowMs / 1000);
  if (claims.exp <= nowSeconds) throw new Error("stack token expired");
  if (claims.iat > nowSeconds + 60) throw new Error("stack token issued-at is in the future");
  return claims;
}
