// OAuth 2.1 authorization-server front for the mcpx
// Worker (#13657). Hosted MCP clients (Claude, ChatGPT) connect to
// https://mcpx.security.cab/mcp from their own servers, so they
// cannot pass a Cloudflare Access login at the /mcp door. This module
// gives the door the standard MCP authorization shape instead:
//
//   - discovery documents (RFC 9728 / RFC 8414) — public
//   - dynamic client registration (RFC 7591) — public
//   - /authorize — the one path still behind Cloudflare Access; only
//     the owner's Google identity gets in, and the Worker re-verifies
//     the Access JWT itself (signature, audience, email) before
//     rendering the approval page
//   - /token — authorization-code exchange with PKCE S256, plus
//     rotating refresh tokens
//
// State lives in the OAUTH_KV namespace. Authorization codes, access
// tokens and refresh tokens are stored under the SHA-256 of the
// presented value, never in the clear.

export interface OAuthEnv {
  OAUTH_KV: KVNamespace;
  // Audience (AUD) of the Access application scoped to /authorize.
  ACCESS_AUD: string;
  // Access team domain hosting the JWKS, e.g.
  // "snowy-sun-58a9.cloudflareaccess.com".
  ACCESS_TEAM_DOMAIN: string;
}

export const ISSUER = "https://mcpx.security.cab";
export const RESOURCE = "https://mcpx.security.cab/mcp";
export const SCOPE = "mcpx";
export const OWNER_EMAIL = "drewrypope@gmail.com";

const CODE_TTL_S = 600;
const PENDING_TTL_S = 600;
const ACCESS_TOKEN_TTL_S = 3600;
const REFRESH_TOKEN_TTL_S = 30 * 24 * 3600;

interface ClientRecord {
  client_name?: string;
  redirect_uris: string[];
  grant_types: string[];
  response_types: string[];
  token_endpoint_auth_method: string;
  created_at: number;
}

interface PendingRecord {
  client_id: string;
  redirect_uri: string;
  state?: string;
  scope: string;
  code_challenge: string;
}

interface CodeRecord {
  client_id: string;
  redirect_uri: string;
  code_challenge: string;
  scope: string;
  exp: number;
}

