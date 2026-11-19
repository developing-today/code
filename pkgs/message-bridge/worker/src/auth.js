// Secret-path auth for the message bridge.
//
// Every route that reads or writes messages carries the access token as
// a path segment (/mcp/<token>, /p/<token>, /api/<token>/...). The token
// is the Worker secret ACCESS_TOKEN (set by deploy.mjs from Bitwarden,
// never in git). A wrong or missing token gets the same plain 404 as an
// unknown path, so the routes don't advertise themselves, and an MCP
// client is never pushed into an OAuth discovery dance by a 401.

export const MIN_TOKEN_LENGTH = 32;

/** Constant-time string compare (length is not secret: it's fixed). */
export function safeEqual(a, b) {
  if (typeof a !== "string" || typeof b !== "string") return false;
  const enc = new TextEncoder();
  const x = enc.encode(a);
  const y = enc.encode(b);
  let diff = x.length ^ y.length;
  const n = Math.max(x.length, y.length);
  for (let i = 0; i < n; i++) diff |= (x[i] ?? 0) ^ (y[i] ?? 0);
  return diff === 0;
}

/** True when the Worker has a usable access token configured. */
export function tokenConfigured(env) {
  return typeof env.ACCESS_TOKEN === "string" && env.ACCESS_TOKEN.length >= MIN_TOKEN_LENGTH;
}

/**
 * Splits a request path into { area, rest } when its second segment is
 * the access token: "/mcp/<token>" -> { area: "mcp", rest: [] },
 * "/api/<token>/messages" -> { area: "api", rest: ["messages"] }.
 * Returns null when the token is missing or wrong.
 */
export function matchTokenPath(pathname, env) {
  if (!tokenConfigured(env)) return null;
  const parts = pathname.split("/").filter(Boolean);
  if (parts.length < 2) return null;
  const [area, token, ...rest] = parts;
  if (!safeEqual(token, env.ACCESS_TOKEN)) return null;
  return { area, rest };
}

/**
 * A short public fingerprint of the access token, reported by /healthz so
 * a deploy can check the live Worker carries the token it expects (a
 * code-only upload must keep the live one) without the token itself
 * leaving the Worker. 64 bits of a SHA-256 over a 256-bit random token
 * reveal nothing usable about it. deploy.mjs computes the same value.
 */
export async function tokenFingerprint(token) {
  const data = new TextEncoder().encode(`message-bridge/token-fingerprint/v1:${token}`);
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", data));
  return Array.from(digest.subarray(0, 8), (b) => b.toString(16).padStart(2, "0")).join("");
}
