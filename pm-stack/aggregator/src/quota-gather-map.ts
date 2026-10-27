// Mapping from OpenChamber quota-module results onto the hub's
// quota-current contract + §4 snapshot records (Stage 2, #1151).
//
// The quota module (devtoday-openchamber, server/lib/quota) answers
// one result per provider credential:
//   { providerId, providerName, ok, configured, usage, fetchedAt,
//     error?, planLabel?, credentialId?, credentialLabel? }
// with usage.windows a record of window name ->
//   { usedPercent, remainingPercent, windowSeconds, resetAt,
//     resetAfterSeconds, valueLabel? }
// where resetAt is epoch milliseconds. Everything here is a pure
// function of that shape, so a provider adding windows needs no
// gatherer change; a provider whose usage is not window-shaped keeps
// its raw usage on the entry and yields no windows (never zeros).
import type {
  QuotaCurrentEntry,
  QuotaCurrentWindow,
  SnapshotRecord,
} from "./model.ts";

/** The subset of a quota-module result the mapper reads. */
export interface QuotaProviderResult {
  providerId: string;
  providerName?: string;
  ok: boolean;
  configured: boolean;
  usage?: unknown;
  fetchedAt?: number;
  error?: string;
  planLabel?: string;
  credentialId?: string;
  credentialLabel?: string;
}

interface RawWindow {
  usedPercent?: unknown;
  windowSeconds?: unknown;
  resetAt?: unknown;
  valueLabel?: unknown;
}

const num = (v: unknown): number | null =>
  typeof v === "number" && Number.isFinite(v) ? v : null;

/** Provider reset stamps arrive as epoch ms (or s); normalize to ISO. */
export function normalizeResetAt(v: unknown): string | null {
  if (typeof v === "number" && Number.isFinite(v) && v > 0) {
    const ms = v < 1e12 ? v * 1000 : v;
    const d = new Date(ms);
    return Number.isNaN(d.getTime()) ? null : d.toISOString();
  }
  if (typeof v === "string" && v !== "") {
    const t = Date.parse(v);
    return Number.isNaN(t) ? null : new Date(t).toISOString();
  }
  return null;
}

/**
 * Hub pool/snapshot service id for a provider result. Mirrors
 * POOL_SPECS (src/quota-extract.ts) where a pool exists, so gatherer
 * records continue the series the dated-read writers started.
 * OpenCode Go answers one combined row today (the module has no
 * per-credential fetch for it), so it lands under the bare id until
 * the module splits workspaces.
 */
export function serviceIdFor(providerId: string): string {
  switch (providerId) {
    case "claude":
      return "anthropic:claude-pro";
    case "codex":
      return "openai:codex";
    case "cline-pass":
      return "cline-pass:cline-pass";
    case "deepseek":
      return "deepseek:api";
    default:
      return providerId;
  }
}

export const GATHERER_SOURCE = "openchamber-quota";

/**
 * The file-export convention (Meta Phase 1, #1301 — the pattern
 * other laptop exports follow): a producer on the laptop writes
 * <name>-quota-export/latest.json atomically; success documents
 * carry measuredAt + windows, failure documents are separate status
 * documents with measuredAt null. A latest.json older than the
 * staleness bound means the producer has stopped — the service is
 * unreachable, not at zero.
 */
export interface FileExportDoc {
  service?: string;
  source?: string;
  status?: string;
  measuredAt?: string | null;
  tier?: string;
  windows?: { name?: string; usedPct?: number | null; resetAt?: string | number | null }[];
  freeCreditsUsd?: number | null;
  paygBalanceUsd?: number | null;
}

const FILE_EXPORT_WINDOW_SECONDS: Record<string, number> = { "5h": 18_000, weekly: 604_800 };

/**
 * Map one file-export document onto a quota-current entry (v1
 * producer: Meta, service meta:high-usage, ~/atlas/meta-quota-export/
 * latest.json on the laptop). `doc` null = the file could not be
 * read or parsed; `stale` = its mtime is past the staleness bound.
 * Both are error entries — never zeros.
 */
export function entryFromFileExport(
  doc: FileExportDoc | null,
  opts: { fileMtimeIso: string | null; stale: boolean },
): QuotaCurrentEntry {
  const entry: QuotaCurrentEntry = {
    providerId: "meta",
    providerName: "Meta",
    serviceId: "meta:high-usage",
    state: "error",
    measuredAt: opts.fileMtimeIso ?? new Date(0).toISOString(),
    source: doc?.source ?? "file-export",
    windows: [],
  };
  if (doc?.tier) entry.planLabel = doc.tier;
  if (!doc) {
    entry.error = "file-export unreadable (missing or unparseable latest.json)";
    return entry;
  }
  if (opts.stale) {
    entry.error = "file-export stale (latest.json older than the staleness bound — producer unreachable)";
    return entry;
  }
  const measuredMs = doc.measuredAt ? Date.parse(doc.measuredAt) : Number.NaN;
  if (doc.status !== "ok" || Number.isNaN(measuredMs)) {
    entry.error = `file-export status: ${doc.status ?? "unknown"}`;
    return entry;
  }
  const measuredAt = new Date(measuredMs).toISOString();
  entry.state = "ok";
  entry.measuredAt = measuredAt;
  for (const w of doc.windows ?? []) {
    const pct = num(w.usedPct);
    if (!w.name || pct == null) continue;
    entry.windows.push({
      name: w.name,
      value: pct,
      pctOfLimit: pct,
      valueLabel: null,
      windowSeconds: FILE_EXPORT_WINDOW_SECONDS[w.name] ?? null,
      resetsAt: normalizeResetAt(w.resetAt),
      measuredAt,
      source: entry.source,
      state: "ok",
    });
  }
  const labelWindow = (name: string, usd: number | null | undefined): void => {
    if (typeof usd !== "number" || !Number.isFinite(usd)) return;
    entry.windows.push({
      name,
      value: null,
      pctOfLimit: null,
      valueLabel: `$${usd.toFixed(2)}`,
      windowSeconds: null,
      resetsAt: null,
      measuredAt,
      source: entry.source,
      state: "ok",
    });
  };
  labelWindow("free_credits", doc.freeCreditsUsd);
  labelWindow("payg_balance", doc.paygBalanceUsd);
  return entry;
}

