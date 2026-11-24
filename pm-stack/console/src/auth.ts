// Who is calling.
//
// The console never runs a sign-in itself. A caller arrives with a
// signed token from an issuer the deployment trusts, and the console
// checks the signature, the issuer, the audience and the times, then
// asks one question: is this identity the owner?
//
// Today the issuer is Cloudflare Access at the public edge, which runs
// the interactive sign-in (Google, and any OIDC provider or client
// certificate authority added to it later). Any other OIDC issuer's
// token is accepted the same way once it is listed in the issuers file,
// so the central provider needs configuration, not code.
//
// A request that did not come through the edge at all (loopback host,
// no edge headers) is a process on this machine and is the owner: such
// a process already has the owner's shell.
import type { Config, Issuer } from "./config.ts";
import { timed, unread } from "./timed.ts";

export interface Identity {
  /** Issuer name, or "loopback". */
  via: string;
  email?: string;
  certName?: string;
  sub?: string;
}

export class AuthError extends Error {
  constructor(
    public status: 401 | 403,
    message: string,
  ) {
    super(message);
  }
}

type Jwk = JsonWebKey & { kid?: string };
type FetchLike = (
  url: string,
  init?: { signal?: AbortSignal },
) => Promise<{ ok: boolean; status: number; json(): Promise<unknown> }>;

const JWKS_TTL_MS = 10 * 60_000;
const JWKS_REFRESH_COOLDOWN_MS = 30_000;
const JWKS_FETCH_TIMEOUT_MS = 5_000;
const SKEW_MS = 60_000;
/** The issuers' key sets. Module memory only, by decision: each object
 * starting asks its issuer again (one public request, cached ten
 * minutes while the object lasts), and sharing keys across objects
 * would need storage reads on every request to save one cacheable
 * fetch per object lifetime. */
const jwksCache = new Map<string, { at: number; keys: Jwk[] }>();
const jwksInFlight = new Map<string, Promise<Jwk[]>>();

export function clearJwksCache(): void {
  jwksCache.clear();
  jwksInFlight.clear();
}

/**
 * The issuer's keys. `force` asks for a fresh copy because a token named
 * a key the cache does not hold. Anyone can send such a token, so a
 * forced refresh happens at most once per cooldown, concurrent callers
 * share one request, and the request has a deadline.
 */
async function jwks(uri: string, fetchImpl: FetchLike, now: number, force = false): Promise<Jwk[]> {
  const hit = jwksCache.get(uri);
  if (hit && now - hit.at < (force ? JWKS_REFRESH_COOLDOWN_MS : JWKS_TTL_MS)) return hit.keys;
  const pending = jwksInFlight.get(uri);
  if (pending) return pending;
  const request = timed(JWKS_FETCH_TIMEOUT_MS, async (signal) => {
    const res = await fetchImpl(uri, { signal });
    if (!res.ok) {
      unread(res);
      throw new Error(`key set fetch failed: http ${res.status}`);
    }
    const body = (await res.json()) as { keys?: unknown };
    if (!Array.isArray(body.keys)) throw new Error("key set has no keys");
    const keys = body.keys as Jwk[];
    jwksCache.set(uri, { at: now, keys });
    return keys;
  });
  jwksInFlight.set(uri, request);
  try {
    return await request;
  } finally {
    jwksInFlight.delete(uri);
  }
}

function b64url(text: string): Uint8Array<ArrayBuffer> {
  if (!/^[A-Za-z0-9_-]*$/.test(text)) throw new Error("not base64url");
  const pad = text.length % 4 === 0 ? "" : "=".repeat(4 - (text.length % 4));
  const bin = atob(text.replace(/-/g, "+").replace(/_/g, "/") + pad);
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
}

const ALGS: Record<
  string,
  { kty: string; importAlg: RsaHashedImportParams | EcKeyImportParams; verifyAlg: AlgorithmIdentifier | EcdsaParams }
