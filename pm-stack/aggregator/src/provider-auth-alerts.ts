// Provider-auth keeper alerts (#6671): the board/readout projection of
// the two provider browser-auth keepers' state files —
// opendesign-renewal.json (src/opendesign-renewal.ts) and
// meta-renewal.json (src/meta-renewal.ts), written beside
// quota-current.json in the aggregator state dir.
//
// Why this exists: the keepers record `status: "attention"` + `alert` +
// `alertSince` when a provider session is dying, but before #6671
// nothing outside the keeper modules and their tests read those files,
// so a failing renewal was invisible where a human looks every day —
// exactly the silent-expiry failure mode Drew's 2026-11-05
// provider-auth rule exists to prevent. The gatherer embeds this
// projection in quota-current.json (one gather cycle), the overlay
// carries it onto the board's QuotaView, and renderQuota shows every
// non-ok keeper as a visible badge.
//
// Truth contract: a missing state file, an unparsable one, and a stale
// one (checkedAt older than PROVIDER_AUTH_STALE_MS) each project as
// their own visible reporting state — "keeper not reporting" — never
// as silence and never as ok. Only whitelisted status/alert fields are
// projected; no credential or session value is read or rendered (the
// state files do not carry any, and the whitelist keeps it that way).

import { readFile } from "node:fs/promises";
import { join } from "node:path";

export type ProviderAuthProvider = "opendesign" | "meta";

export type ProviderAuthReporting = "attention" | "ok" | "missing" | "stale" | "invalid";

export interface ProviderAuthAlert {
  provider: ProviderAuthProvider;
  /** The keeper's own status string ("valid", "attention", ...), or
   * null when no state could be read at all. */
  status: string | null;
  alert: string | null;
  alertSince: string | null;
  checkedAt: string | null;
  /** OpenDesign: when the current session expires (the horizon the
   * keeper records). Null for meta / when unknown. */
  sessionExpiresAt: string | null;
  /** OpenDesign: hours of session horizon left at checkedAt. */
  horizonHours: number | null;
  /** Meta: the exporter's last good read. Null for opendesign. */
  lastGoodAt: string | null;
  reporting: ProviderAuthReporting;
}

/** Keepers run on every gather tick (15 min); a state file older than
 * three ticks has missed runs — the keeper is not reporting. */
export const PROVIDER_AUTH_STALE_MS = 45 * 60_000;

export const PROVIDER_AUTH_FILES: Record<ProviderAuthProvider, string> = {
  opendesign: "opendesign-renewal.json",
  meta: "meta-renewal.json",
};

const str = (v: unknown): string | null => (typeof v === "string" ? v : null);
const num = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);

/** Project one keeper state document (parsed JSON, or null when the
 * file could not be read) into the board shape. Pure. */
export function projectKeeperState(
  provider: ProviderAuthProvider,
  doc: unknown,
  now = new Date(),
  staleMs = PROVIDER_AUTH_STALE_MS,
): ProviderAuthAlert {
  const base: ProviderAuthAlert = {
    provider,
    status: null,
    alert: null,
    alertSince: null,
    checkedAt: null,
    sessionExpiresAt: null,
    horizonHours: null,
    lastGoodAt: null,
    reporting: "missing",
  };
  if (doc === null || doc === undefined) return base;
  if (typeof doc !== "object" || Array.isArray(doc)) return { ...base, reporting: "invalid" };
  const rec = doc as Record<string, unknown>;
  const checkedAt = str(rec.checkedAt);
  if (!checkedAt || Number.isNaN(Date.parse(checkedAt))) return { ...base, reporting: "invalid" };
  const out: ProviderAuthAlert = {
    ...base,
    status: str(rec.status),
    alert: str(rec.alert),
    alertSince: str(rec.alertSince),
    checkedAt,
    sessionExpiresAt: str(rec.sessionExpiresAt),
    horizonHours: num(rec.horizonHours),
    lastGoodAt: str(rec.lastGoodAt),
    reporting: "ok",
  };
  if (out.status === "attention" || out.alert) {
    out.reporting = "attention";
  } else if (now.getTime() - Date.parse(checkedAt) > staleMs) {
    out.reporting = "stale";
  }
  return out;
}

