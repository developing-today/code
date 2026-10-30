// The quota gatherer's quota-current.json, applied to the quota view
// (#1151, final visibility hop). The gatherer (scripts/gather-quota.ts)
// writes the latest measured state per provider into the aggregator's
// own data dir every 15 minutes; this module is the reader side: pure
// functions over the parsed file, so the precedence semantics are
// pinned by tests and the live source only does the file IO.
//
// Semantics (task #1151):
// - An entry whose state is "ok" and whose measuredAt is fresh
//   (<= FRESH_MS old) takes precedence for the pool whose id equals
//   the entry's serviceId: its percent windows replace the pool's
//   dated-read windows, and the pool state is recomputed from them.
// - An entry that is an error, or "ok" but past the freshness bound,
//   surfaces as the pool's state (error/stale, with the reason) —
//   never as zeros, and the pool's older windows are never silently
//   presented as current: the freshness stamp rides on the pool.
// - Pools with no entry keep their existing values untouched.
// - Several entries can share a serviceId (one per credential): a
//   fresh ok entry wins over an error entry; otherwise the newest
//   entry decides the surfaced state.
import { poolState } from "./derive.ts";
import { computeChoices } from "./quota-guidance.ts";
import type {
  GathererSummary,
  PoolFreshness,
  QuotaCurrentEntry,
  QuotaCurrentFile,
  QuotaPool,
  QuotaView,
  QuotaWindow,
  SaasService,
  UsageMetric,
  UsageUnit,
} from "./model.ts";

/** Freshness bound for gatherer precedence: two missed ticks. */
export const QUOTA_CURRENT_FRESH_MS = 20 * 60_000;

/**
 * Future clock-skew tolerance for a gatherer entry's measuredAt (#1541):
 * NTP-disciplined hosts skew in seconds, so a read stamped at most this far
 * in the future is indistinguishable from skew and stays fresh. Beyond it
 * the entry is not fresh at all - it classifies stale, so one skewed write
 * cannot pin a pool's windows (or outrank genuinely fresh reads) for as
 * long as the skew lasts.
 */
export const QUOTA_CURRENT_FUTURE_SKEW_MS = 60_000;

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

/** Structural check of a parsed quota-current.json document. */
export function parseQuotaCurrent(doc: unknown): QuotaCurrentFile | null {
  if (!isRecord(doc)) return null;
  if (doc.version !== 1 || typeof doc.generatedAt !== "string") return null;
  if (!Array.isArray(doc.providers)) return null;
  for (const p of doc.providers) {
    if (!isRecord(p)) return null;
    if (typeof p.providerId !== "string" || typeof p.serviceId !== "string") return null;
    if (p.state !== "ok" && p.state !== "error" && p.state !== "unconfigured") return null;
    if (typeof p.measuredAt !== "string" || typeof p.source !== "string") return null;
    if (!Array.isArray(p.windows)) return null;
  }
  return doc as unknown as QuotaCurrentFile;
}

type FreshState = PoolFreshness["state"];

function entryState(entry: QuotaCurrentEntry, nowMs: number): FreshState {
  if (entry.state === "ok") {
    const t = Date.parse(entry.measuredAt);
    return Number.isFinite(t) && t - nowMs <= QUOTA_CURRENT_FUTURE_SKEW_MS && nowMs - t <= QUOTA_CURRENT_FRESH_MS
      ? "ok"
      : "stale";
  }
  return "error";
}

function errorText(entry: QuotaCurrentEntry): string {
  if (entry.error) return entry.error;
  return entry.state === "unconfigured" ? "not configured on the gatherer host" : "gatherer could not read this pool";
}

/** Gatherer window names onto the pool card's naming. */
function windowName(name: string): string {
  return name === "5h" ? "5-hour" : name;
}

function pctWindows(entry: QuotaCurrentEntry): QuotaWindow[] {
  return entry.windows
    .filter((w) => typeof w.pctOfLimit === "number" && Number.isFinite(w.pctOfLimit))
    .map((w) => ({
      name: windowName(w.name),
      unit: "other" as const,
      limit: 100,
      used: w.pctOfLimit as number,
      percent: w.pctOfLimit as number,
      resetsAt: w.resetsAt ?? null,
    }));
}

function applyEntry(pool: QuotaPool, entry: QuotaCurrentEntry, state: FreshState): boolean {
  // Returns true when the pool's windows were replaced by a fresh read.
  const freshness: PoolFreshness = {
    measuredAt: entry.measuredAt,
    source: entry.source,
    state,
  };
  if (state === "error") freshness.error = errorText(entry);
  pool.freshness = freshness;
  if (state === "error") {
    pool.state = "error";
    return false;
  }
  if (state === "stale") {
    pool.state = "stale";
    return false;
  }
  const windows = pctWindows(entry);
  if (windows.length === 0) return false; // balance-only read: freshness, no windows to show
  pool.windows = windows;
  pool.state = poolState(pool);
  delete pool.wallLiftsAt;
  return true;
}