> = {
  RS256: { kty: "RSA", importAlg: { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" }, verifyAlg: "RSASSA-PKCS1-v1_5" },
  ES256: {
    kty: "EC",
    importAlg: { name: "ECDSA", namedCurve: "P-256" },
    verifyAlg: { name: "ECDSA", hash: "SHA-256" },
  },
};

/** Verify a compact JWT against one issuer. Returns its claims. */
export async function verifyJwt(
  token: string,
  issuer: Issuer,
  fetchImpl: FetchLike = fetch as unknown as FetchLike,
  now: number = Date.now(),
): Promise<Record<string, unknown>> {
  const parts = token.split(".");
  if (parts.length !== 3) throw new Error("malformed token");
  let header: Record<string, unknown>;
  let claims: Record<string, unknown>;
  try {
    header = JSON.parse(new TextDecoder().decode(b64url(parts[0])));
    claims = JSON.parse(new TextDecoder().decode(b64url(parts[1])));
  } catch {
    throw new Error("malformed token");
  }
  if (header === null || typeof header !== "object" || claims === null || typeof claims !== "object") {
    throw new Error("malformed token");
  }
  // The algorithm comes from a fixed list, never from the key or from
  // whatever the token asks for ("none", HS256 with a public key, ...).
  const alg = typeof header.alg === "string" ? ALGS[header.alg] : undefined;
  if (!alg) throw new Error("unsupported token algorithm");
  if (typeof header.kid !== "string" || header.kid === "") throw new Error("token names no key");
  const match = (keys: Jwk[]) => keys.find((k) => k.kid === header.kid && k.kty === alg.kty);
  let jwk = match(await jwks(issuer.jwksUri, fetchImpl, now));
  // An issuer may rotate keys inside the cache window: one forced refresh.
  if (!jwk) jwk = match(await jwks(issuer.jwksUri, fetchImpl, now, true));
  if (!jwk) throw new Error("no key matches the token");
  const { kty, n, e, crv, x, y } = jwk;
  const key = await crypto.subtle.importKey("jwk", { kty, n, e, crv, x, y }, alg.importAlg, false, ["verify"]);
  const ok = await crypto.subtle.verify(
    alg.verifyAlg,
    key,
    b64url(parts[2]),
    new TextEncoder().encode(`${parts[0]}.${parts[1]}`),
  );
  if (!ok) throw new Error("token signature invalid");

  if (claims.iss !== issuer.issuer) throw new Error("token issuer mismatch");
  const aud = claims.aud;
  const audOk = typeof aud === "string" ? aud === issuer.audience : Array.isArray(aud) && aud.includes(issuer.audience);
  if (!audOk) throw new Error("token audience mismatch");
  if (typeof claims.exp !== "number" || claims.exp * 1000 < now - SKEW_MS) throw new Error("token expired");
  if (claims.nbf !== undefined && (typeof claims.nbf !== "number" || claims.nbf * 1000 > now + SKEW_MS)) {
    throw new Error("token not yet valid");
  }
  if (claims.iat !== undefined && (typeof claims.iat !== "number" || claims.iat * 1000 > now + SKEW_MS)) {
    throw new Error("token issued in the future");
  }
  return claims;
}

const EDGE_HEADERS = [
  "cf-ray",
  "cf-connecting-ip",
  "cdn-loop",
  "cf-visitor",
  "cf-access-jwt-assertion",
  "x-forwarded-for",
];

export function isLoopbackHost(host: string | null): boolean {
  if (!host) return false;
  const name = host.toLowerCase().replace(/:\d+$/, "");
  return name === "127.0.0.1" || name === "localhost" || name === "[::1]";
}

/** True when the request shows any sign of having crossed a proxy. */
export function cameThroughEdge(req: Request): boolean {
  return !isLoopbackHost(req.headers.get("host")) || EDGE_HEADERS.some((h) => req.headers.has(h));
}

function cookie(req: Request, name: string): string | null {
  for (const part of (req.headers.get("cookie") ?? "").split(";")) {
    const i = part.indexOf("=");
    if (i > 0 && part.slice(0, i).trim() === name) return part.slice(i + 1).trim();
  }
  return null;
}

function presented(req: Request, source: Issuer["source"]): string | null {
  if (source === "access") return req.headers.get("cf-access-jwt-assertion") ?? cookie(req, "CF_Authorization");
  const m = /^Bearer\s+(\S+)$/i.exec(req.headers.get("authorization") ?? "");
  return m ? m[1] : null;
}

/** Map verified claims to the owner, or refuse. */
export function authorize(
  claims: Record<string, unknown>,
  via: string,
  cfg: Pick<Config, "owners" | "certNames">,
): Identity {
  const sub = typeof claims.sub === "string" ? claims.sub : undefined;
  const email = typeof claims.email === "string" ? claims.email.toLowerCase() : "";
  // An issuer that says the address is unverified has not identified anyone.
  if (email !== "" && claims.email_verified !== false && cfg.owners.includes(email)) return { via, email, sub };
  const certName = typeof claims.common_name === "string" ? claims.common_name : "";
  if (certName !== "" && cfg.certNames.includes(certName)) return { via, certName, sub };
  throw new AuthError(403, "this identity is not the owner");
}

/** True for an address on this machine's loopback interface. */
export function isLoopbackAddress(address: string | null | undefined): boolean {
  if (!address) return false;
  const a = address.toLowerCase().replace(/^::ffff:/, "");
  return a === "::1" || /^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(a);
}

export interface IdentifyOptions {
  /** The connection's remote address, from the server, never from a header. */
  peer?: string | null;
  fetchImpl?: FetchLike;
  now?: number;
}

/** Identify the caller of a request, or throw AuthError. */
export async function identify(
  req: Request,
  cfg: Pick<Config, "owners" | "certNames" | "issuers">,
  opts: IdentifyOptions = {},
): Promise<Identity> {
  const fetchImpl = opts.fetchImpl ?? (fetch as unknown as FetchLike);
  const now = opts.now ?? Date.now();
  let sawToken = false;
  for (const issuer of cfg.issuers) {
    const token = presented(req, issuer.source);
    if (!token) continue;
    sawToken = true;
    let claims: Record<string, unknown>;
    try {
      claims = await verifyJwt(token, issuer, fetchImpl, now);
    } catch {
      continue; // another issuer may own this token
    }
    return authorize(claims, issuer.name, cfg);
  }
  if (sawToken) throw new AuthError(401, "the presented token was not accepted");
  // Local means the connection itself came from this machine. The Host
  // and forwarding headers are the caller's to write, so they can only
  // take the local identity away, never grant it.
  if (!isLoopbackAddress(opts.peer) || cameThroughEdge(req)) throw new AuthError(401, "sign-in required");
  return { via: "loopback", email: "local" };
}
