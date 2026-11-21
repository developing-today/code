// Secret-path auth for the message bridge.
//
// Every route that reads or writes messages carries the access token as
// a path segment (/mcp/<token>, /p/<token>, /api/<token>/...). The token
// is the Worker secret ACCESS_TOKEN (set by deploy.mjs from Bitwarden,
// never in git). It is the owner's token: every route, and /mcp on the
// Claude channel.
//
// CHATGPT_TOKEN is a second Worker secret, also from Bitwarden, held by
// the ChatGPT connector's private Site. It opens /mcp/<token> only, on
// the ChatGPT channel: it can't read Claude's messages, the page or the
// JSON API. A wrong or missing token gets the same plain 404 as an
// unknown path, so the routes don't advertise themselves, and an MCP
// client given a token URL is never pushed into OAuth discovery by a
// 401. The bare address (/ and /mcp, no token) uses OAuth: oauth.js.

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

const usable = (t) => typeof t === "string" && t.length >= MIN_TOKEN_LENGTH;

/** True when the Worker has a usable access token configured. */
export function tokenConfigured(env) {
  return usable(env.ACCESS_TOKEN);
}

/** True when the ChatGPT connector's token is configured (and distinct). */
export function chatgptConfigured(env) {
  return usable(env.CHATGPT_TOKEN) && !safeEqual(env.CHATGPT_TOKEN, env.ACCESS_TOKEN);
}

/**
 * Splits a request path into { area, rest, role, channel } when its
 * second segment is a known token: "/mcp/<token>" -> { area: "mcp",
 * rest: [] }, "/api/<token>/messages" -> { area: "api", rest:
 * ["messages"] }. role is "owner" for ACCESS_TOKEN (channel "claude" for
 * /mcp) and "chatgpt" for CHATGPT_TOKEN, which matches /mcp only.
 * Returns null when the token is missing or wrong.
 */
export function matchTokenPath(pathname, env) {
  const parts = pathname.split("/").filter(Boolean);
  if (parts.length < 2) return null;
  const [area, token, ...rest] = parts;
  // Compare against both tokens every time, so timing doesn't say which.
  const owner = tokenConfigured(env) && safeEqual(token, env.ACCESS_TOKEN);
  const chatgpt = chatgptConfigured(env) && safeEqual(token, env.CHATGPT_TOKEN);
  if (owner) return { area, rest, role: "owner", channel: "claude" };
  if (chatgpt && area === "mcp") return { area, rest, role: "chatgpt", channel: "chatgpt" };
  return null;
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
