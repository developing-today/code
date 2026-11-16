/**
 * OpenDesign session renewal — Drew's provider browser-auth
 * auto-renewal rule (2026-11-05): no provider integration may depend
 * on a browser cookie/session that silently expires. Every
 * browser-dependent auth carries an auto-renewing long-lived token,
 * or built-in cookie auto-renewal with an alert BEFORE hard expiry
 * when renewal fails.
 *
 * Why OpenDesign needs this: its quota provider reads plan windows
 * from the AMR billing API, which accepts ONLY a Better Auth session
 * (cookie + workspace id, held in OpenChamber's managed quota
 * credential store). The AMR inference API key is refused there
 * (verified 403, 2026-11-03; the server-side gap is code#5060). The
 * session was imported once by hand on 2026-10-29, its import-time
 * expiry (2026-11-06) was treated as a hard death date, and the only
 * plan was a one-shot re-import cron.
 *
 * What actually keeps it alive, verified live 2026-11-05: the Better
 * Auth session SLIDES on use — once its update interval (observed
 * ~24h) has elapsed, an authenticated get-session read re-extends
 * expiresAt to read-time + 7 days (stored session created
 * 2026-10-30T01:23Z; a read at 2026-11-05T01:59Z moved expiresAt to
 * 2026-11-12T01:59Z, and a read 5h later left it untouched). Against
 * a 7-day expiry, the gatherer's 15-minute tick was already renewing
 * it by accident, with days of slack. This module makes that
 * deliberate and monitored, and adds a browser re-capture for when
 * use is not enough:
 *
 *   1. Every tick: read the stored session (get-session). The read
 *      is itself the renewal-by-use; expiresAt + horizon land in the
 *      state file, so the horizon is a watched number, not a
 *      surprise.
 *   2. Stored session failed auth, or horizon below
 *      LOW_HORIZON_HOURS: re-capture from the standing laptop
 *      Helium over CDP — read the live better-auth session cookie
 *      from the browser profile, validate the candidate against the
 *      usage API BEFORE storing it, and only then replace the
 *      stored credential. A candidate that fails validation never
 *      overwrites a working credential.
 *   3. Renewal cannot complete (the browser holds no Open Design
 *      session — sign-in is an agent/Drew step — or CDP is down):
 *      the state file carries an alert with the remaining horizon.
 *      The warning is raised while days remain, never as a silent
 *      lapse at zero.
 *
 * The state file is rewritten every run (see gather-quota.ts for
 * the default path, beside quota-current.json). No credential value
 * is logged or written anywhere in this module; capture moves the
 * cookie from the browser straight into the managed store.
 */

import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";

export const OPENDESIGN_API_BASE = "https://amr-api.open-design.ai/api";
export const OPENDESIGN_APP_URL = "https://open-design.ai";
export const LOW_HORIZON_HOURS = 72;

const SESSION_COOKIE_NAMES = ["__Secure-better-auth.session_token", "better-auth.session_token"];
// The session header's name, kept as a constant: fleet log tooling
// pattern-matches inline cookie assignments and mangles them.
const SESSION_HEADER = "Cookie";

export interface OpenDesignCredentialLike {
  cookie: string;
  workspaceId: string;
}

export interface BrowserCookie {
  name: string;
  value: string;
  domain: string;
}

export type CaptureResult = "stored" | "no-browser-session" | "cdp-unreachable" | "validation-failed" | "not-attempted";

export interface OpenDesignRenewalState {
  provider: "opendesign";
  checkedAt: string;
  status: "not-configured" | "valid" | "renewed" | "attention";
  sessionExpiresAt: string | null;
  horizonHours: number | null;
  capture: { attempted: boolean; result: CaptureResult | string | null };
  alert: string | null;
  alertSince: string | null;
}

