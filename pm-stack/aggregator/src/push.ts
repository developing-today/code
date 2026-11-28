// Web Push sending (RFC 8030 + VAPID, design doc
// 2026-11-01T03-54-00Z_design_pwa_push_notifications.md §4) via the
// `web-push` package. The VAPID public key ships in code (it is
// public by design); the private key is fetched BY NAME from
// Bitwarden Secrets Manager at runtime (`pm-aggregator.
// vapid-private-key`) and held only in memory — never in the repo,
// the Nix store, or any file. Sending is outbound only: the
// aggregator POSTs to the subscription's vendor push endpoint.
import webpush from "web-push";
import { forgetBwsSecret, resolveBwsSecret } from "./secrets.ts";

export const VAPID_PUBLIC_KEY =
  "BNBCc8IjMrerQY8aGviwq71OoJwf6BCnVOJd0UMAnSwRuXJ8nlIkfh2TFEmAPyPGKU2OOpDEnqdhIGTcBv4Jb2E";
export const VAPID_SUBJECT = "mailto:drewrypope@gmail.com";
export const VAPID_PRIVATE_KEY_SECRET = "pm-aggregator.vapid-private-key";

export interface PushTarget {
  endpoint: string;
  p256dh: string;
  auth: string;
}

export type PushOutcome = { ok: true; status: number } | { ok: false; gone: boolean; status?: number; error: string };

let configured = false;
let configurePromise: Promise<boolean> | null = null;

/** Lazily fetch the VAPID private key by name and configure web-push.
 *
 * Only success is memoized (#3729): when an attempt settles false —
 * the secret resolving to null on a transient bws problem, or the
 * configure throwing — the pending promise clears itself, so the next
 * call re-resolves and can succeed without a process restart (the
 * secrets layer already heals its own null cache after 5 minutes; an
 * unbounded latch here defeated that). Concurrent callers still share
 * the one in-flight attempt. Same retry-after-failure shape as
 * createAlertsStoreGetter in alerts.ts.
 * Lazy-init contract (success-only memoization): README "Lazy-initialization contract" (#3879). */
export function ensurePushConfigured(): Promise<boolean> {
  if (configured) return Promise.resolve(true);
  if (!configurePromise) {
    const pending = (async () => {
      const priv = await resolveBwsSecret(VAPID_PRIVATE_KEY_SECRET);
      if (!priv) return false;
      webpush.setVapidDetails(VAPID_SUBJECT, VAPID_PUBLIC_KEY, priv);
      configured = true;
      return true;
    })().catch(() => false);
    configurePromise = pending;
    void pending.then((ok) => {
      if (!ok && configurePromise === pending) configurePromise = null;
    });
  }
  return configurePromise;
}

/** Send one push payload to one subscription. 404/410 => gone. */
export async function sendPush(target: PushTarget, payload: unknown): Promise<PushOutcome> {
  if (!(await ensurePushConfigured())) {
    return { ok: false, gone: false, error: "VAPID private key unavailable (bws)" };
  }
  try {
    const res = await webpush.sendNotification(
      { endpoint: target.endpoint, keys: { p256dh: target.p256dh, auth: target.auth } },
      JSON.stringify(payload),
      { TTL: 3600, urgency: "high" },
    );
    return { ok: true, status: res.statusCode };
  } catch (err) {
    const status = (err as { statusCode?: number })?.statusCode;
    if (status === 401 || status === 403) {
      // VAPID rejected — the cached key may be stale after a rotation.
      forgetBwsSecret(VAPID_PRIVATE_KEY_SECRET);
      configured = false;
      configurePromise = null;
    }
    return {
      ok: false,
      gone: status === 404 || status === 410,
      status,
      error: err instanceof Error ? err.message : "send failed",
    };
  }
}

// --- /api/sw-probe reporting (design §6 step 2) ---------------------------
// Pages report the service-worker registrations they see; the counts
// live in memory (observational only — no content, no persistence).

export interface SwProbeSummary {
  reports: number;
  lastReportAt: string | null;
  foreignSeen: Record<string, number>;
  evictedSeen: Record<string, number>;
}

const probe: SwProbeSummary = { reports: 0, lastReportAt: null, foreignSeen: {}, evictedSeen: {} };

/** #10539: bound the observational probe maps. POST /api/sw-probe accepts
 * arbitrary client-supplied strings and the old code stored every distinct
 * one forever, so repeated requests grew process memory and the GET
 * summary payload without limit (request bodies are capped at 100KB, but
 * distinct keys accumulate across requests). Each map keeps at most
 * MAX_SW_PROBE_KEYS distinct keys; keys past the cap (or longer than
 * MAX_SW_PROBE_KEY_LENGTH) are dropped, not stored — same shape as the
 * #4981 class-key cap in alerts.ts. Counts for already-tracked keys still
 * increment, so the probe keeps its "what did we see" signal at a fixed
 * memory ceiling (~2 * 64 * ~512B worst case). */
export const MAX_SW_PROBE_KEYS = 64;
/** #10539: longest accepted probe key. Real script URLs/paths are short;
 * anything longer is not a plausible service-worker script reference. */
export const MAX_SW_PROBE_KEY_LENGTH = 512;

/** #10539: record one probe key, enforcing the length + count caps. */
function noteSwProbeKey(map: Record<string, number>, key: string): void {
  if (key.length === 0 || key.length > MAX_SW_PROBE_KEY_LENGTH) return;
  // "__proto__" assignment on a plain object hits the prototype setter
  // instead of creating an own key, so it can never be counted — skip it.
  if (key === "__proto__") return;
  if (!Object.hasOwn(map, key) && Object.keys(map).length >= MAX_SW_PROBE_KEYS) return;
  map[key] = (map[key] ?? 0) + 1;
}

/** Our own worker's script path — any other registration is foreign. */
export const SW_SCRIPT_PATH = "/sw.js";

export function isForeignSwScript(scriptUrl: string): boolean {
  try {
    return new URL(scriptUrl).pathname !== SW_SCRIPT_PATH;
  } catch {
    return true;
  }
}

export function recordSwProbe(report: { registrations?: unknown; evicted?: unknown }): SwProbeSummary {
  probe.reports += 1;
  probe.lastReportAt = new Date().toISOString();
  const regs = Array.isArray(report.registrations) ? report.registrations : [];
  for (const r of regs) {
    if (typeof r === "string" && isForeignSwScript(r)) {
      noteSwProbeKey(probe.foreignSeen, r);
    }
  }
  const evicted = Array.isArray(report.evicted) ? report.evicted : [];
  for (const r of evicted) {
    if (typeof r === "string") noteSwProbeKey(probe.evictedSeen, r);
  }
  return swProbeSummary();
}

export function swProbeSummary(): SwProbeSummary {
  return {
    reports: probe.reports,
    lastReportAt: probe.lastReportAt,
    foreignSeen: { ...probe.foreignSeen },
    evictedSeen: { ...probe.evictedSeen },
  };
}