export function isProviderAuthAlert(v: unknown): v is ProviderAuthAlert {
  if (typeof v !== "object" || v === null || Array.isArray(v)) return false;
  const rec = v as Record<string, unknown>;
  return (
    (rec.provider === "opendesign" || rec.provider === "meta") &&
    (rec.reporting === "attention" ||
      rec.reporting === "ok" ||
      rec.reporting === "missing" ||
      rec.reporting === "stale" ||
      rec.reporting === "invalid")
  );
}

/** The keepers that need a human's eye: everything not reporting ok. */
export function providerAuthProblems(alerts: ProviderAuthAlert[]): ProviderAuthAlert[] {
  return alerts.filter((a) => a.reporting !== "ok");
}

/** One human line per problem keeper, for readout/log surfaces. */
export function providerAuthProblemLines(alerts: ProviderAuthAlert[]): string[] {
  return providerAuthProblems(alerts).map((a) => {
    switch (a.reporting) {
      case "attention": {
        const since = a.alertSince ? ` since ${a.alertSince}` : "";
        const horizon = a.sessionExpiresAt
          ? `; session expires ${a.sessionExpiresAt}`
          : a.lastGoodAt
            ? `; last good ${a.lastGoodAt}`
            : "";
        return `${a.provider} provider auth: ${a.alert ?? a.status ?? "attention"}${since}${horizon}`;
      }
      case "stale":
        return `${a.provider} provider auth: keeper not reporting — last check ${a.checkedAt ?? "unknown"} is stale`;
      case "invalid":
        return `${a.provider} provider auth: keeper not reporting — state file is invalid`;
      default:
        return `${a.provider} provider auth: keeper not reporting — no state file`;
    }
  });
}

/** Re-derive freshness at serve time (#6671, Codex P2 on the first
 * cut): the gather-time `reporting` classification freezes when the
 * gatherer stops — a keeper whose last tick said ok would render ok
 * forever. An ok keeper whose checkedAt is older than the stale
 * horizon at serve time is reclassified stale; attention (already
 * visible), missing, stale, and invalid pass through unchanged. */
export function reevaluateProviderAuth(
  alerts: ProviderAuthAlert[],
  now = new Date(),
  staleMs = PROVIDER_AUTH_STALE_MS,
): ProviderAuthAlert[] {
  return alerts.map((a) => {
    if (a.reporting !== "ok" || !a.checkedAt) return a;
    return now.getTime() - Date.parse(a.checkedAt) > staleMs
      ? { ...a, reporting: "stale" as const }
      : a;
  });
}

export interface ProviderAuthPaths {
  opendesign: string;
  meta: string;
}

export function providerAuthPathsIn(dir: string): ProviderAuthPaths {
  return {
    opendesign: join(dir, PROVIDER_AUTH_FILES.opendesign),
    meta: join(dir, PROVIDER_AUTH_FILES.meta),
  };
}

/** Read both keeper state files and project them. A file that cannot
 * be read projects as "missing"; one that cannot be parsed projects
 * as "invalid" — the read distinguishes the two so a corrupt file is
 * not mistaken for an absent keeper. */
export async function readProviderAuthAlerts(
  paths: ProviderAuthPaths,
  opts: { now?: Date; staleMs?: number; readText?: (path: string) => Promise<string> } = {},
): Promise<ProviderAuthAlert[]> {
  const readText = opts.readText ?? ((p: string) => readFile(p, "utf8"));
  const one = async (provider: ProviderAuthProvider): Promise<ProviderAuthAlert> => {
    let text: string;
    try {
      text = await readText(paths[provider]);
    } catch {
      return projectKeeperState(provider, null, opts.now, opts.staleMs);
    }
    try {
      return projectKeeperState(provider, JSON.parse(text) as unknown, opts.now, opts.staleMs);
    } catch {
      return { ...projectKeeperState(provider, null, opts.now, opts.staleMs), reporting: "invalid" };
    }
  };
  return [await one("opendesign"), await one("meta")];
}
