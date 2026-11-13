/**
 * Meta (dev.meta.ai) session keeper — Drew's provider browser-auth
 * auto-renewal rule (2026-11-05) as answered for Meta on 2026-11-13
 * (decision meta-auth-keeper-6120, #6120): Meta auth is owned by the
 * provider itself. The aggregator's Meta leg holds the Meta session
 * credential in ONE provider-owned location — meta-credential.json
 * beside the keeper state file — captured and renewed by this keeper
 * via CDP against the standing signed-in laptop Helium profile, in
 * the opendesign-renewal.ts pattern: a captured credential is stored
 * only after it validates against the real portal API
 * (validate-before-store), renewal runs on cadence (low horizon) and
 * on auth failure (a stored credential that stops validating), and a
 * renewal failure raises BOTH the Muse-visible provider-auth alert
 * (this state file, projected by provider-auth-alerts.ts) and a push
 * notification (deps.notifyPush, wired in gather-quota.ts to the
 * fleet Web Push channel). The human email-code sign-in is the
 * last-resort fallback only when CDP re-capture cannot restore a
 * session, and that state is itself the alert — never a silent gap.
 *
 * This supersedes the #6079 probe-watch design, whose keeper watched
 * the meta-quota-export probe document and re-armed the exporter
 * systemd units: the exporter arrangement made renewal somebody
 * else's machinery. The exporter still runs as an independent
 * measurement exporter; this keeper reads its latest document only
 * to carry lastGoodAt continuity, and no longer arms, restarts, or
 * otherwise manages its units.
 *
 * Credential hygiene: the credential value lives only in
 * meta-credential.json (written mode 0600 via tmp+rename). It is
 * never logged, never copied into this state file (which carries
 * only presence, capture time, and expiry horizon), and never
 * rendered by the alert projection (whitelisted fields only).
 */

