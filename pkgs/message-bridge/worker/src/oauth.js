// OAuth 2.1 sign-in for the bare address (https://bridge.security.cab
// and /mcp), so a connector added with just the host works.
//
// Claude's connector client discovers the server through
// /.well-known/oauth-protected-resource, registers itself with dynamic
// client registration (POST /register), sends the owner to /authorize
// to approve, then trades the code (PKCE S256 only) at /token for a
// bearer token it sends with every MCP call.
//
// Nothing is stored: client ids, the pending request, codes, access and
// refresh tokens are all HMAC-signed blobs. The key is derived from
// ACCESS_TOKEN, so rotating that token revokes every grant at once.
//
// Approving needs the owner: a browser that has opened the history page
// (/p/<token>) carries a signed owner cookie and gets a one-tap Allow;
// any other browser pastes the page link or the token. The ChatGPT
// token approves for the ChatGPT channel only. The path-token routes
// (/mcp/<token> and the rest) are unchanged and never answer 401.

import { chatgptConfigured, safeEqual, tokenConfigured } from "./auth.js";
import { escapeHtml } from "./page.js";

const enc = new TextEncoder();
const dec = new TextDecoder();

export const OWNER_COOKIE = "__Host-mb_owner";
const OWNER_COOKIE_TTL = 365 * 24 * 3600;
const REQUEST_TTL = 15 * 60;
const CODE_TTL = 5 * 60;
export const ACCESS_TTL = 24 * 3600;
const REFRESH_TTL = 365 * 24 * 3600;
const MAX_REDIRECTS = 5;
const MAX_URI = 512;
const MAX_STATE = 1024;
const MAX_BODY = 16 * 1024;
const SCOPE = "messages";

const nowSec = () => Math.floor(Date.now() / 1000);

function b64u(bytes) {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function fromB64u(s) {
  if (!/^[A-Za-z0-9_-]*$/.test(s)) throw new Error("bad base64url");
  const bin = atob(s.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (s.length % 4)) % 4));
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
}

let cachedKey = { token: null, key: null };
async function keyFor(env) {
  if (cachedKey.token === env.ACCESS_TOKEN) return cachedKey.key;
  const raw = await crypto.subtle.digest("SHA-256", enc.encode(`message-bridge/oauth-key/v1:${env.ACCESS_TOKEN}`));
  const key = await crypto.subtle.importKey("raw", raw, { name: "HMAC", hash: "SHA-256" }, false, ["sign", "verify"]);
  cachedKey = { token: env.ACCESS_TOKEN, key };
  return key;
}

async function mac(env, text) {
  return b64u(new Uint8Array(await crypto.subtle.sign("HMAC", await keyFor(env), enc.encode(text))));
}

/** Signs a payload; typ keeps one kind of blob from passing as another. */
export async function seal(env, payload) {
  const body = b64u(enc.encode(JSON.stringify(payload)));
  return `${body}.${await mac(env, body)}`;
}

/** Returns the payload when the signature, typ and expiry check out. */
export async function unseal(env, value, typ) {
  if (typeof value !== "string" || value.length > 8192) return null;
  const parts = value.split(".");
  if (parts.length !== 2 || !parts[0] || !parts[1]) return null;
  let payload;
  try {
    const ok = await crypto.subtle.verify("HMAC", await keyFor(env), fromB64u(parts[1]), enc.encode(parts[0]));
    if (!ok) return null;
    payload = JSON.parse(dec.decode(fromB64u(parts[0])));
  } catch {
    return null;
  }
  if (!payload || typeof payload !== "object" || payload.typ !== typ) return null;
  if (typeof payload.exp === "number" && payload.exp < nowSec()) return null;
  return payload;
}

const clientSecretFor = (env, clientId) => mac(env, `client-secret:${clientId}`);

async function pkceChallenge(verifier) {
  return b64u(new Uint8Array(await crypto.subtle.digest("SHA-256", enc.encode(verifier))));
}

// --- responses -------------------------------------------------------

const NO_STORE = { "cache-control": "no-store", pragma: "no-cache", "x-content-type-options": "nosniff" };

const jsonRes = (data, status = 200, extra = {}) =>
  new Response(JSON.stringify(data), {
    status,
    headers: { "content-type": "application/json", ...NO_STORE, ...extra },
  });