/** Map one provider result onto a quota-current entry. */
export function entryFromResult(result: QuotaProviderResult, tickIso: string): QuotaCurrentEntry {
  const measuredAt =
    typeof result.fetchedAt === "number" && Number.isFinite(result.fetchedAt)
      ? new Date(result.fetchedAt).toISOString()
      : tickIso;
  const state: QuotaCurrentEntry["state"] = result.ok ? "ok" : result.configured ? "error" : "unconfigured";
  const entry: QuotaCurrentEntry = {
    providerId: result.providerId,
    providerName: result.providerName ?? result.providerId,
    serviceId: serviceIdFor(result.providerId),
    state,
    measuredAt,
    source: GATHERER_SOURCE,
    windows: [],
  };
  if (result.credentialId) entry.credentialId = result.credentialId;
  if (result.credentialLabel) entry.credentialLabel = result.credentialLabel;
  if (result.planLabel) entry.planLabel = result.planLabel;
  if (result.error) entry.error = result.error;
  if (result.usage != null) entry.usage = result.usage;

  const pushWindow = (name: string, raw: RawWindow | null | undefined): void => {
    const pct = num(raw?.usedPercent);
    entry.windows.push({
      name,
      value: pct,
      pctOfLimit: pct,
      valueLabel: typeof raw?.valueLabel === "string" ? raw.valueLabel : null,
      windowSeconds: num(raw?.windowSeconds),
      resetsAt: normalizeResetAt(raw?.resetAt),
      measuredAt,
      source: GATHERER_SOURCE,
      state: result.ok ? "ok" : "error",
    });
  };
  const usageObj = result.usage as { windows?: unknown; models?: unknown } | null | undefined;
  if (typeof usageObj?.windows === "object" && usageObj.windows !== null) {
    for (const [name, raw] of Object.entries(usageObj.windows as Record<string, RawWindow>)) {
      pushWindow(name, raw);
    }
  }
  // Per-model usage (Claude's model-scoped windows, Antigravity's
  // whole shape): flatten to "<model>:<window>" so the data lands in
  // the same contract. Pool attribution across models is the hub's
  // modeling step, not the gatherer's.
  if (typeof usageObj?.models === "object" && usageObj.models !== null) {
    for (const [model, mu] of Object.entries(usageObj.models as Record<string, { windows?: unknown }>)) {
      if (typeof mu?.windows !== "object" || mu.windows === null) continue;
      for (const [name, raw] of Object.entries(mu.windows as Record<string, RawWindow>)) {
        pushWindow(`${model}:${name}`, raw);
      }
    }
  }
  return entry;
}

/**
 * Snapshot records for one tick: a pct record per window whose VALUE
 * changed since the gatherer last appended it (the state map is
 * serviceId|metricId -> value), so the store gains a point when a
 * reading moves, not every 15 minutes. The stamp is the value alone:
 * several providers recompute the reset instant from a duration on
 * every fetch (sub-second drift, or "now + window" for idle 5h
 * windows), so keying on resetsAt would append noise points for
 * readings that did not move. A window roll still shows up — as the
 * value change it causes. The record itself carries the current
 * resetsAt. Label-only windows (balances) yield no record in v1 —
 * the CNY balance case is #1052's unit work; quota-current still
 * carries the label.
 */
export function recordsFromEntries(
  entries: QuotaCurrentEntry[],
  lastAppended: ReadonlyMap<string, string>,
  observedBy: string,
): { records: SnapshotRecord[]; appended: Map<string, string> } {
  const records: SnapshotRecord[] = [];
  const appended = new Map<string, string>();
  for (const entry of entries) {
    if (entry.state !== "ok") continue;
    for (const w of entry.windows) {
      if (w.pctOfLimit == null) continue;
      const key = `${entry.serviceId}|${w.name}`;
      const stamp = `${w.pctOfLimit}`;
      if (lastAppended.get(key) === stamp) continue;
      appended.set(key, stamp);
      records.push({
        ts: w.measuredAt,
        serviceId: entry.serviceId,
        metricId: w.name,
        value: w.pctOfLimit,
        unit: "pct",
        limit: 100,
        ...(w.resetsAt ? { resetsAt: w.resetsAt } : {}),
        // File exports are console reads by another route; module
        // providers are API reads.
        source: entry.source.startsWith("file-export") ? "console" : "api",
        observedBy,
      });
    }
  }
  return { records, appended };
}
