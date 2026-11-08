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

/** Lazily fetch the VAPID private key by name and configure web-push. */
export function ensurePushConfigured(): Promise<boolean> {
  if (configured) return Promise.resolve(true);
  if (!configurePromise) {
    configurePromise = (async () => {
      const priv = await resolveBwsSecret(VAPID_PRIVATE_KEY_SECRET);
      if (!priv) return false;
      webpush.setVapidDetails(VAPID_SUBJECT, VAPID_PUBLIC_KEY, priv);
      configured = true;
      return true;
    })().catch(() => false);
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
      probe.foreignSeen[r] = (probe.foreignSeen[r] ?? 0) + 1;
    }
  }
  const evicted = Array.isArray(report.evicted) ? report.evicted : [];
  for (const r of evicted) {
    if (typeof r === "string") probe.evictedSeen[r] = (probe.evictedSeen[r] ?? 0) + 1;
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