const CORS = {
  "access-control-allow-origin": "*",
  "access-control-allow-headers": "authorization, content-type, mcp-protocol-version",
  "access-control-allow-methods": "GET, POST, OPTIONS",
};

const oauthError = (error, description, status = 400) =>
  jsonRes({ error, error_description: description }, status, CORS);

function redirectTo(redirectUri, params) {
  const u = new URL(redirectUri);
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== null) u.searchParams.set(k, v);
  return new Response(null, { status: 302, headers: { location: u.toString(), ...NO_STORE } });
}

// --- metadata --------------------------------------------------------

function authServerMetadata(origin) {
  return {
    issuer: origin,
    authorization_endpoint: `${origin}/authorize`,
    token_endpoint: `${origin}/token`,
    registration_endpoint: `${origin}/register`,
    response_types_supported: ["code"],
    grant_types_supported: ["authorization_code", "refresh_token"],
    code_challenge_methods_supported: ["S256"],
    token_endpoint_auth_methods_supported: ["none", "client_secret_basic", "client_secret_post"],
    scopes_supported: [SCOPE],
    authorization_response_iss_parameter_supported: true,
    service_documentation: `${origin}/`,
  };
}

/** The resource the bare address serves: the origin, or origin + /mcp. */
const resourceFor = (origin, suffix) => (suffix === "/mcp" ? `${origin}/mcp` : origin);

// --- registration ----------------------------------------------------

function validRedirect(uri) {
  if (typeof uri !== "string" || uri.length > MAX_URI) return false;
  let u;
  try {
    u = new URL(uri);
  } catch {
    return false;
  }
  if (u.hash || u.username || u.password) return false;
  if (u.protocol === "https:") return true;
  return u.protocol === "http:" && ["localhost", "127.0.0.1", "[::1]"].includes(u.hostname);
}

async function register(request, env) {
  let data;
  try {
    const text = await request.text();
    if (text.length > MAX_BODY) return oauthError("invalid_client_metadata", "request too large");
    data = JSON.parse(text);
  } catch {
    return oauthError("invalid_client_metadata", "body must be JSON");
  }
  const uris = data?.redirect_uris;
  if (!Array.isArray(uris) || uris.length === 0 || uris.length > MAX_REDIRECTS || !uris.every(validRedirect)) {
    return oauthError("invalid_redirect_uri", "redirect_uris must be 1 to 5 https (or localhost) URLs");
  }
  const method = data.token_endpoint_auth_method ?? "client_secret_basic";
  if (!["none", "client_secret_basic", "client_secret_post"].includes(method)) {
    return oauthError("invalid_client_metadata", "unsupported token_endpoint_auth_method");
  }
  const name = typeof data.client_name === "string" ? data.client_name.trim().slice(0, 80) : "";
  const clientId = await seal(env, { typ: "client", r: uris, m: method, n: name, iat: nowSec() });
  const out = {
    client_id: clientId,
    client_id_issued_at: nowSec(),
    client_name: name || undefined,
    redirect_uris: uris,
    grant_types: ["authorization_code", "refresh_token"],
    response_types: ["code"],
    token_endpoint_auth_method: method,
    scope: SCOPE,
  };
  if (method !== "none") {
    out.client_secret = await clientSecretFor(env, clientId);
    out.client_secret_expires_at = 0;
  }
  return jsonRes(out, 201, CORS);
}

// --- authorize -------------------------------------------------------

const PAGE_STYLE = `body{font:16px/1.45 system-ui,sans-serif;max-width:32rem;margin:2rem auto;padding:0 1rem;color:#222;background:#fff}
h1{font-size:1.3rem}.box{border:1px solid #ccc;border-radius:.5rem;padding:1rem}.err{color:#b00020}
input[type=password]{width:100%;box-sizing:border-box;padding:.5rem;font:inherit}
button{font:inherit;padding:.5rem 1rem;margin:.75rem .5rem 0 0;border-radius:.4rem;border:1px solid #888;background:#f4f4f4}
button.primary{background:#1a5fd0;color:#fff;border-color:#1a5fd0}
@media (prefers-color-scheme:dark){body{background:#151515;color:#eee}.box{border-color:#444}button{background:#2a2a2a;color:#eee}.err{color:#ff7a8a}}`;

