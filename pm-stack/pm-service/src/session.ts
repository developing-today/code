/**
 * Browser session primitive for SSO relying-party slice H3 (#14261).
 *
 * The design (§4.2) terminates surface A's browser login in a front-door
 * session cookie: httpOnly, Secure, SameSite=Lax, 12h idle expiry, bound to
 * `binding_id` plus a role-claim snapshot. The cookie is a pointer, not a
 * capability — every request re-resolves live `role_grant` rows through
 * `resolveLivePrincipal`, so revocation lands within one request instead of
 * waiting for cookie expiry. Provider bearer values are never placed in the
 * cookie and never trusted from it.
 */
import type { ResolvedPrincipal } from "./front-door.ts";

export const SESSION_COOKIE = "pm_session";

/** Twelve hours in milliseconds: the idle-expiry budget from §4.2. */
export const IDLE_TIMEOUT_MS = 12 * 60 * 60 * 1000;

export interface BrowserSession {
  id: string;
  principal: ResolvedPrincipal;
  createdAt: number;
  lastSeenAt: number;
}

export interface BrowserSessionStore {
  get(id: string): Promise<BrowserSession | null>;
  put(session: BrowserSession): Promise<void>;
  touch(id: string, lastSeenAt: number): Promise<void>;
  delete(id: string): Promise<void>;
}

export type ResolveLivePrincipal = (
  session: Readonly<BrowserSession>,
) => Promise<ResolvedPrincipal | null>;

export type SessionDenyReason = "missing_session" | "expired_session" | "revoked_session";

export type AuthenticateResult =
  | { ok: true; principal: ResolvedPrincipal }
  | { ok: false; reason: SessionDenyReason };

function newSessionId(): string {
  const random = crypto.randomUUID?.();
  if (typeof random === "string" && random.length > 0) return random;
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function setCookieHeader(sessionId: string, maxAgeSeconds: number): string {
  const parts = [
    `${SESSION_COOKIE}=${sessionId}`,
    "Path=/",
    `Max-Age=${maxAgeSeconds}`,
    "HttpOnly",
    "Secure",
    "SameSite=Lax",
  ];
  return parts.join("; ");
}

function readSessionId(req: Request): string | null {
  const header = req.headers.get("cookie");
  if (header === null || header.trim() === "") return null;
  for (const part of header.split(";")) {
    const index = part.indexOf("=");
    if (index < 0) continue;
    if (part.slice(0, index).trim() !== SESSION_COOKIE) continue;
    const value = part.slice(index + 1).trim();
    if (value === "") return null;
    return value;
  }
  return null;
}

/**
 * Issue an opaque session cookie for an already-resolved principal.
 * Returns the `Set-Cookie` header value; the provider token's `jti` and
 * subject stay server-side and are never encoded in the cookie.
 */
export async function createBrowserSession(
  principal: ResolvedPrincipal,
  opts: { store: BrowserSessionStore; now?: () => number },
): Promise<string> {
  const nowMs = (opts.now ?? Date.now)();
  const session: BrowserSession = {
    id: newSessionId(),
    principal,
    createdAt: nowMs,
    lastSeenAt: nowMs,
  };
  await opts.store.put(session);
  return setCookieHeader(session.id, IDLE_TIMEOUT_MS / 1000);
}

export interface AuthenticateOptions {
  store: BrowserSessionStore;
  now?: () => number;
  idleTimeoutMs?: number;
  resolveLivePrincipal: ResolveLivePrincipal;
}

/**
 * Authenticate one request: validate the opaque cookie, enforce idle expiry,
 * refresh activity, then re-resolve live roles. Denials are returned, never
 * thrown; an unknown server-side session means the session was logged out or
 * revoked upstream.
 */
export async function authenticateBrowserSession(
  req: Request,
  opts: AuthenticateOptions,
): Promise<AuthenticateResult> {
  const sessionId = readSessionId(req);
  if (sessionId === null) return { ok: false, reason: "missing_session" };
  const nowMs = (opts.now ?? Date.now)();
  const idleTimeoutMs = opts.idleTimeoutMs ?? IDLE_TIMEOUT_MS;
  const session = await opts.store.get(sessionId);
  if (session === null) return { ok: false, reason: "revoked_session" };
  if (nowMs - session.lastSeenAt >= idleTimeoutMs) {
    await opts.store.delete(session.id);
    return { ok: false, reason: "expired_session" };
  }
  await opts.store.touch(session.id, nowMs);
  const live = await opts.resolveLivePrincipal({ ...session });
  if (live === null) return { ok: false, reason: "revoked_session" };
  return { ok: true, principal: live };
}

/**
 * Log out: delete the server-side session when present and always return a
 * cookie-clearing `Set-Cookie` header so the browser drops the pointer.
 */
export async function logoutBrowserSession(
  req: Request,
  opts: { store: BrowserSessionStore; now?: () => number },
): Promise<string> {
  const sessionId = readSessionId(req);
  if (sessionId !== null) await opts.store.delete(sessionId);
  return setCookieHeader("", 0);
}