function summarize(file: QuotaCurrentFile, nowMs: number): GathererSummary {
  return {
    generatedAt: file.generatedAt,
    source: file.source,
    tickAt: file.heartbeat?.tickAt ?? null,
    ok: file.heartbeat?.ok ?? true,
    providers: file.providers.map((e) => {
      const s: GathererSummary["providers"][number] = {
        providerId: e.providerId,
        serviceId: e.serviceId,
        state: entryState(e, nowMs),
        measuredAt: e.measuredAt,
        source: e.source,
      };
      if (e.credentialLabel) s.credentialLabel = e.credentialLabel;
      if (s.state === "error") s.error = errorText(e);
      return s;
    }),
  };
}

/**
 * Overlay a parsed quota-current document onto a quota view. Pure:
 * the input view is not mutated; pools are copied when changed.
 */
export function overlayQuotaCurrent(view: QuotaView, file: QuotaCurrentFile, now = new Date()): QuotaView {
  const nowMs = now.getTime();
  const byService = new Map<string, QuotaCurrentEntry[]>();
  for (const e of file.providers) {
    const list = byService.get(e.serviceId);
    if (list) list.push(e);
    else byService.set(e.serviceId, [e]);
  }

  let anyFreshWindows = false;
  const pools = view.pools.map((orig) => {
    const entries = byService.get(orig.id);
    if (!entries || entries.length === 0) return orig;
    const pool: QuotaPool = { ...orig, windows: [...orig.windows] };
    const freshOk = entries
      .filter((e) => entryState(e, nowMs) === "ok")
      .sort((a, b) => Date.parse(b.measuredAt) - Date.parse(a.measuredAt));
    if (freshOk.length > 0) {
      if (applyEntry(pool, freshOk[0], "ok")) anyFreshWindows = true;
      return pool;
    }
    const newest = [...entries].sort((a, b) => Date.parse(b.measuredAt) - Date.parse(a.measuredAt))[0];
    applyEntry(pool, newest, entryState(newest, nowMs));
    return pool;
  });

  const out: QuotaView = { ...view, pools, gatherer: summarize(file, nowMs) };
  if (anyFreshWindows) out.choices = computeChoices(pools, nowMs);
  return out;
}

/** Turso window name -> SaaS card metric shape (labels mirror saas-extract). */
const TURSO_WINDOW_META: Record<string, { label: string; unit: UsageUnit; limit: number }> = {
  databases: { label: "Databases", unit: "count", limit: 100 },
  storage: { label: "Storage", unit: "gb", limit: 5 },
  "rows-read": { label: "Rows read / month", unit: "rows", limit: 500_000_000 },
  "rows-written": { label: "Rows written / month", unit: "rows", limit: 10_000_000 },
  sync: { label: "Sync / month", unit: "gb", limit: 3 },
};

/**
 * Apply the gatherer's Turso entry (#1049) to the SaaS services —
 * the usage() counterpart of overlayQuotaCurrent, with the same
 * semantics: a fresh ok read replaces the Turso card's metrics,
 * measuredAt, and cycle end; an error or stale read leaves the
 * card's dated values standing and adds a dated conflict flag, so
 * the failure is loud without faking a fresh zero. Services without
 * a Turso card, and files without a Turso entry, pass through
 * unchanged (same array reference).
 */
export function overlayTursoUsage(services: SaasService[], file: QuotaCurrentFile, now = new Date()): SaasService[] {
  const entries = file.providers.filter((e) => e.serviceId === "turso");
  if (entries.length === 0) return services;
  const idx = services.findIndex((s) => s.id === "turso");
  if (idx < 0) return services;
  const nowMs = now.getTime();
  const svc = services[idx];
  const freshOk = entries
    .filter((e) => entryState(e, nowMs) === "ok")
    .sort((a, b) => Date.parse(b.measuredAt) - Date.parse(a.measuredAt));
  const out = [...services];
  if (freshOk.length > 0) {
    const entry = freshOk[0];
    const prev = new Map(svc.metrics.map((m) => [m.id, m]));
    const metrics: UsageMetric[] = [];
    for (const w of entry.windows) {
      const meta = TURSO_WINDOW_META[w.name];
      if (!meta || w.value == null) continue;
      metrics.push({
        id: w.name,
        label: prev.get(w.name)?.label ?? meta.label,
        used: w.value,
        limit: meta.limit,
        unit: meta.unit,
        ...(w.pctOfLimit != null ? { percent: w.pctOfLimit } : {}),
        highlight: prev.get(w.name)?.highlight ?? true,
      });
    }
    if (metrics.length === 0) return services;
    const resetsAt = entry.windows.find((w) => w.resetsAt)?.resetsAt ?? null;
    out[idx] = {
      ...svc,
      metrics,
      measuredAt: entry.measuredAt,
      sourceClass: "api",
      sourceNote: `${svc.sourceNote} · live gatherer read (turso-api) at ${entry.measuredAt}`,
      ...(resetsAt ? { nextDate: `cycle ends ${resetsAt.slice(0, 10)}` } : {}),
    };
    return out;
  }
  const newest = [...entries].sort((a, b) => Date.parse(b.measuredAt) - Date.parse(a.measuredAt))[0];
  const flag = `quota gatherer turso read ${entryState(newest, nowMs)} at ${newest.measuredAt}: ${errorText(newest)} — card keeps its last dated read`;
  if (svc.conflicts.includes(flag)) return services;
  out[idx] = { ...svc, conflicts: [...svc.conflicts, flag] };
  return out;
}