function htmlPage(title, inner, status = 200, formTarget = null) {
  const formAction = formTarget ? `'self' ${formTarget}` : "'self'";
  return new Response(
    `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>${escapeHtml(title)}</title><style>${PAGE_STYLE}</style></head><body>${inner}</body></html>`,
    {
      status,
      headers: {
        "content-type": "text/html; charset=utf-8",
        "content-security-policy": `default-src 'none'; style-src 'unsafe-inline'; form-action ${formAction}; frame-ancestors 'none'; base-uri 'none'`,
        "referrer-policy": "no-referrer",
        "x-robots-tag": "noindex, nofollow",
        ...NO_STORE,
      },
    },
  );
}

const errorPage = (message, status = 400) =>
  htmlPage("Message Bridge", `<h1>Message Bridge</h1><p class="err">${escapeHtml(message)}</p>`, status);

async function cookieOwner(request, env) {
  const cookies = request.headers.get("cookie") ?? "";
  for (const part of cookies.split(";")) {
    const [k, ...v] = part.trim().split("=");
    if (k === OWNER_COOKIE) return !!(await unseal(env, v.join("="), "owner"));
  }
  return false;
}

/** The Set-Cookie header that marks this browser as the owner's. */
export async function ownerCookie(env) {
  const value = await seal(env, { typ: "owner", exp: nowSec() + OWNER_COOKIE_TTL });
  return `${OWNER_COOKIE}=${value}; Path=/; Max-Age=${OWNER_COOKIE_TTL}; Secure; HttpOnly; SameSite=Lax`;
}

/** Looks the client up and settles the redirect URI, or says why not. */
async function clientAndRedirect(env, clientId, redirectUri) {
  const client = await unseal(env, clientId, "client");
  if (!client) return { error: "This connection request is from an unknown app. Start again from Claude." };
  const uri = redirectUri || (client.r.length === 1 ? client.r[0] : null);
  if (!uri || !client.r.includes(uri)) return { error: "This connection request has a return address the app didn't register." };
  return { client, uri };
}

function approvalPage({ client, uri, req, owner, error, origin }) {
  const app = client.n ? escapeHtml(client.n) : "An app";
  const dest = escapeHtml(new URL(uri).host);
  const proof = owner
    ? `<p>This browser is signed in as the bridge's owner.</p>`
    : `<p><label for="secret">To approve, paste your bridge page link or access token:</label></p>
<input id="secret" name="secret" type="password" autocomplete="off" required>`;
  const inner = `<h1>Connect to Message Bridge</h1>
<div class="box"><p><b>${app}</b> wants to read and send messages on your message bridge at ${escapeHtml(new URL(origin).host)}.</p>
<p>Approving sends you back to <b>${dest}</b>.</p>
${error ? `<p class="err">${escapeHtml(error)}</p>` : ""}
<form method="post" action="/authorize">
<input type="hidden" name="req" value="${escapeHtml(req)}">
${proof}
<button class="primary" type="submit" name="action" value="allow">Allow</button>
<button type="submit" name="action" value="deny" formnovalidate>Deny</button>
</form></div>`;
  return htmlPage("Connect to Message Bridge", inner, error ? 401 : 200, new URL(uri).origin);
}

async function authorizeGet(request, env, url) {
  const p = url.searchParams;
  const found = await clientAndRedirect(env, p.get("client_id"), p.get("redirect_uri"));
  if (found.error) return errorPage(found.error);
  const { client, uri } = found;
  const state = p.get("state") ?? undefined;
  if (state && state.length > MAX_STATE) return errorPage("The request's state is too long.");
  const fail = (error, description) => redirectTo(uri, { error, error_description: description, state, iss: url.origin });
  if (p.get("response_type") !== "code") return fail("unsupported_response_type", "only code is supported");
  const challenge = p.get("code_challenge");
  if (!challenge || !/^[A-Za-z0-9_-]{43,128}$/.test(challenge)) return fail("invalid_request", "code_challenge is required");
  if ((p.get("code_challenge_method") ?? "plain") !== "S256") return fail("invalid_request", "code_challenge_method must be S256");
  const req = await seal(env, {
    typ: "req",
    c: p.get("client_id"),
    r: uri,
    cc: challenge,
    s: state,
    exp: nowSec() + REQUEST_TTL,
  });
  return approvalPage({ client, uri, req, owner: await cookieOwner(request, env), origin: url.origin });
}