export interface OpenDesignRenewalDeps {
  readCredential: () => OpenDesignCredentialLike | null;
  writeCredential: (value: OpenDesignCredentialLike) => void;
  /** Throws when the credential cannot read real quota windows. */
  validateCredential: (credential: OpenDesignCredentialLike) => Promise<void>;
  statePath: string;
  cdpUrl?: string;
  now?: () => Date;
  forceCapture?: boolean;
  lowHorizonHours?: number;
  /** Injectable for tests; default is the real get-session read. */
  fetchSession?: (credential: OpenDesignCredentialLike) => Promise<{ expiresAt: string | null } | null>;
  /** Injectable for tests; defaults are the real CDP operations. */
  getBrowserCookies?: (cdpUrl: string) => Promise<BrowserCookie[]>;
  refreshBrowserTab?: (cdpUrl: string) => Promise<void>;
  log?: (line: string) => void;
}

/** Pick the Open Design Better Auth session cookie out of a browser cookie jar. */
export function pickOpenDesignSessionCookie(cookies: BrowserCookie[]): BrowserCookie | null {
  const matches = cookies.filter(
    (c) => SESSION_COOKIE_NAMES.includes(c.name) && c.domain.endsWith("open-design.ai") && c.value !== "",
  );
  // The __Secure- prefixed name is the one the app sets today; prefer it.
  return matches.find((c) => c.name.startsWith("__Secure-")) ?? matches[0] ?? null;
}

export function horizonHours(expiresAt: string | null, now: Date): number | null {
  if (!expiresAt) return null;
  const ms = Date.parse(expiresAt) - now.getTime();
  return Number.isFinite(ms) ? Math.round((ms / 3_600_000) * 10) / 10 : null;
}

const defaultFetchSession = async (
  credential: OpenDesignCredentialLike,
): Promise<{ expiresAt: string | null } | null> => {
  const headers: Record<string, string> = {
    Accept: "application/json",
    "x-vela-workspace-id": credential.workspaceId,
    "User-Agent": "OpenChamber quota gatherer (opendesign renewal)",
  };
  headers[SESSION_HEADER] = credential.cookie;
  try {
    const response = await fetch(`${OPENDESIGN_API_BASE}/auth/get-session`, {
      method: "GET",
      headers,
      signal: AbortSignal.timeout(15_000),
    });
    if (!response.ok) return null;
    const payload = (await response.json().catch(() => null)) as { session?: { expiresAt?: unknown } | null } | null;
    // Better Auth answers 200 with a null/empty body when the cookie
    // names no live session — that is an auth failure, not a session
    // with an unknown expiry.
    const session = payload?.session;
    if (!session || typeof session !== "object") return null;
    return { expiresAt: typeof session.expiresAt === "string" ? session.expiresAt : null };
  } catch {
    return null;
  }
};

interface CdpSocket {
  addEventListener: (type: string, listener: (event: { data?: unknown }) => void) => void;
  send: (data: string) => void;
  close: () => void;
}

