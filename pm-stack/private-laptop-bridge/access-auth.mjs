// Verify Cloudflare Access service-token JWTs at the Worker origin.
// The caller still needs its independent Ed25519 bridge signature.
const MAX_JWT_LENGTH = 16_384;
const MAX_KEYS_BYTES = 32_768;
const SKEW_SECONDS = 30;
const MAX_TOKEN_AGE_SECONDS = 15 * 60;
const KEY_CACHE_MS = 5 * 60_000;
const keyCache = new Map();
export const BRIDGE_HOST = 'laptop-bridge-staging.security.cab';
export const BRIDGE_ORIGIN = `https://${BRIDGE_HOST}`;

const fail = () => { throw new Error('access-denied'); };
const decodeBytes = value => {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_-]+$/.test(value) || value.length % 4 === 1) fail();
  const bytes = Uint8Array.from(atob(value.replaceAll('-', '+').replaceAll('_', '/') + '='.repeat((4 - value.length % 4) % 4)), c => c.charCodeAt(0));
  const canonical = btoa(String.fromCharCode(...bytes)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
  if (canonical !== value) fail();
  return bytes;
};
const decodePart = value => {
  return JSON.parse(new TextDecoder('utf-8', {fatal: true}).decode(decodeBytes(value)));
};

function accessConfig(env) {
  const {ACCESS_ISSUER: issuer, ACCESS_AUDIENCE: audience, BRIDGE_HOST: host} = env;
  let parsed;
  try { parsed = new URL(issuer); } catch { fail(); }
  if (parsed.protocol !== 'https:' || parsed.origin !== issuer || parsed.pathname !== '/' || parsed.search || parsed.hash ||
      !/^[a-z0-9-]+\.cloudflareaccess\.com$/.test(parsed.hostname) ||
      typeof audience !== 'string' || !/^[A-Za-z0-9_-]{20,128}$/.test(audience) ||
      host !== BRIDGE_HOST) fail();
  return {issuer, audience, host, certsUrl: `${issuer}/cdn-cgi/access/certs`};
}

async function accessKeys({issuer, certsUrl}, fetchImpl, now) {
  const cached = keyCache.get(issuer);
  if (cached && cached.expiresAt > now) return cached.keys;
  let response, text, body;
  try {
    response = await fetchImpl(certsUrl, {method: 'GET', redirect: 'error', signal: AbortSignal.timeout(3000)});
    if (!response.ok || response.redirected) fail();
    text = await response.text();
    if (text.length > MAX_KEYS_BYTES) fail();
    body = JSON.parse(text);
  } catch { fail(); }
  if (!body || !Array.isArray(body.keys) || body.keys.length < 1 || body.keys.length > 4) fail();
  const keys = new Map();
  for (const jwk of body.keys) {
    if (!jwk || jwk.kty !== 'RSA' || jwk.alg !== 'RS256' || jwk.use !== 'sig' ||
        typeof jwk.kid !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/.test(jwk.kid) || keys.has(jwk.kid) ||
        typeof jwk.n !== 'string' || typeof jwk.e !== 'string') fail();
    try { keys.set(jwk.kid, await crypto.subtle.importKey('jwk', jwk, {name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256'}, false, ['verify'])); }
    catch { fail(); }
  }
  keyCache.set(issuer, {keys, expiresAt: now + KEY_CACHE_MS});
  return keys;
}

/** Return the verified service-token Client ID. Never trusts caller-supplied
 * identity fields or Access headers without the signed JWT and exact config. */
export async function verifyAccessServiceToken(request, env, {fetchImpl = fetch, nowMs = Date.now()} = {}) {
  const cfg = accessConfig(env);
  const url = new URL(request.url);
  if (request.method !== 'POST' || url.protocol !== 'https:' || url.hostname !== cfg.host || url.port || url.username || url.password || url.search || url.hash) fail();
  const token = request.headers.get('cf-access-jwt-assertion');
  const clientId = request.headers.get('cf-access-client-id');
  if (!token || token.length > MAX_JWT_LENGTH || !clientId || !/^[A-Za-z0-9._-]{8,128}$/.test(clientId)) fail();
  const parts = token.split('.');
  if (parts.length !== 3) fail();
  const [header, claims] = [decodePart(parts[0]), decodePart(parts[1])];
  if (!header || header.alg !== 'RS256' || header.typ !== 'JWT' || typeof header.kid !== 'string' ||
      !claims || claims.type !== 'app' || claims.iss !== cfg.issuer || claims.sub !== '' ||
      !Array.isArray(claims.aud) || claims.aud.length < 1 || !claims.aud.includes(cfg.audience) ||
      claims.common_name !== clientId || !Number.isSafeInteger(claims.exp) || !Number.isSafeInteger(claims.iat) ||
      claims.exp <= Math.floor(nowMs / 1000) - SKEW_SECONDS || claims.iat > Math.floor(nowMs / 1000) + SKEW_SECONDS ||
      claims.exp <= claims.iat || claims.exp - claims.iat > MAX_TOKEN_AGE_SECONDS ||
      (claims.nbf !== undefined && (!Number.isSafeInteger(claims.nbf) || claims.nbf > Math.floor(nowMs / 1000) + SKEW_SECONDS))) fail();
  const keys = await accessKeys(cfg, fetchImpl, nowMs);
  const key = keys.get(header.kid);
  if (!key) fail();
  let valid = false;
  try {
    valid = await crypto.subtle.verify('RSASSA-PKCS1-v1_5', key,
      decodeBytes(parts[2]),
      new TextEncoder().encode(`${parts[0]}.${parts[1]}`));
  } catch { fail(); }
  if (!valid) fail();
  return clientId;
}

export function clearAccessKeyCacheForTests() { keyCache.clear(); }