/** Pulls a token out of a pasted page link, connector URL or bare token. */
function pastedToken(text) {
  const s = String(text ?? "").trim();
  const m = s.match(/\/(?:p|mcp|api)\/([^/?#\s]+)/);
  return m ? m[1] : s;
}

async function authorizePost(request, env, url) {
  // Same-origin form posts only (the owner cookie is SameSite=Lax too).
  if (request.headers.get("origin") !== url.origin) return errorPage("This form can only be sent from its own page.", 403);
  const text = await request.text();
  if (text.length > MAX_BODY) return errorPage("Request too large.", 413);
  const form = new URLSearchParams(text);
  const req = await unseal(env, form.get("req"), "req");
  if (!req) return errorPage("This approval page expired. Start the connection again from Claude.");
  const found = await clientAndRedirect(env, req.c, req.r);
  if (found.error) return errorPage(found.error);
  const { client, uri } = found;
  if (form.get("action") !== "allow") {
    return redirectTo(uri, { error: "access_denied", error_description: "the owner denied access", state: req.s, iss: url.origin });
  }
  let channel = null;
  let setCookie = null;
  const secret = form.get("secret");
  if (secret) {
    const token = pastedToken(secret);
    if (tokenConfigured(env) && safeEqual(token, env.ACCESS_TOKEN)) {
      channel = "claude";
      setCookie = await ownerCookie(env);
    } else if (chatgptConfigured(env) && safeEqual(token, env.CHATGPT_TOKEN)) {
      channel = "chatgpt";
    }
  } else if (await cookieOwner(request, env)) {
    channel = "claude";
  }
  if (!channel) {
    const again = await seal(env, { ...req, exp: nowSec() + REQUEST_TTL });
    return approvalPage({
      client,
      uri,
      req: again,
      owner: false,
      origin: url.origin,
      error: secret ? "That isn't this bridge's page link or access token." : "Paste your bridge page link or access token.",
    });
  }
  const code = await seal(env, { typ: "code", c: req.c, r: uri, cc: req.cc, ch: channel, exp: nowSec() + CODE_TTL });
  const res = redirectTo(uri, { code, state: req.s, iss: url.origin });
  if (setCookie) res.headers.append("set-cookie", setCookie);
  return res;
}

// --- token -----------------------------------------------------------

async function issueTokens(env, clientId, channel) {
  return jsonRes(
    {
      access_token: await seal(env, { typ: "at", ch: channel, exp: nowSec() + ACCESS_TTL }),
      token_type: "Bearer",
      expires_in: ACCESS_TTL,
      refresh_token: await seal(env, { typ: "rt", c: clientId, ch: channel, exp: nowSec() + REFRESH_TTL }),
      scope: SCOPE,
    },
    200,
    CORS,
  );
}

async function token(request, env) {
  const text = await request.text();
  if (text.length > MAX_BODY) return oauthError("invalid_request", "request too large");
  const type = request.headers.get("content-type") ?? "";
  let form;
  if (type.includes("application/json")) {
    try {
      form = new URLSearchParams(Object.entries(JSON.parse(text) ?? {}).map(([k, v]) => [k, String(v)]));
    } catch {
      return oauthError("invalid_request", "body must be form-encoded");
    }
  } else {
    form = new URLSearchParams(text);
  }
  let clientId = form.get("client_id");
  let secret = form.get("client_secret");
  const basic = (request.headers.get("authorization") ?? "").match(/^Basic\s+(.+)$/i);
  if (basic) {
    try {
      const [id, ...rest] = atob(basic[1]).split(":");
      clientId = decodeURIComponent(id);
      secret = decodeURIComponent(rest.join(":"));
    } catch {
      return oauthError("invalid_client", "bad client credentials", 401);
    }
  }
  const client = await unseal(env, clientId, "client");
  if (!client) return oauthError("invalid_client", "unknown client", 401);
  if (client.m !== "none" && !safeEqual(secret ?? "", await clientSecretFor(env, clientId))) {
    return oauthError("invalid_client", "bad client credentials", 401);
  }
  const grant = form.get("grant_type");
  if (grant === "authorization_code") {
    const code = await unseal(env, form.get("code"), "code");
    if (!code || code.c !== clientId) return oauthError("invalid_grant", "the code is invalid or expired");
    const redirectUri = form.get("redirect_uri");
    if (redirectUri && redirectUri !== code.r) return oauthError("invalid_grant", "redirect_uri doesn't match");
    const verifier = form.get("code_verifier") ?? "";
    if (!/^[A-Za-z0-9._~-]{43,128}$/.test(verifier) || (await pkceChallenge(verifier)) !== code.cc) {
      return oauthError("invalid_grant", "code_verifier doesn't match");
    }
    return issueTokens(env, clientId, code.ch);
  }
  if (grant === "refresh_token") {
    const rt = await unseal(env, form.get("refresh_token"), "rt");
    if (!rt || rt.c !== clientId) return oauthError("invalid_grant", "the refresh token is invalid or expired");
    return issueTokens(env, clientId, rt.ch);
  }
  return oauthError("unsupported_grant_type", "use authorization_code or refresh_token");
}

// --- entry points ----------------------------------------------------

/** The channel a request's bearer token grants, or null. */
export async function bearerChannel(request, env) {
  if (!tokenConfigured(env)) return null;
  const m = (request.headers.get("authorization") ?? "").match(/^Bearer\s+(\S+)$/i);
  if (!m) return null;
  const at = await unseal(env, m[1], "at");
  if (!at) return null;
  if (at.ch === "chatgpt" && !chatgptConfigured(env)) return null;
  return at.ch === "chatgpt" ? "chatgpt" : "claude";
}

/** 401 that points an MCP client at the protected-resource metadata. */
export function unauthorized(request, url) {
  const suffix = url.pathname.startsWith("/mcp") ? "/mcp" : "";
  const hadToken = /^Bearer\s/i.test(request.headers.get("authorization") ?? "");
  const params = [
    ...(hadToken ? ['error="invalid_token"'] : []),
    `resource_metadata="${url.origin}/.well-known/oauth-protected-resource${suffix}"`,
  ];
  const browser = (request.headers.get("accept") ?? "").includes("text/html") && request.method === "GET";
  return new Response(
    browser
      ? "This is a Message Bridge connector address. Add it to Claude as a custom connector and approve it there.\n"
      : JSON.stringify({ error: "unauthorized", error_description: "sign in with OAuth" }),
    {
      status: 401,
      headers: {
        "content-type": browser ? "text/plain; charset=utf-8" : "application/json",
        "www-authenticate": `Bearer ${params.join(", ")}`,
        ...CORS,
        "access-control-expose-headers": "www-authenticate",
        ...NO_STORE,
      },
    },
  );
}

const WELL_KNOWN = /^\/\.well-known\/(oauth-protected-resource|oauth-authorization-server|openid-configuration)(\/mcp)?\/?$/;

/**
 * Handles the OAuth endpoints; returns null for any other path. Only
 * active when the Worker has an access token.
 */
export async function handleOAuth(request, env, url) {
  if (!tokenConfigured(env)) return null;
  const path = url.pathname;
  const wk = path.match(WELL_KNOWN);
  if (!wk && !["/register", "/authorize", "/token"].includes(path)) return null;
  if (request.method === "OPTIONS") return new Response(null, { status: 204, headers: CORS });
  if (wk) {
    if (request.method !== "GET" && request.method !== "HEAD") return oauthError("invalid_request", "use GET", 405);
    if (wk[1] === "oauth-protected-resource") {
      return jsonRes(
        {
          resource: resourceFor(url.origin, wk[2]),
          authorization_servers: [url.origin],
          bearer_methods_supported: ["header"],
          scopes_supported: [SCOPE],
          resource_name: "Message Bridge",
        },
        200,
        CORS,
      );
    }
    return jsonRes(authServerMetadata(url.origin), 200, CORS);
  }
  if (path === "/register") return request.method === "POST" ? register(request, env) : oauthError("invalid_request", "use POST", 405);
  if (path === "/token") return request.method === "POST" ? token(request, env) : oauthError("invalid_request", "use POST", 405);
  if (request.method === "GET" || request.method === "HEAD") return authorizeGet(request, env, url);
  if (request.method === "POST") return authorizePost(request, env, url);
  return errorPage("Method not allowed.", 405);
}
