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
 *
 * Jar restore (#10839): the portal reads themselves — the fork quota
 * provider and the exporter — ride the LIVE browser session, not the
 * stored credential, so a Helium profile restart that drops the
 * session-scoped cookie (llama_dev_sess) darkens the Meta leg while
 * this keeper still sees a validating credential; on 2026-11-16 that
 * stayed silent for hours because the keeper recorded the exporter's
 * session-expired status and never acted on it. Now, when the read
 * path reports session-expired across two consecutive keeper cycles
 * (or a cadence capture proves the jar holds no live session) and
 * the stored credential still validates, the keeper restores the jar
 * from the stored credential — the reverse of capture,
 * Storage.setCookies over the same CDP endpoint — under an
 * exponential backoff, recording each attempt as a restore event in
 * this state file and the gatherer log (cookie names only, never
 * values). Restores that keep failing raise the attention alert
 * after RESTORE_ALERT_AFTER_ATTEMPTS attempts: a restore that does
 * not revive the session becomes a signal, not a silent loop.
 *
 * Stale-readings alert (#15114): STALE_MS was declared for this
 * keeper but never consulted, so a valid stored credential made
 * every cycle report "valid" while the exporter's last good read
 * aged for hours — in the 2026-11-21 incident the readings were
 * ~4 days stale (408 consecutive exporter failures) before the
 * watchdog escalation surfaced it, and the restore-stuck alert
 * alone needs ~2.5 h of backoff (30 min + 2 h) to fire. Now, when
 * the last good exporter read is older than STALE_MS and the
 * current exporter document is not a fresh ok read, the keeper
 * raises the attention alert (state file + push on the new
 * episode) in that same cycle, naming the exporter status and
 * the last good read — session expiry becomes a loud, owned
 * alert at the staleness bound, not a discovery days later.
 */

import { chmod, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { requireHome } from "./home.ts";

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
  /** The captured cookies themselves (name/value/domain/expiry), so
   * the jar restore (#10839) can re-seed them verbatim. Absent on
   * credentials stored before the field existed — the restore then
   * falls back to parsing cookieHeader (see
   * restoreCookiesFromCredential). Values live only in this file,
   * under the same hygiene rules as cookieHeader. */
  cookies?: MetaBrowserCookie[];
}

export type MetaCaptureResult =
  | "stored"
  | "no-browser-session"
  | "cdp-unreachable"
  | "validation-failed"
  | "not-attempted";

export type MetaRestoreResult = "restored" | "restore-failed" | "no-cookies" | "not-attempted";

/** Browser-jar restore bookkeeping (#10839). Carried in the state
 * file so the backoff survives keeper restarts; cookie values are
 * never copied here — counts and timestamps only. */
export interface MetaRestoreState {
  /** A restore was attempted in the cycle that wrote this state. */
  attempted: boolean;
  /** This cycle's restore outcome ("not-attempted" when none ran). */
  result: MetaRestoreResult | string | null;
  /** Consecutive restore attempts in the current dead-session episode. */
  attempts: number;
  lastAttemptAt: string | null;
  /** Backoff gate: no new attempt before this instant. */
  nextAttemptAt: string | null;
}

/** After this many restores in one episode without the read path
 * recovering, the keeper raises the attention alert (the restore is
 * not reviving the session — a human needs the signal). */
export const RESTORE_ALERT_AFTER_ATTEMPTS = 3;

/** Exponential restore backoff: 30 min, 2 h, then 6 h flat — a few
 * attempts a day at most, never a hot loop against CDP. */
export const restoreBackoffMs = (attempts: number): number =>
  attempts <= 1 ? 30 * 60 * 1000 : attempts === 2 ? 2 * 3_600_000 : 6 * 3_600_000;

export interface MetaRenewalState {
  provider: "meta";
  checkedAt: string;
  status: "valid" | "renewed" | "recovered" | "attention";
  /** Session horizon of the provider-owned credential (projection fields). */
  sessionExpiresAt: string | null;
  horizonHours: number | null;
  credential: { present: boolean; capturedAt: string | null };
  capture: { attempted: boolean; result: MetaCaptureResult | string | null };
  restore: MetaRestoreState;
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
  /** Attributes CDP Storage.getCookies reports; kept so a jar restore
   * writes the cookie back as it was captured (#10878 follow-up). */
  path?: string;
  secure?: boolean;
  httpOnly?: boolean;
  sameSite?: "Strict" | "Lax" | "None";
}

/** The Storage.setCookies parameter for one cookie: the captured
 * attributes verbatim, with path "/" only when none was captured.
 * Dropping secure/httpOnly/sameSite would hand a restored session
 * cookie to page scripts and plain-http requests. */
export function cdpSetCookieParam(c: MetaBrowserCookie): Record<string, unknown> {
  return {
    name: c.name,
    value: c.value,
    domain: c.domain,
    path: typeof c.path === "string" && c.path.length > 0 ? c.path : "/",
    ...(typeof c.expires === "number" && c.expires > 0 ? { expires: c.expires } : {}),
    ...(typeof c.secure === "boolean" ? { secure: c.secure } : {}),
    ...(typeof c.httpOnly === "boolean" ? { httpOnly: c.httpOnly } : {}),
    ...(c.sameSite === "Strict" || c.sameSite === "Lax" || c.sameSite === "None" ? { sameSite: c.sameSite } : {}),
  };
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
  /** Injectable for tests; default writes cookies into the Helium
   * jar over CDP (Storage.setCookies) — the jar restore (#10839). */
  setBrowserCookies?: (cdpUrl: string, cookies: MetaBrowserCookie[]) => Promise<void>;
  /** Push dispatch on a NEW alert episode; wired by the gatherer to
   * the fleet Web Push channel. Returns an outcome summary string. */
  notifyPush?: (notice: { title: string; body: string }) => Promise<string>;
  log?: (line: string) => void;
}

export const defaultLatestPath = (): string =>
  join(requireHome("the meta quota export's latest.json"), "atlas", "meta-quota-export", "latest.json");

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
    const cookies: MetaBrowserCookie[] = Array.isArray(parsed.cookies)
      ? parsed.cookies.filter(
          (c): c is MetaBrowserCookie =>
            typeof c === "object" &&
            c !== null &&
            typeof (c as MetaBrowserCookie).name === "string" &&
            typeof (c as MetaBrowserCookie).value === "string" &&
            typeof (c as MetaBrowserCookie).domain === "string",
        )
      : [];
    return {
      cookieHeader: parsed.cookieHeader,
      capturedAt: typeof parsed.capturedAt === "string" ? parsed.capturedAt : "",
      expiresAt: typeof parsed.expiresAt === "string" ? parsed.expiresAt : null,
      cookieNames: Array.isArray(parsed.cookieNames) ? parsed.cookieNames.filter((n) => typeof n === "string") : [],
      ...(cookies.length > 0 ? { cookies } : {}),
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

/** The jar restore (#10839): write cookies back into the Helium jar
 * over the same CDP session shape capture reads from. A protocol
 * error answer is a failure, not a silent no-op. */
const defaultSetBrowserCookies = async (cdpUrl: string, cookies: MetaBrowserCookie[]): Promise<void> => {
  const { send, close } = await cdpSession(cdpUrl);
  try {
    const answer = await send("Storage.setCookies", {
      cookies: cookies.map(cdpSetCookieParam),
    });
    if ((answer as { error?: unknown }).error) throw new Error("CDP Storage.setCookies returned an error");
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
    cookies: picked,
  };
}

/**
 * The cookie set a jar restore (#10839) writes back: the captured
 * cookies verbatim when the credential carries them; for credentials
 * stored before the cookies field existed, the cookieHeader pairs
 * re-homed on .meta.ai (a domain cookie there reaches dev.meta.ai)
 * with the credential's own expiry horizon as their persistence.
 */
export function restoreCookiesFromCredential(credential: MetaCredential): MetaBrowserCookie[] {
  if (Array.isArray(credential.cookies) && credential.cookies.length > 0) {
    return pickMetaCookies(credential.cookies);
  }
  const expiresMs = credential.expiresAt ? Date.parse(credential.expiresAt) : Number.NaN;
  const expires = Number.isFinite(expiresMs) && expiresMs > 0 ? expiresMs / 1000 : undefined;
  const out: MetaBrowserCookie[] = [];
  for (const part of credential.cookieHeader.split(";")) {
    const eq = part.indexOf("=");
    if (eq <= 0) continue;
    const name = part.slice(0, eq).trim();
    const value = part.slice(eq + 1).trim();
    if (!name) continue;
    // A header-only credential lost the attributes; dev.meta.ai is
    // https-only, so the restored cookie is at least marked secure.
    out.push({ name, value, domain: ".meta.ai", secure: true, ...(expires !== undefined ? { expires } : {}) });
  }
  return out;
}

const emptyRestore = (): MetaRestoreState => ({
  attempted: false,
  result: null,
  attempts: 0,
  lastAttemptAt: null,
  nextAttemptAt: null,
});

/** Previous state files may predate the restore block (#10839) or
 * carry a partial one; normalize defensively rather than trusting
 * the cast. */
const normalizeRestore = (v: unknown): MetaRestoreState => {
  if (typeof v !== "object" || v === null) return emptyRestore();
  const rec = v as Record<string, unknown>;
  return {
    attempted: rec.attempted === true,
    result: typeof rec.result === "string" ? rec.result : null,
    attempts:
      typeof rec.attempts === "number" && Number.isFinite(rec.attempts) && rec.attempts > 0
        ? Math.floor(rec.attempts)
        : 0,
    lastAttemptAt: typeof rec.lastAttemptAt === "string" ? rec.lastAttemptAt : null,
    nextAttemptAt: typeof rec.nextAttemptAt === "string" ? rec.nextAttemptAt : null,
  };
};

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
  const setBrowserCookies = deps.setBrowserCookies ?? defaultSetBrowserCookies;
  const log = deps.log ?? (() => undefined);
  const previous = await readPreviousState(deps.statePath);

  const doc = await readLatest().catch(() => null);
  const lastGoodAt = doc?.status === "ok" && doc.measuredAt ? doc.measuredAt : (previous?.lastGoodAt ?? null);

  // Stale readings (#15114): the last good exporter read is older
  // than the stale bound and this cycle's document is not a fresh
  // ok read (a fresh ok read sets lastGoodAt to ~now above, so it
  // can never trip this). Unknown/unparseable lastGoodAt never
  // trips it — no evidence of staleness is not staleness.
  const staleMs = deps.staleMs ?? STALE_MS;
  const lastGoodMs = lastGoodAt ? Date.parse(lastGoodAt) : Number.NaN;
  const readingsStale = Number.isFinite(lastGoodMs) && now.getTime() - lastGoodMs > staleMs;
  const staleAlertText = (): string =>
    `Meta portal quota readings are stale: the exporter last produced a good read ${lastGoodAt} (older than the ${Math.round(staleMs / 60_000)} min stale bound) and its current status is ${doc?.status ?? "not reporting"}, though the stored credential itself still validates against the portal API. The live browser session the exporter rides is not producing readings; last-resort fallback: re-login to dev.meta.ai in the laptop Helium profile (Meta emails a login code).`;

  // Jar-restore episode state (#10839): carried across cycles via the
  // state file. A healthy exporter read closes any open episode; the
  // per-cycle fields (attempted/result) restart every cycle.
  let restore: MetaRestoreState = normalizeRestore(previous?.restore);
  if (doc?.status === "ok") {
    restore = {
      attempted: false,
      result: "not-attempted",
      attempts: 0,
      lastAttemptAt: restore.lastAttemptAt,
      nextAttemptAt: null,
    };
  } else {
    restore = { ...restore, attempted: false, result: "not-attempted" };
  }
  /** Set by the restore evaluation when this cycle saw positive
   * evidence the browser session is dead (persistent exporter
   * session-expired, or a capture that found no live session). */
  let sessionDeadSignal = false;

  const finish = async (
    partial: Omit<
      MetaRenewalState,
      "provider" | "checkedAt" | "alertSince" | "exporterStatus" | "lastGoodAt" | "restore"
    >,
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
      restore,
      notification,
    };
    await writeState(deps.statePath, state).catch(() => undefined);
    log(
      `meta-renewal: status=${state.status} credential=${state.credential.present ? "present" : "absent"} horizonHours=${state.horizonHours ?? "unknown"} capture=${state.capture.result ?? "not-attempted"} restore=${state.restore.result ?? "not-attempted"}${state.restore.attempts > 0 ? ` restoreAttempts=${state.restore.attempts}` : ""} push=${state.notification.result ?? "not-attempted"}${state.alert ? ` alert=${state.alert}` : ""}`,
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

  /**
   * Jar restore (#10839): when the read path's browser session is
   * dead but the stored credential still validates, re-seed the jar
   * from the stored credential — the reverse of capture. Only called
   * from branches where `stored` validated: restoring a credential
   * that does not itself validate would loop pointlessly, and that
   * path raises the attention alert instead. Backoff-gated via the
   * persisted restore state; every attempt is logged as a restore
   * event (cookie names only, never values).
   */
  const attemptRestoreIfNeeded = async (jarProvenDead: boolean): Promise<void> => {
    const persistentExpiry = doc?.status === "session-expired" && previous?.exporterStatus === "session-expired";
    sessionDeadSignal = persistentExpiry || jarProvenDead;
    if (!sessionDeadSignal || !stored) return;
    if (restore.nextAttemptAt && now.getTime() < Date.parse(restore.nextAttemptAt)) return;
    const cookies = restoreCookiesFromCredential(stored);
    let result: MetaRestoreResult = "restored";
    if (cookies.length === 0) {
      result = "no-cookies";
    } else {
      try {
        await setBrowserCookies(cdpUrl, cookies);
      } catch {
        result = "restore-failed";
      }
    }
    const attempts = restore.attempts + 1;
    restore = {
      attempted: true,
      result,
      attempts,
      lastAttemptAt: now.toISOString(),
      nextAttemptAt: new Date(now.getTime() + restoreBackoffMs(attempts)).toISOString(),
    };
    log(
      `meta-renewal: browser-jar restore event result=${result} attempt=${attempts} cookies=${cookies.map((c) => c.name).join(",") || "none"} nextAttemptAt=${restore.nextAttemptAt}`,
    );
  };

  /** Restores are not reviving the session: escalate to the same
   * attention surface a dead credential uses. */
  const restoreStuck = (): boolean => sessionDeadSignal && restore.attempts >= RESTORE_ALERT_AFTER_ATTEMPTS;

  const restoreAlertText = (): string =>
    `Meta portal quota reads are down: the read path reports session-expired and the keeper has restored the laptop Helium cookie jar from the stored credential ${restore.attempts} times without the session recovering, though the stored credential itself still validates against the portal API. Last-resort fallback: re-login to dev.meta.ai in the laptop Helium profile (Meta emails a login code)${lastGoodAt ? `; last good exporter read ${lastGoodAt}` : ""}.`;

  if (stored) {
    const valid = await validateCredential(stored).catch(() => false);
    const horizon = horizonHours(stored.expiresAt, now);
    if (valid && (horizon === null || horizon > lowHorizon)) {
      // The credential is healthy — but the read path rides the live
      // browser session, which may be dead anyway (#10839).
      await attemptRestoreIfNeeded(false);
      if (restoreStuck()) {
        return finish({
          status: "attention",
          sessionExpiresAt: stored.expiresAt,
          horizonHours: horizon,
          credential: { present: true, capturedAt: stored.capturedAt || null },
          capture: { attempted: false, result: "not-attempted" },
          notification: { attempted: false, result: null },
          alert: restoreAlertText(),
        });
      }
      if (readingsStale) {
        return finish({
          status: "attention",
          sessionExpiresAt: stored.expiresAt,
          horizonHours: horizon,
          credential: { present: true, capturedAt: stored.capturedAt || null },
          capture: { attempted: false, result: "not-attempted" },
          notification: { attempted: false, result: null },
          alert: staleAlertText(),
        });
      }
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
      // Capture produced nothing while the stored credential still
      // validates: a jar with no live session (or only non-session
      // cookies) is the #10839 shape — restore it from the stored
      // credential instead of silently riding the stored one.
      if (attempt.result === "no-browser-session" || attempt.result === "validation-failed") {
        await attemptRestoreIfNeeded(true);
        if (restoreStuck()) {
          return finish({
            status: "attention",
            sessionExpiresAt: stored.expiresAt,
            horizonHours: horizon,
            credential: { present: true, capturedAt: stored.capturedAt || null },
            capture: { attempted: true, result: attempt.result },
            notification: { attempted: false, result: null },
            alert: restoreAlertText(),
          });
        }
      }
      if (readingsStale) {
        return finish({
          status: "attention",
          sessionExpiresAt: stored.expiresAt,
          horizonHours: horizon,
          credential: { present: true, capturedAt: stored.capturedAt || null },
          capture: { attempted: true, result: attempt.result },
          notification: { attempted: false, result: null },
          alert: staleAlertText(),
        });
      }
      return finish({
        status: "valid",
        sessionExpiresAt: stored.expiresAt,
        horizonHours: horizon,
        credential: { present: true, capturedAt: stored.capturedAt || null },
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
      alert: `Meta provider credential stopped validating and CDP re-capture from the laptop Helium profile did not restore it (capture=${attempt.result}). Last-resort fallback: re-login to dev.meta.ai in the laptop Helium profile (Meta emails a login code). Meta quota reads ride the live browser session and lapse until then${lastGoodAt ? `; last good exporter read ${lastGoodAt}` : ""}.`,
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