interface TokenRecord {
  client_id: string;
  scope: string;
  exp: number;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function oauthError(error: string, description: string, status = 400): Response {
  return json({ error, error_description: description }, status);
}

const encoder = new TextEncoder();

export function base64urlEncode(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

function base64urlDecode(value: string): Uint8Array {
  const padded = value.replaceAll("-", "+").replaceAll("_", "/").padEnd(Math.ceil(value.length / 4) * 4, "=");
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

export async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", encoder.encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function sha256Base64url(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", encoder.encode(text));
  return base64urlEncode(new Uint8Array(digest));
}

function randomToken(bytes = 32): string {
  return base64urlEncode(crypto.getRandomValues(new Uint8Array(bytes)));
}

function randomHex(bytes = 16): string {
  return [...crypto.getRandomValues(new Uint8Array(bytes))]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

// Constant-time comparison for secret material. Length is not secret
// for a fixed-size bearer, so a length mismatch returns early.
export function timingSafeEqual(a: string, b: string): boolean {
  const ab = encoder.encode(a);
  const bb = encoder.encode(b);
  if (ab.length !== bb.length) return false;
  let diff = 0;
  for (let i = 0; i < ab.length; i++) diff |= ab[i] ^ bb[i];
  return diff === 0;
}

function escapeHtml(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

// ---------------------------------------------------------------------------
// Discovery (RFC 9728 / RFC 8414)

export function protectedResourceMetadata(): Response {
  return json({
    resource: RESOURCE,
    authorization_servers: [ISSUER],
    bearer_methods_supported: ["header"],
    scopes_supported: [SCOPE],
  });
}

export function authorizationServerMetadata(): Response {
  return json({
    issuer: ISSUER,
    authorization_endpoint: `${ISSUER}/authorize`,
    token_endpoint: `${ISSUER}/token`,
    registration_endpoint: `${ISSUER}/register`,
    response_types_supported: ["code"],
    grant_types_supported: ["authorization_code", "refresh_token"],
    code_challenge_methods_supported: ["S256"],
    token_endpoint_auth_methods_supported: ["none"],
    scopes_supported: [SCOPE],
    service_documentation: "https://github.com/developing-today/code/issues/13657",
  });
}

// ---------------------------------------------------------------------------
// Dynamic client registration (RFC 7591)

function isRegistrableRedirectUri(value: string): boolean {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return false;
  }
  if (url.protocol === "https:") return true;
  // Loopback http is allowed for native / desktop clients (RFC 8252).
  if (url.protocol === "http:") {
    return url.hostname === "localhost" || url.hostname === "127.0.0.1" || url.hostname === "[::1]";
  }
  return false;
}

export async function registerClient(request: Request, env: OAuthEnv): Promise<Response> {
  let body: Record<string, unknown>;
  try {
    body = (await request.json()) as Record<string, unknown>;
  } catch {
    return oauthError("invalid_client_metadata", "request body must be a JSON object");
  }
  if (body === null || typeof body !== "object" || Array.isArray(body)) {
    return oauthError("invalid_client_metadata", "request body must be a JSON object");
  }

  const redirectUris = body.redirect_uris;
  if (
    !Array.isArray(redirectUris) ||
    redirectUris.length === 0 ||
    !redirectUris.every((u) => typeof u === "string" && isRegistrableRedirectUri(u))
  ) {
    return oauthError(
      "invalid_redirect_uri",
      "redirect_uris must be a non-empty array of absolute https (or loopback http) URIs",
    );
  }
  if (body.token_endpoint_auth_method !== undefined && body.token_endpoint_auth_method !== "none") {
    return oauthError("invalid_client_metadata", "only token_endpoint_auth_method \"none\" (public client) is supported");
  }
  const grantTypes = Array.isArray(body.grant_types) ? body.grant_types.filter((g) => typeof g === "string") : ["authorization_code"];
  const responseTypes = Array.isArray(body.response_types) ? body.response_types.filter((g) => typeof g === "string") : ["code"];
  const clientName = typeof body.client_name === "string" ? body.client_name : undefined;

  const clientId = randomHex(16);
  const record: ClientRecord = {
    client_name: clientName,
    redirect_uris: redirectUris as string[],
    grant_types: grantTypes as string[],
    response_types: responseTypes as string[],
    token_endpoint_auth_method: "none",
    created_at: Math.floor(Date.now() / 1000),
  };
  await env.OAUTH_KV.put(`client:${clientId}`, JSON.stringify(record));

  return json(
    {
      client_id: clientId,
      client_id_issued_at: record.created_at,
      client_name: clientName,
      redirect_uris: record.redirect_uris,
      grant_types: record.grant_types,
      response_types: record.response_types,
      token_endpoint_auth_method: "none",
    },
    201,
  );
}

async function getClient(env: OAuthEnv, clientId: string | null): Promise<ClientRecord | null> {
  if (!clientId) return null;
  return env.OAUTH_KV.get<ClientRecord>(`client:${clientId}`, "json");
}

// ---------------------------------------------------------------------------
// Cloudflare Access JWT verification for /authorize.
//
// Access is the enforcement point (the /authorize app only lets the
// owner's identity through); this re-verification is defense in depth
// so the approval page never renders on a forged or stray request.

interface AccessClaims {
  email?: string;
  aud?: string | string[];
  exp?: number;
  iss?: string;
  [key: string]: unknown;
}

let jwksCache: { teamDomain: string; keys: JsonWebKey[]; fetchedAt: number } | null = null;

async function fetchJwks(teamDomain: string): Promise<JsonWebKey[] | null> {
  const now = Date.now();
  if (jwksCache && jwksCache.teamDomain === teamDomain && now - jwksCache.fetchedAt < 300_000) {
    return jwksCache.keys;
  }
  const res = await fetch(`https://${teamDomain}/cdn-cgi/access/certs`);
  if (!res.ok) return null;
  const body = (await res.json()) as { keys?: JsonWebKey[] };
  if (!Array.isArray(body.keys)) return null;
  jwksCache = { teamDomain, keys: body.keys, fetchedAt: now };
  return body.keys;
}

export async function verifyAccessAssertion(request: Request, env: OAuthEnv): Promise<AccessClaims | null> {
  const jwt = request.headers.get("Cf-Access-Jwt-Assertion");
  if (!jwt) return null;
  const parts = jwt.split(".");
  if (parts.length !== 3) return null;

  let header: { alg?: string; kid?: string };
  let claims: AccessClaims;
  try {
    header = JSON.parse(new TextDecoder().decode(base64urlDecode(parts[0]))) as typeof header;
    claims = JSON.parse(new TextDecoder().decode(base64urlDecode(parts[1]))) as AccessClaims;
  } catch {
    return null;
  }
  if (header.alg !== "RS256" || !header.kid) return null;

  const keys = await fetchJwks(env.ACCESS_TEAM_DOMAIN).catch(() => null);
  if (!keys) return null;
  const jwk = keys.find((k) => (k as { kid?: string }).kid === header.kid);
  if (!jwk) return null;

  let verified = false;
  try {
    const key = await crypto.subtle.importKey(
      "jwk",
      jwk,
      { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
      false,
      ["verify"],
    );
    verified = await crypto.subtle.verify(
      "RSASSA-PKCS1-v1_5",
      key,
      base64urlDecode(parts[2]),
      encoder.encode(`${parts[0]}.${parts[1]}`),
    );
  } catch {
    return null;
  }
  if (!verified) return null;

  const now = Math.floor(Date.now() / 1000);
  if (typeof claims.exp !== "number" || claims.exp <= now) return null;
  const audiences = Array.isArray(claims.aud) ? claims.aud : [claims.aud];
  if (!audiences.includes(env.ACCESS_AUD)) return null;
  if (claims.iss !== `https://${env.ACCESS_TEAM_DOMAIN}`) return null;
  if (claims.email !== OWNER_EMAIL) return null;
  return claims;
}

// ---------------------------------------------------------------------------
// Authorization endpoint (approval page + decision).

function authorizeErrorPage(status: number, message: string): Response {
  const html = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>mcpx — authorization</title></head>
<body style="font-family: system-ui, sans-serif; max-width: 34rem; margin: 3rem auto; padding: 0 1rem;">
<h1>mcpx</h1>
<p>${escapeHtml(message)}</p>
</body></html>`;
  return new Response(html, { status, headers: { "content-type": "text/html; charset=utf-8" } });
}

function redirectWithParams(redirectUri: string, params: Record<string, string>): Response {
  const url = new URL(redirectUri);
  for (const [key, value] of Object.entries(params)) url.searchParams.set(key, value);
  return new Response(null, { status: 302, headers: { location: url.toString() } });
}

export async function authorizeGet(request: Request, env: OAuthEnv): Promise<Response> {
  const claims = await verifyAccessAssertion(request, env);
  if (!claims) return authorizeErrorPage(403, "This page is only reachable through the owner's Cloudflare Access sign-in.");

  const url = new URL(request.url);
  const params = url.searchParams;
  const clientId = params.get("client_id");
  const client = await getClient(env, clientId);
  if (!client || !clientId) return authorizeErrorPage(400, "Unknown client_id.");

  const redirectUri = params.get("redirect_uri");
  if (!redirectUri || !client.redirect_uris.includes(redirectUri)) {
    return authorizeErrorPage(400, "redirect_uri does not match a registered URI for this client.");
  }
  if (params.get("response_type") !== "code") {
    return authorizeErrorPage(400, "Only response_type=code is supported.");
  }
  const codeChallenge = params.get("code_challenge");
  if (!codeChallenge || params.get("code_challenge_method") !== "S256") {
    return authorizeErrorPage(400, "A PKCE code_challenge with method S256 is required.");
  }
  const requestedScope = params.get("scope");
  if (requestedScope !== null && !requestedScope.split(" ").includes(SCOPE)) {
    return authorizeErrorPage(400, `Unsupported scope; this server issues only "${SCOPE}".`);
  }

  const nonce = randomToken(24);
  const pending: PendingRecord = {
    client_id: clientId,
    redirect_uri: redirectUri,
    state: params.get("state") ?? undefined,
    scope: SCOPE,
    code_challenge: codeChallenge,
  };
  await env.OAUTH_KV.put(`pending:${nonce}`, JSON.stringify(pending), { expirationTtl: PENDING_TTL_S });

  const clientLabel = client.client_name ? `${client.client_name} (${clientId})` : clientId;
  const redirectHost = new URL(redirectUri).host;
  const html = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>mcpx — authorize</title></head>
<body style="font-family: system-ui, sans-serif; max-width: 34rem; margin: 3rem auto; padding: 0 1rem;">
<h1>mcpx</h1>
<p><strong>${escapeHtml(clientLabel)}</strong> is asking to connect to the mcpx
on your laptop via <code>${escapeHtml(redirectHost)}</code>, with scope <code>${SCOPE}</code>.</p>
<p>Approving gives this client an access token (1 hour) and a refresh token (30 days)
that you can revoke by deleting them from the Worker's KV namespace.</p>
<form method="POST" action="/authorize">
<input type="hidden" name="nonce" value="${escapeHtml(nonce)}">
<button type="submit" name="decision" value="approve" style="padding: 0.5rem 1.25rem;">Approve</button>
<button type="submit" name="decision" value="deny" style="padding: 0.5rem 1.25rem;">Deny</button>
</form>
</body></html>`;
  return new Response(html, { status: 200, headers: { "content-type": "text/html; charset=utf-8" } });
}

export async function authorizePost(request: Request, env: OAuthEnv): Promise<Response> {
  const claims = await verifyAccessAssertion(request, env);
  if (!claims) return authorizeErrorPage(403, "This page is only reachable through the owner's Cloudflare Access sign-in.");

  const form = await request.formData();
  const nonce = form.get("nonce");
  const decision = form.get("decision");
  if (typeof nonce !== "string" || !nonce) return authorizeErrorPage(400, "Missing authorization request nonce.");

  const pending = await env.OAUTH_KV.get<PendingRecord>(`pending:${nonce}`, "json");
  // The pending request is single-use regardless of the outcome.
  await env.OAUTH_KV.delete(`pending:${nonce}`);
  if (!pending) return authorizeErrorPage(400, "This authorization request has expired or was already used.");

  const stateParam: Record<string, string> = pending.state ? { state: pending.state } : {};
  if (decision !== "approve") {
    return redirectWithParams(pending.redirect_uri, { error: "access_denied", ...stateParam });
  }

  const code = randomToken(32);
  const record: CodeRecord = {
    client_id: pending.client_id,
    redirect_uri: pending.redirect_uri,
    code_challenge: pending.code_challenge,
    scope: pending.scope,
    exp: Math.floor(Date.now() / 1000) + CODE_TTL_S,
  };
  await env.OAUTH_KV.put(`code:${await sha256Hex(code)}`, JSON.stringify(record), { expirationTtl: CODE_TTL_S });
  return redirectWithParams(pending.redirect_uri, { code, ...stateParam });
}

// ---------------------------------------------------------------------------
// Token endpoint.

async function issueTokens(env: OAuthEnv, clientId: string, scope: string): Promise<Response> {
  const now = Math.floor(Date.now() / 1000);
  const accessToken = randomToken(32);
  const refreshToken = randomToken(32);
  const accessRecord: TokenRecord = { client_id: clientId, scope, exp: now + ACCESS_TOKEN_TTL_S };
  const refreshRecord: TokenRecord = { client_id: clientId, scope, exp: now + REFRESH_TOKEN_TTL_S };
  await env.OAUTH_KV.put(`at:${await sha256Hex(accessToken)}`, JSON.stringify(accessRecord), {
    expirationTtl: ACCESS_TOKEN_TTL_S,
  });
  await env.OAUTH_KV.put(`rt:${await sha256Hex(refreshToken)}`, JSON.stringify(refreshRecord), {
    expirationTtl: REFRESH_TOKEN_TTL_S,
  });
  return json({
    access_token: accessToken,
    token_type: "Bearer",
    expires_in: ACCESS_TOKEN_TTL_S,
    refresh_token: refreshToken,
    scope,
  });
}

export async function tokenEndpoint(request: Request, env: OAuthEnv): Promise<Response> {
  let form: FormData;
  try {
    form = await request.formData();
  } catch {
    return oauthError("invalid_request", "request body must be application/x-www-form-urlencoded");
  }
  const grantType = form.get("grant_type");

  if (grantType === "authorization_code") {
    const code = form.get("code");
    const clientId = form.get("client_id");
    const redirectUri = form.get("redirect_uri");
    const verifier = form.get("code_verifier");
    if (typeof code !== "string" || typeof clientId !== "string" || typeof redirectUri !== "string" || typeof verifier !== "string") {
      return oauthError("invalid_request", "code, client_id, redirect_uri and code_verifier are required");
    }
    const codeKey = `code:${await sha256Hex(code)}`;
    const record = await env.OAUTH_KV.get<CodeRecord>(codeKey, "json");
    if (!record) return oauthError("invalid_grant", "unknown or expired authorization code");
    // Consume the code on any exchange attempt, before issuing, so a
    // failed PKCE or binding check cannot be retried against it.
    await env.OAUTH_KV.delete(codeKey);
    if (record.exp <= Math.floor(Date.now() / 1000)) return oauthError("invalid_grant", "authorization code expired");
    if (record.client_id !== clientId) return oauthError("invalid_grant", "client_id does not match the authorization request");
    if (record.redirect_uri !== redirectUri) return oauthError("invalid_grant", "redirect_uri does not match the authorization request");
    if ((await sha256Base64url(verifier)) !== record.code_challenge) {
      return oauthError("invalid_grant", "PKCE code_verifier does not match the challenge");
    }
    return issueTokens(env, clientId, record.scope);
  }

  if (grantType === "refresh_token") {
    const refreshToken = form.get("refresh_token");
    const clientId = form.get("client_id");
    if (typeof refreshToken !== "string" || typeof clientId !== "string") {
      return oauthError("invalid_request", "refresh_token and client_id are required");
    }
    const refreshKey = `rt:${await sha256Hex(refreshToken)}`;
    const record = await env.OAUTH_KV.get<TokenRecord>(refreshKey, "json");
    if (!record) return oauthError("invalid_grant", "unknown or expired refresh token");
    // Rotation: the presented refresh token dies whatever happens next.
    await env.OAUTH_KV.delete(refreshKey);
    if (record.exp <= Math.floor(Date.now() / 1000)) return oauthError("invalid_grant", "refresh token expired");
    if (record.client_id !== clientId) return oauthError("invalid_grant", "client_id does not match the refresh token");
    return issueTokens(env, clientId, record.scope);
  }

  return oauthError("unsupported_grant_type", "grant_type must be authorization_code or refresh_token");
}

// ---------------------------------------------------------------------------
// /mcp bearer validation.

export async function isValidAccessToken(env: OAuthEnv, token: string): Promise<boolean> {
  const record = await env.OAUTH_KV.get<TokenRecord>(`at:${await sha256Hex(token)}`, "json");
  return record !== null && record.exp > Math.floor(Date.now() / 1000);
}
