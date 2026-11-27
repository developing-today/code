import { resolveFrontDoor, type FrontDoorDeps } from "./front-door.ts";
import {
  createBrowserSession,
  type BrowserSessionStore,
  type ResolveLivePrincipal,
  type SessionAudit,
} from "./session.ts";

const STATE_COOKIE = "__Host-pm_oauth_state";
const MAX_PENDING_MS = 10 * 60 * 1000;

export interface PendingLogin {
  state: string;
  nonce: string;
  verifier: string;
  createdAt: number;
}

export interface PendingLoginStore {
  put(login: PendingLogin): Promise<void>;
  get(state: string): Promise<PendingLogin | null>;
  delete(state: string): Promise<void>;
}

export interface OAuthProvider {
  authorizationEndpoint: string;
  clientId: string;
  redirectUri: string;
  /** Exchange must validate the provider's code response and return its ID token. */
  exchangeCode(input: { code: string; verifier: string; redirectUri: string }): Promise<{ idToken: string }>;
}

export interface BrowserOAuthDeps {
  provider: OAuthProvider;
  pending: PendingLoginStore;
  sessions: BrowserSessionStore;
  resolveLivePrincipal: ResolveLivePrincipal;
  frontDoor: Omit<FrontDoorDeps, "now">;
  now?: () => number;
  audit?: SessionAudit;
  randomBytes?: (size: number) => Uint8Array;
}

function random(size: number, source?: (size: number) => Uint8Array): Uint8Array {
  if (source) return source(size);
  return crypto.getRandomValues(new Uint8Array(size));
}

function base64url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

function randomToken(size: number, source?: (size: number) => Uint8Array): string {
  return base64url(random(size, source));
}

function stateCookie(state: string, maxAge: number): string {
  return `${STATE_COOKIE}=${state}; Path=/; Max-Age=${maxAge}; HttpOnly; Secure; SameSite=Lax`;
}

function cookieValue(req: Request, name: string): string | null {
  for (const part of (req.headers.get("cookie") ?? "").split(";")) {
    const split = part.indexOf("=");
    if (split >= 0 && part.slice(0, split).trim() === name) return part.slice(split + 1).trim() || null;
  }
  return null;
}

function redirect(location: string, status = 302, cookie?: string): Response {
  const headers = new Headers({ location, "cache-control": "no-store", pragma: "no-cache" });
  if (cookie) headers.append("set-cookie", cookie);
  return new Response(null, { status, headers });
}

export async function startBrowserLogin(deps: BrowserOAuthDeps): Promise<Response> {
  const now = (deps.now ?? Date.now)();
  const state = randomToken(32, deps.randomBytes);
  const nonce = randomToken(32, deps.randomBytes);
  const verifier = randomToken(32, deps.randomBytes);
  const challenge = base64url(
    new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))),
  );
  await deps.pending.put({ state, nonce, verifier, createdAt: now });

  const target = new URL(deps.provider.authorizationEndpoint);
  if (target.protocol !== "https:") throw new Error("OAuth authorization endpoint must use HTTPS");
  target.searchParams.set("response_type", "code");
  target.searchParams.set("client_id", deps.provider.clientId);
  target.searchParams.set("redirect_uri", deps.provider.redirectUri);
  target.searchParams.set("scope", "openid profile");
  target.searchParams.set("state", state);
  target.searchParams.set("nonce", nonce);
  target.searchParams.set("code_challenge", challenge);
  target.searchParams.set("code_challenge_method", "S256");
  return redirect(target.toString(), 302, stateCookie(state, MAX_PENDING_MS / 1000));
}

export async function completeBrowserLogin(req: Request, deps: BrowserOAuthDeps): Promise<Response> {
  const url = new URL(req.url);
  const clearState = stateCookie("", 0);
  const state = url.searchParams.get("state");
  const code = url.searchParams.get("code");
  const cookieState = cookieValue(req, STATE_COOKIE);
  if (!state || !code || !cookieState || state !== cookieState)
    return redirect("/login?error=invalid_state", 303, clearState);

  const pending = await deps.pending.get(state);
  await deps.pending.delete(state);
  const now = (deps.now ?? Date.now)();
  if (!pending || pending.state !== state || now - pending.createdAt >= MAX_PENDING_MS || now < pending.createdAt)
    return redirect("/login?error=expired_state", 303, clearState);

  try {
    const { idToken } = await deps.provider.exchangeCode({
      code,
      verifier: pending.verifier,
      redirectUri: deps.provider.redirectUri,
    });
    const result = await resolveFrontDoor(idToken, { ...deps.frontDoor, now: () => now / 1000 });
    if (!result.ok) return redirect("/login?error=authentication_failed", 303, clearState);
    const claims = readJwtClaims(idToken);
    if (!claims || claims.nonce !== pending.nonce) return redirect("/login?error=invalid_nonce", 303, clearState);
    const sessionCookie = await createBrowserSession(result.principal, {
      store: deps.sessions,
      now: () => now,
      audit: deps.audit,
    });
    const response = redirect("/", 303, sessionCookie);
    response.headers.append("set-cookie", clearState);
    return response;
  } catch {
    return redirect("/login?error=authentication_failed", 303, clearState);
  }
}

function readJwtClaims(token: string): Record<string, unknown> | null {
  const parts = token.split(".");
  if (parts.length !== 3 || !/^[A-Za-z0-9_-]+$/.test(parts[1] ?? "")) return null;
  try {
    const encoded = (parts[1] as string).replaceAll("-", "+").replaceAll("_", "/");
    const bytes = Uint8Array.from(atob(encoded.padEnd(Math.ceil(encoded.length / 4) * 4, "=")), (char) =>
      char.charCodeAt(0),
    );
    const value: unknown = JSON.parse(new TextDecoder().decode(bytes));
    return typeof value === "object" && value !== null && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}