const withTimeout = async <T>(promise: Promise<T>, ms: number, label: string): Promise<T> => {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out after ${ms}ms`)), ms);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
};

const cdpSession = async (
  cdpUrl: string,
): Promise<{
  send: (method: string, params?: Record<string, unknown>) => Promise<Record<string, unknown>>;
  close: () => void;
}> => {
  const version = (await withTimeout(
    fetch(`${cdpUrl}/json/version`).then((r) => r.json() as Promise<{ webSocketDebuggerUrl?: string }>),
    10_000,
    "CDP version read",
  )) as { webSocketDebuggerUrl?: string };
  if (!version.webSocketDebuggerUrl) throw new Error("CDP version response carried no webSocketDebuggerUrl");
  const SocketCtor = (globalThis as { WebSocket?: new (url: string) => CdpSocket }).WebSocket;
  if (!SocketCtor) throw new Error("no WebSocket implementation available for CDP");
  const socket = new SocketCtor(version.webSocketDebuggerUrl);
  await withTimeout(
    new Promise<void>((resolve, reject) => {
      socket.addEventListener("open", () => resolve());
      socket.addEventListener("error", () => reject(new Error("CDP socket error")));
    }),
    10_000,
    "CDP socket open",
  );
  let nextId = 0;
  const pending = new Map<number, (message: Record<string, unknown>) => void>();
  socket.addEventListener("message", (event) => {
    try {
      const message = JSON.parse(String(event.data)) as { id?: number };
      if (typeof message.id === "number" && pending.has(message.id)) {
        pending.get(message.id)?.(message as Record<string, unknown>);
        pending.delete(message.id);
      }
    } catch {
      // Non-JSON frames are not answers; ignore them.
    }
  });
  const send = (method: string, params: Record<string, unknown> = {}): Promise<Record<string, unknown>> =>
    withTimeout(
      new Promise<Record<string, unknown>>((resolve) => {
        const id = ++nextId;
        pending.set(id, resolve);
        socket.send(JSON.stringify({ id, method, params }));
      }),
      15_000,
      `CDP ${method}`,
    );
  return { send, close: () => socket.close() };
};

const defaultGetBrowserCookies = async (cdpUrl: string): Promise<BrowserCookie[]> => {
  const { send, close } = await cdpSession(cdpUrl);
  try {
    const answer = await send("Storage.getCookies", {});
    const result = (answer as { result?: { cookies?: BrowserCookie[] } }).result;
    return Array.isArray(result?.cookies) ? result.cookies : [];
  } finally {
    close();
  }
};

const defaultRefreshBrowserTab = async (cdpUrl: string): Promise<void> => {
  const { send, close } = await cdpSession(cdpUrl);
  try {
    const created = await send("Target.createTarget", { url: OPENDESIGN_APP_URL });
    const targetId = (created as { result?: { targetId?: string } }).result?.targetId;
    // Give the app a moment to load and slide/set its session cookie.
    await new Promise((resolve) => setTimeout(resolve, 4_000));
    if (targetId) await send("Target.closeTarget", { targetId }).catch(() => undefined);
  } finally {
    close();
  }
};

const readPreviousState = async (statePath: string): Promise<OpenDesignRenewalState | null> => {
  try {
    return JSON.parse(await readFile(statePath, "utf8")) as OpenDesignRenewalState;
  } catch {
    return null;
  }
};

const writeState = async (statePath: string, state: OpenDesignRenewalState): Promise<void> => {
  await mkdir(dirname(statePath), { recursive: true });
  const tmp = `${statePath}.tmp-${process.pid}`;
  await writeFile(tmp, JSON.stringify(state, null, 2) + "\n", "utf8");
  await rename(tmp, statePath);
};

/**
 * One renewal cycle. Fail-soft by contract: it always resolves with
 * a state object (which it also persists); it never throws, so the
 * gatherer's tick can call it unconditionally.
 */
export async function renewOpenDesignSession(deps: OpenDesignRenewalDeps): Promise<OpenDesignRenewalState> {
  const now = (deps.now ?? (() => new Date()))();
  const lowHorizon = deps.lowHorizonHours ?? LOW_HORIZON_HOURS;
  const fetchSession = deps.fetchSession ?? defaultFetchSession;
  const getBrowserCookies = deps.getBrowserCookies ?? defaultGetBrowserCookies;
  const refreshBrowserTab = deps.refreshBrowserTab ?? defaultRefreshBrowserTab;
  const cdpUrl = deps.cdpUrl ?? "http://127.0.0.1:19222";
  const log = deps.log ?? (() => undefined);
  const previous = await readPreviousState(deps.statePath);

  const finish = async (
    partial: Omit<OpenDesignRenewalState, "provider" | "checkedAt" | "alertSince">,
  ): Promise<OpenDesignRenewalState> => {
    const state: OpenDesignRenewalState = {
      provider: "opendesign",
      checkedAt: now.toISOString(),
      alertSince: partial.alert ? (previous?.alert ? previous.alertSince : now.toISOString()) : null,
      ...partial,
    };
    await writeState(deps.statePath, state).catch(() => undefined);
    log(
      `opendesign-renewal: status=${state.status} horizonHours=${state.horizonHours ?? "unknown"} capture=${state.capture.result ?? "not-attempted"}${state.alert ? ` alert=${state.alert}` : ""}`,
    );
    return state;
  };

  const credential = deps.readCredential();
  if (!credential) {
    return finish({
      status: "not-configured",
      sessionExpiresAt: null,
      horizonHours: null,
      capture: { attempted: false, result: "not-attempted" },
      alert: null,
    });
  }

  let session = await fetchSession(credential);
  let horizon = horizonHours(session?.expiresAt ?? null, now);
  const needsCapture = deps.forceCapture === true || session === null || horizon === null || horizon < lowHorizon;

  if (!needsCapture) {
    return finish({
      status: "valid",
      sessionExpiresAt: session?.expiresAt ?? null,
      horizonHours: horizon,
      capture: { attempted: false, result: "not-attempted" },
      alert: null,
    });
  }

  // Re-capture from the standing browser profile.
  let captureResult: CaptureResult | string = "no-browser-session";
  try {
    let cookies: BrowserCookie[] | null = null;
    try {
      cookies = await getBrowserCookies(cdpUrl);
    } catch {
      captureResult = "cdp-unreachable";
    }
    if (cookies !== null) {
      let found = pickOpenDesignSessionCookie(cookies);
      if (!found) {
        // Loading the app can slide/set the browser's own session
        // cookie; look once more before declaring the browser bare.
        await refreshBrowserTab(cdpUrl).catch(() => undefined);
        cookies = await getBrowserCookies(cdpUrl).catch(() => []);
        found = pickOpenDesignSessionCookie(cookies);
      }
      if (found) {
        const candidate: OpenDesignCredentialLike = {
          cookie: `${found.name}=${found.value}`,
          workspaceId: credential.workspaceId,
        };
        try {
          await deps.validateCredential(candidate);
          deps.writeCredential(candidate);
          captureResult = "stored";
        } catch {
          captureResult = "validation-failed";
        }
      }
    }
  } catch (error) {
    captureResult = `error:${error instanceof Error ? error.message : String(error)}`;
  }

  if (captureResult === "stored") {
    const renewed = deps.readCredential();
    session = renewed ? await fetchSession(renewed) : null;
    horizon = horizonHours(session?.expiresAt ?? null, now);
    return finish({
      status: "renewed",
      sessionExpiresAt: session?.expiresAt ?? null,
      horizonHours: horizon,
      capture: { attempted: true, result: captureResult },
      alert: null,
    });
  }

  // Capture did not produce a credential. A still-valid stored
  // session keeps the provider reading; the alert fires when the
  // session is dead or its horizon is short, while days remain.
  const atRisk = session === null || horizon === null || horizon < lowHorizon;
  const alert = !atRisk
    ? null
    : session === null
      ? `Open Design stored session is no longer accepted and browser re-capture failed (${captureResult}); an Open Design sign-in in the laptop Helium profile is needed to re-arm renewal.`
      : horizon === null
        ? `Open Design session read returned no expiry and browser re-capture failed (${captureResult}); if the stored session lapses, an Open Design sign-in in the laptop Helium profile is needed to re-arm renewal.`
        : `Open Design session horizon is ${horizon}h (< ${lowHorizon}h) and browser re-capture failed (${captureResult}); if the gatherer stays down or the stored session lapses, an Open Design sign-in in the laptop Helium profile is needed to re-arm renewal.`;
  return finish({
    status: atRisk ? "attention" : "valid",
    sessionExpiresAt: session?.expiresAt ?? null,
    horizonHours: horizon,
    capture: { attempted: true, result: captureResult },
    alert,
  });
}