import { chmod, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

/** The portal team the quota read targets (exporter's DEFAULT_TEAM_ID, #1167). */
export const META_DEFAULT_TEAM_ID = "1073226932208501";

export const META_PORTAL_BASE = "https://dev.meta.ai";

/** Below this session horizon the keeper re-captures proactively. */
export const LOW_HORIZON_HOURS = 24;

/** Two exporter ticks (2 × 15 min) without a fresh document = stale. */
export const STALE_MS = 30 * 60 * 1000;

const COOKIE_HEADER = "Cookie";

export interface MetaExporterDoc {
  status: string | null;
  measuredAt: string | null;
  attemptedAt: string | null;
}

export interface MetaCredential {
  cookieHeader: string;
  capturedAt: string;
  /** Earliest expiry among the captured persistent cookies, or null
   * when every captured cookie is a session cookie (unknown horizon). */
  expiresAt: string | null;
  cookieNames: string[];
}

export type MetaCaptureResult =
  | "stored"
  | "no-browser-session"
  | "cdp-unreachable"
  | "validation-failed"
  | "not-attempted";

export interface MetaRenewalState {
  provider: "meta";
  checkedAt: string;
  status: "valid" | "renewed" | "recovered" | "attention";
  /** Session horizon of the provider-owned credential (projection fields). */
  sessionExpiresAt: string | null;
  horizonHours: number | null;
  credential: { present: boolean; capturedAt: string | null };
  capture: { attempted: boolean; result: MetaCaptureResult | string | null };
  notification: { attempted: boolean; result: string | null };
  exporterStatus: string | null;
  lastGoodAt: string | null;
  alert: string | null;
  alertSince: string | null;
}

export interface MetaBrowserCookie {
  name: string;
  value: string;
  domain: string;
  expires?: number;
}

export interface MetaRenewalDeps {
  statePath: string;
  /** The ONE provider-owned credential location; defaults beside the state file. */
  credentialPath?: string;
  latestPath?: string;
  now?: () => Date;
  staleMs?: number;
  lowHorizonHours?: number;
  teamId?: string;
  cdpUrl?: string;
  /** Injectable for tests; defaults read the exporter's document. */
  readLatest?: () => Promise<MetaExporterDoc | null>;
  /** Injectable for tests; defaults read/write the credential file. */
  readCredential?: () => Promise<MetaCredential | null>;
  writeCredential?: (credential: MetaCredential) => Promise<void>;
  /** Injectable for tests; default validates against the portal API. */
  validateCredential?: (credential: MetaCredential) => Promise<boolean>;
  /** Injectable for tests; default reads the Helium jar over CDP. */
  getBrowserCookies?: (cdpUrl: string) => Promise<MetaBrowserCookie[]>;
  /** Push dispatch on a NEW alert episode; wired by the gatherer to
   * the fleet Web Push channel. Returns an outcome summary string. */
  notifyPush?: (notice: { title: string; body: string }) => Promise<string>;
  log?: (line: string) => void;
}

export const defaultLatestPath = (): string => join(homedir(), "atlas", "meta-quota-export", "latest.json");

export const defaultCredentialPath = (statePath: string): string => join(dirname(statePath), "meta-credential.json");

export const metaQuotaUrl = (teamId: string): string =>
  `${META_PORTAL_BASE}/api/portal/teams/${teamId}/subscription-quota`;

const defaultReadLatest = async (latestPath: string): Promise<MetaExporterDoc | null> => {
  try {
    const parsed = JSON.parse(await readFile(latestPath, "utf8")) as {
      status?: unknown;
      measuredAt?: unknown;
      attemptedAt?: unknown;
    };
    return {
      status: typeof parsed.status === "string" ? parsed.status : null,
      measuredAt: typeof parsed.measuredAt === "string" ? parsed.measuredAt : null,
      attemptedAt: typeof parsed.attemptedAt === "string" ? parsed.attemptedAt : null,
    };
  } catch {
    return null;
  }
};

const defaultReadCredential = async (credentialPath: string): Promise<MetaCredential | null> => {
  try {
    const parsed = JSON.parse(await readFile(credentialPath, "utf8")) as Partial<MetaCredential>;
    if (typeof parsed.cookieHeader !== "string" || parsed.cookieHeader.length === 0) return null;
    return {
      cookieHeader: parsed.cookieHeader,
      capturedAt: typeof parsed.capturedAt === "string" ? parsed.capturedAt : "",
      expiresAt: typeof parsed.expiresAt === "string" ? parsed.expiresAt : null,
      cookieNames: Array.isArray(parsed.cookieNames) ? parsed.cookieNames.filter((n) => typeof n === "string") : [],
    };
  } catch {
    return null;
  }
};

const defaultWriteCredential = async (credentialPath: string, credential: MetaCredential): Promise<void> => {
  await mkdir(dirname(credentialPath), { recursive: true });
  const tmp = `${credentialPath}.tmp-${process.pid}`;
  await writeFile(tmp, JSON.stringify(credential, null, 2) + "\n", { encoding: "utf8", mode: 0o600 });
  await chmod(tmp, 0o600).catch(() => undefined);
  await rename(tmp, credentialPath);
};

const defaultValidateCredential = async (credential: MetaCredential, teamId: string): Promise<boolean> => {
  try {
    const res = await fetch(metaQuotaUrl(teamId), {
      headers: { [COOKIE_HEADER]: credential.cookieHeader, Accept: "application/json" },
      redirect: "manual",
    });
    await res.text().catch(() => undefined);
    return res.status === 200;
  } catch {
    return false;
  }
};

/* --- CDP cookie capture (same minimal client shape as
 * opendesign-renewal.ts; the two keepers intentionally stay
 * independent modules, one per provider). --- */

interface CdpSocket {
  addEventListener(type: string, listener: (event: { data?: unknown }) => void): void;
  send(data: string): void;
  close(): void;
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

const defaultGetBrowserCookies = async (cdpUrl: string): Promise<MetaBrowserCookie[]> => {
  const { send, close } = await cdpSession(cdpUrl);
  try {
    const answer = await send("Storage.getCookies", {});
    const result = (answer as { result?: { cookies?: MetaBrowserCookie[] } }).result;
    return Array.isArray(result?.cookies) ? result.cookies : [];
  } finally {
    close();
  }
};

/** Cookies that authenticate a dev.meta.ai portal request. */
export function pickMetaCookies(cookies: MetaBrowserCookie[]): MetaBrowserCookie[] {
  return cookies.filter((c) => {
    const d = (c.domain ?? "").replace(/^\./, "");
    return d === "dev.meta.ai" || d === "meta.ai";
  });
}

/** Build the provider-owned credential from a browser cookie jar. */
export function credentialFromCookies(cookies: MetaBrowserCookie[], now: Date): MetaCredential | null {
  const picked = pickMetaCookies(cookies);
  if (picked.length === 0) return null;
  const cookieHeader = picked.map((c) => `${c.name}=${c.value}`).join("; ");
  const expiries = picked
    .map((c) => (typeof c.expires === "number" && c.expires > 0 ? c.expires * 1000 : null))
    .filter((v): v is number => v !== null);
  return {
    cookieHeader,
    capturedAt: now.toISOString(),
    expiresAt: expiries.length > 0 ? new Date(Math.min(...expiries)).toISOString() : null,
    cookieNames: picked.map((c) => c.name),
  };
}

const horizonHours = (expiresAt: string | null, now: Date): number | null => {
  if (!expiresAt) return null;
  const t = Date.parse(expiresAt);
  if (!Number.isFinite(t)) return null;
  return Math.round(((t - now.getTime()) / 3_600_000) * 10) / 10;
};

const readPreviousState = async (statePath: string): Promise<MetaRenewalState | null> => {
  try {
    return JSON.parse(await readFile(statePath, "utf8")) as MetaRenewalState;
  } catch {
    return null;
  }
};

const writeState = async (statePath: string, state: MetaRenewalState): Promise<void> => {
  await mkdir(dirname(statePath), { recursive: true });
  const tmp = `${statePath}.tmp-${process.pid}`;
  await writeFile(tmp, JSON.stringify(state, null, 2) + "\n", "utf8");
  await rename(tmp, statePath);
};

/**
 * One keeper cycle. Fail-soft by contract: it always resolves with a
 * state object (which it also persists); it never throws, so the
 * gatherer's tick can call it unconditionally.
 */
export async function renewMetaSession(deps: MetaRenewalDeps): Promise<MetaRenewalState> {
  const now = (deps.now ?? (() => new Date()))();
  const credentialPath = deps.credentialPath ?? defaultCredentialPath(deps.statePath);
  const latestPath = deps.latestPath ?? defaultLatestPath();
  const teamId = deps.teamId ?? process.env.META_TEAM_ID ?? META_DEFAULT_TEAM_ID;
  const cdpUrl = deps.cdpUrl ?? "http://127.0.0.1:19222";
  const lowHorizon = deps.lowHorizonHours ?? LOW_HORIZON_HOURS;
  const readLatest = deps.readLatest ?? (() => defaultReadLatest(latestPath));
  const readCredential = deps.readCredential ?? (() => defaultReadCredential(credentialPath));
  const writeCredential = deps.writeCredential ?? ((c: MetaCredential) => defaultWriteCredential(credentialPath, c));
  const validateCredential = deps.validateCredential ?? ((c: MetaCredential) => defaultValidateCredential(c, teamId));
  const getBrowserCookies = deps.getBrowserCookies ?? defaultGetBrowserCookies;
  const log = deps.log ?? (() => undefined);
  const previous = await readPreviousState(deps.statePath);

  const doc = await readLatest().catch(() => null);
  const lastGoodAt =
    doc?.status === "ok" && doc.measuredAt ? doc.measuredAt : (previous?.lastGoodAt ?? null);

  const finish = async (
    partial: Omit<MetaRenewalState, "provider" | "checkedAt" | "alertSince" | "exporterStatus" | "lastGoodAt">,
  ): Promise<MetaRenewalState> => {
    let notification: MetaRenewalState["notification"] = partial.notification;
    const isNewEpisode = Boolean(partial.alert) && !previous?.alert;
    if (isNewEpisode && deps.notifyPush && partial.alert) {
      const result = await deps
        .notifyPush({
          title: "Meta provider auth needs attention",
          body: partial.alert,
        })
        .catch((error) => `error:${error instanceof Error ? error.message : String(error)}`);
      notification = { attempted: true, result };
    }
    const state: MetaRenewalState = {
      provider: "meta",
      checkedAt: now.toISOString(),
      alertSince: partial.alert ? (previous?.alert ? previous.alertSince : now.toISOString()) : null,
      exporterStatus: doc?.status ?? null,
      lastGoodAt,
      ...partial,
      notification,
    };
    await writeState(deps.statePath, state).catch(() => undefined);
    log(
      `meta-renewal: status=${state.status} credential=${state.credential.present ? "present" : "absent"} horizonHours=${state.horizonHours ?? "unknown"} capture=${state.capture.result ?? "not-attempted"} push=${state.notification.result ?? "not-attempted"}${state.alert ? ` alert=${state.alert}` : ""}`,
    );
    return state;
  };

  /** Attempt a CDP capture; store only a credential that validates. */
  const attemptCapture = async (): Promise<{
    result: MetaCaptureResult;
    credential: MetaCredential | null;
  }> => {
    let cookies: MetaBrowserCookie[];
    try {
      cookies = await getBrowserCookies(cdpUrl);
    } catch {
      return { result: "cdp-unreachable", credential: null };
    }
    const candidate = credentialFromCookies(cookies, now);
    if (!candidate) return { result: "no-browser-session", credential: null };
    const ok = await validateCredential(candidate).catch(() => false);
    if (!ok) return { result: "validation-failed", credential: null };
    await writeCredential(candidate);
    return { result: "stored", credential: candidate };
  };

  const stored = await readCredential().catch(() => null);
  if (stored) {
    const valid = await validateCredential(stored).catch(() => false);
    const horizon = horizonHours(stored.expiresAt, now);
    if (valid && (horizon === null || horizon > lowHorizon)) {
      return finish({
        status: "valid",
        sessionExpiresAt: stored.expiresAt,
        horizonHours: horizon,
        credential: { present: true, capturedAt: stored.capturedAt || null },
        capture: { attempted: false, result: "not-attempted" },
        notification: { attempted: false, result: null },
        alert: null,
      });
    }
    if (valid) {
      // Cadence renewal: still valid, but inside the low horizon —
      // re-capture now instead of expiring between ticks.
      const attempt = await attemptCapture();
      const active = attempt.credential ?? stored;
      return finish({
        status: attempt.credential ? "renewed" : "valid",
        sessionExpiresAt: active.expiresAt,
        horizonHours: horizonHours(active.expiresAt, now),
        credential: { present: true, capturedAt: active.capturedAt || null },
        capture: { attempted: true, result: attempt.result },
        notification: { attempted: false, result: null },
        alert: null,
      });
    }
    // Auth failure: the stored credential stopped validating —
    // re-capture from the browser and store only on validation.
    const attempt = await attemptCapture();
    if (attempt.credential) {
      return finish({
        status: "recovered",
        sessionExpiresAt: attempt.credential.expiresAt,
        horizonHours: horizonHours(attempt.credential.expiresAt, now),
        credential: { present: true, capturedAt: attempt.credential.capturedAt },
        capture: { attempted: true, result: attempt.result },
        notification: { attempted: false, result: null },
        alert: null,
      });
    }
    return finish({
      status: "attention",
      sessionExpiresAt: stored.expiresAt,
      horizonHours: horizon,
      credential: { present: true, capturedAt: stored.capturedAt || null },
      capture: { attempted: true, result: attempt.result },
      notification: { attempted: false, result: null },
      alert: `Meta provider credential stopped validating and CDP re-capture from the laptop Helium profile did not restore it (capture=${attempt.result}). Last-resort fallback: re-login to dev.meta.ai in the laptop Helium profile (Meta emails a login code). Meta quota reads ride the stored credential and may lapse until then${lastGoodAt ? `; last good exporter read ${lastGoodAt}` : ""}.`,
    });
  }

  // No provider-owned credential yet: first capture.
  const attempt = await attemptCapture();
  if (attempt.credential) {
    return finish({
      status: "renewed",
      sessionExpiresAt: attempt.credential.expiresAt,
      horizonHours: horizonHours(attempt.credential.expiresAt, now),
      credential: { present: true, capturedAt: attempt.credential.capturedAt },
      capture: { attempted: true, result: attempt.result },
      notification: { attempted: false, result: null },
      alert: null,
    });
  }
  return finish({
    status: "attention",
    sessionExpiresAt: null,
    horizonHours: null,
    credential: { present: false, capturedAt: null },
    capture: { attempted: true, result: attempt.result },
    notification: { attempted: false, result: null },
    alert: `Meta provider holds no credential and CDP capture from the laptop Helium profile did not produce a valid one (capture=${attempt.result}). Last-resort fallback: sign in to dev.meta.ai in the laptop Helium profile (Meta emails a login code), after which the keeper captures and renews it on its own.`,
  });
}
