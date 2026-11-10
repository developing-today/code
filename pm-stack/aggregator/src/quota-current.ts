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
import { isProviderAuthAlert, reevaluateProviderAuth } from "./provider-auth-alerts.ts";
import { lifecycleForEntry, safeProviderReason } from "./credential-lifecycle.ts";
import { computeChoices } from "./quota-guidance.ts";
import type {
  GathererSummary,
  PoolFreshness,
  QuotaCurrentEntry,
  QuotaCredentialRow,
  QuotaCurrentFile,
  QuotaPool,
  QuotaView,
  QuotaWindow,
  SaasService,
  UsageMetric,
  UsageUnit,
} from "./model.ts";
import { arrayOf, isFiniteNumber, isIsoDate, isNonEmptyString, isRecord } from "./validate.ts";

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

function isQuotaCurrentWindow(value: unknown): value is QuotaCurrentFile["providers"][number]["windows"][number] {
  if (!isRecord(value)) return false;
  const finiteOrNull = (v: unknown) => v === null || isFiniteNumber(v);
  return (
    isNonEmptyString(value.name) &&
    finiteOrNull(value.value) &&
    finiteOrNull(value.pctOfLimit) &&
    (value.valueLabel === null || typeof value.valueLabel === "string") &&
    finiteOrNull(value.windowSeconds) &&
    (value.resetsAt === null || isIsoDate(value.resetsAt)) &&
    isIsoDate(value.measuredAt) &&
    typeof value.source === "string" &&
    (value.state === "ok" || value.state === "error")
  );
}

function isQuotaCurrentEntry(value: unknown): value is QuotaCurrentEntry {
  return (
    isRecord(value) &&
    typeof value.providerId === "string" &&
    typeof value.providerName === "string" &&
    typeof value.serviceId === "string" &&
    (value.state === "ok" || value.state === "error" || value.state === "unconfigured") &&
    isIsoDate(value.measuredAt) &&
    typeof value.source === "string" &&
    (value.credentialWatch === undefined || typeof value.credentialWatch === "boolean") &&
    (value.error === undefined || typeof value.error === "string") &&
    (value.credentialExpiresAt === undefined ||
      (typeof value.credentialExpiresAt === "string" && isIsoDate(value.credentialExpiresAt))) &&
    (value.credentialSource === undefined ||
      value.credentialSource === "claude-code" ||
      value.credentialSource === "opencode" ||
      value.credentialSource === "environment") &&
    Array.isArray(value.windows)
  );
}

/** Structural check of a parsed quota-current.json document. */
export function parseQuotaCurrent(doc: unknown): QuotaCurrentFile | null {
  if (!isRecord(doc)) return null;
  if (doc.version !== 1 || typeof doc.generatedAt !== "string") return null;
  const providers = arrayOf(isQuotaCurrentEntry)(doc.providers);
  // Keep rejecting a wholly malformed provider list; when at least one entry is valid,
  // isolate malformed siblings so one provider cannot suppress unrelated overlays.
  if (!providers || (providers.values.length === 0 && providers.dropped > 0)) return null;
  let droppedWindows = 0;
  const validProviders = providers.values.map((provider) => {
    const windows = arrayOf(isQuotaCurrentWindow)(provider.windows)!;
    droppedWindows += windows.dropped;
    if (provider.state === "ok" && provider.windows.length > 0 && windows.values.length === 0) {
      // Entirely discarded read (#2416, Codex finding on PR #4182): left
      // as "ok", the entry would still win fresh-ok precedence in the
      // overlay, stamp the pool's older windows with fresh provenance,
      // and block a valid fresh sibling for the same service from
      // supplying the headline. Downgrade it to the error it is. An
      // entry that arrived with no windows at all is a legitimate
      // balance-only read and keeps its state.
      return {
        ...provider,
        state: "error" as const,
        error: "all quota windows were malformed and dropped at ingest",
        windows: windows.values,
      };
    }
    return {
      ...provider,
      ...(typeof provider.error === "string" ? { error: safeProviderReason(provider.error) } : {}),
      windows: windows.values,
    };
  });
  // #6671: the optional providerAuth section is isolated at this
  // boundary like the provider list — a non-array value is dropped
  // and malformed rows are filtered, so one bad section can never
  // throw in the overlay and suppress the unrelated quota data.
  const providerAuth = Array.isArray(doc.providerAuth)
    ? doc.providerAuth.filter(isProviderAuthAlert)
    : undefined;
  return {
    ...doc,
    providers: validProviders,
    providerAuth,
    sanitized: { droppedProviders: providers.dropped, droppedWindows },
  } as unknown as QuotaCurrentFile;
}

type FreshState = PoolFreshness["state"];

function entryState(entry: QuotaCurrentEntry, nowMs: number): FreshState {
  if (entry.state === "ok") {
    // #1268: retained last-known entries are stale by definition —
    // the provider's latest run failed, however recent the retained
    // measuredAt is (one tick is 15 min, inside the 20-min bound).
    if (entry.retainedFromFailedTick) return "stale";
    const t = Date.parse(entry.measuredAt);
    return Number.isFinite(t) && t - nowMs <= QUOTA_CURRENT_FUTURE_SKEW_MS && nowMs - t <= QUOTA_CURRENT_FRESH_MS
      ? "ok"
      : "stale";
  }
  return "error";
}

/**
 * Newest-pick ordering for gatherer entries (#2253): one shared
 * comparator for every "newest entry wins" sort in this module. The
 * sort key is the entry's parsed measuredAt, or -Infinity when the
 * stamp does not parse to a finite time or sits beyond the
 * future-skew bound — entryState already refuses such stamps the
 * "ok" classification, but the stale-path pick must refuse them the
 * win too, or a garbage or skewed write becomes the pool's displayed
 * freshness (stamp + source) in place of the newest real read (the
 * stale-path residual of the #1541 gate). Entries equal under the
 * key (both unusable, or the same instant) keep their file order —
 * Array.prototype.sort is stable — so all-finite inputs behave
 * exactly as the raw Date.parse difference did.
 */
function newestFirst(nowMs: number): (a: QuotaCurrentEntry, b: QuotaCurrentEntry) => number {
  const key = (e: QuotaCurrentEntry): number => {
    const t = Date.parse(e.measuredAt);
    return Number.isFinite(t) && t - nowMs <= QUOTA_CURRENT_FUTURE_SKEW_MS ? t : -Infinity;
  };
  return (a, b) => {
    const ka = key(a);
    const kb = key(b);
    return ka === kb ? 0 : kb - ka;
  };
}

function errorText(entry: QuotaCurrentEntry): string {
  if (entry.error) return entry.error;
  return entry.state === "unconfigured" ? "not configured on the gatherer host" : "gatherer could not read this pool";
}

/** Gatherer window names onto the pool card's naming. */
function windowName(name: string): string {
  return name === "5h" ? "5-hour" : name;
}

/**
 * A pctOfLimit is usable only as a finite number inside [0, 100] (#2582,
 * gate pinned by triage task 3837). Out-of-range reads DROP, never clamp:
 * clamping 150 -> 100 would still wall a healthy pool, and publishing -20
 * would report negative usage. Shared by both reader sites below so the
 * two can never drift apart.
 */
function usablePct(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v) && v >= 0 && v <= 100;
}

function pctWindows(entry: QuotaCurrentEntry): QuotaWindow[] {
  return entry.windows
    .filter((w) => usablePct(w.pctOfLimit))
    .map((w) => ({
      name: windowName(w.name),
      unit: "other" as const,
      limit: 100,
      used: w.pctOfLimit as number,
      percent: w.pctOfLimit as number,
      resetsAt: w.resetsAt ?? null,
    }));
}

/**
 * The gatherer leg error for an entry's provider, from the tick's
 * heartbeat (#6942): a retained entry is preserved verbatim with no
 * error field (markRetained in quota-gather-map.ts), so when the leg
 * that produced it errored, the heartbeat's perProvider record —
 * keyed by run id, matched here on the entry's providerId with its
 * serviceId as fallback — is the only place the error text survives.
 */
function legErrorFor(file: QuotaCurrentFile, entry: QuotaCurrentEntry): string | undefined {
  const perProvider = file.heartbeat?.perProvider;
  if (!perProvider) return undefined;
  const leg = perProvider[entry.providerId] ?? perProvider[entry.serviceId];
  return leg && leg.state === "error" && leg.error ? safeProviderReason(leg.error) : undefined;
}

/**
 * #6942: a pool with no windows of its own (a shell pool — its dated
 * windows exist only in the read entry) takes the entry's dated
 * windows on the stale/error path, percentages identical to the
 * read. A pool carrying its own dated windows is untouched: those
 * win over the gatherer entry's, as pinned for spec pools. The
 * shell pool's "no read" note is written against a read that never
 * landed; when the entry does carry a dated read, the note
 * contradicts the read and is suppressed.
 */
function adoptEntryWindows(pool: QuotaPool, entry: QuotaCurrentEntry): void {
  if (pool.windows.length > 0) return;
  const windows = pctWindows(entry);
  if (windows.length === 0) return;
  pool.windows = windows;
  if (pool.note && /no (dated|current) read/i.test(pool.note)) delete pool.note;
}

/**
 * #7033: are the pool's own windows all dead relative to a read —
 * every window carries a reset at or before the read's measuredAt?
 * Such windows describe a period that had already ended when the
 * read was taken, so they cannot be the pool's standing dated read.
 * A window with no reset (or an unparseable one) is never dead: the
 * dated read it belongs to may still be the pool's standing values.
 */
function baseWindowsExpired(pool: QuotaPool, entry: QuotaCurrentEntry): boolean {
  if (pool.windows.length === 0) return false;
  const readAt = Date.parse(entry.measuredAt);
  if (!Number.isFinite(readAt)) return false;
  return pool.windows.every((w) => {
    if (!w.resetsAt) return false;
    const reset = Date.parse(w.resetsAt);
    return Number.isFinite(reset) && reset <= readAt;
  });
}

function applyEntry(pool: QuotaPool, entry: QuotaCurrentEntry, state: FreshState, legError?: string): boolean {
  // Returns true when the pool's windows were replaced by a fresh read.
  const freshness: PoolFreshness = {
    measuredAt: entry.measuredAt,
    source: entry.source,
    state,
  };
  if (state === "error") freshness.error = !entry.error && legError ? legError : errorText(entry);
  pool.freshness = freshness;
  if (state === "error") {
    pool.state = "error";
    adoptEntryWindows(pool, entry);
    return false;
  }
  if (state === "stale") {
    // #6942: a retained entry whose gatherer leg errored is that
    // read error surfacing, not mere staleness — but only a pool
    // with no dated windows of its own changes rendering for it.
    // Spec pools keep the #1268 stale classification and their
    // base dated windows exactly as pinned.
    if (legError && entry.retainedFromFailedTick && pool.windows.length === 0) {
      freshness.state = "error";
      freshness.error = legError;
      pool.freshness = freshness;
      pool.state = "error";
      adoptEntryWindows(pool, entry);
      return false;
    }
    // #7033: the same leg-error surfacing applies to a pool whose
    // base windows are all dead — every window's reset is at or
    // before the retained read's measuredAt, so the base row
    // describes a period that had already ended when the read was
    // taken (claude's quota-view weekly row, reset 2026-10-28,
    // against the read's dated fallback @2026-11-07). That row is
    // not the pool's dated read standing; it is an older
    // transcription substituting for the read. The read's dated
    // fallback windows stand, percentages identical, and the note
    // written against the substituted row is dropped. A pool with
    // any live or undated base window keeps the #1268 pin above.
    if (legError && entry.retainedFromFailedTick && baseWindowsExpired(pool, entry)) {
      freshness.state = "error";
      freshness.error = legError;
      pool.freshness = freshness;
      pool.state = "error";
      const windows = pctWindows(entry);
      if (windows.length > 0) {
        pool.windows = windows;
        delete pool.note;
      }
      return false;
    }
    pool.state = "stale";
    adoptEntryWindows(pool, entry);
    return false;
  }
  const windows = pctWindows(entry);
  if (windows.length === 0) return false; // balance-only read: freshness, no windows to show
  pool.windows = windows;
  pool.state = poolState(pool);
  delete pool.wallLiftsAt;
  // Pass-through (Drew, 2026-11-03): the displayed numbers are now the
  // live read's, so the base pool's note — dated steward prose written
  // against older numbers — must not ride along and outrank the read.
  // The read carries no prose note of its own; its planLabel shows as
  // the pool's plan and its freshness stamps the measurement. A pool
  // whose read did NOT land (error/stale/balance-only) keeps its dated
  // note, which still describes the dated values on display.
  delete pool.note;
  return true;
}

/**
 * Reset instants closer than this are the same window: some producers
 * recompute resetsAt from a duration on every fetch, so one shared
 * read's stamp drifts sub-second between credentials (and by up to
 * the gap between their fetches in one tick). Genuinely different
 * windows reset minutes-to-days apart (#2978 Codex finding).
 */
const RESET_DRIFT_TOLERANCE_MS = 60_000;

function sameReset(a: string | null, b: string | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  const ta = Date.parse(a);
  const tb = Date.parse(b);
  return Number.isFinite(ta) && Number.isFinite(tb) && Math.abs(ta - tb) <= RESET_DRIFT_TOLERANCE_MS;
}

function sameQuotaRead(a: QuotaCredentialRow, b: QuotaCredentialRow): boolean {
  return (
    a.freshness.state === b.freshness.state &&
    a.windows.length === b.windows.length &&
    a.windows.every(
      (w, i) =>
        w.name === b.windows[i].name &&
        w.percent === b.windows[i].percent &&
        sameReset(w.resetsAt, b.windows[i].resetsAt),
    )
  );
}

/**
 * Per-credential rows for one pool's entries (#1113): the newest
 * entry per credential identity — the stable credentialId when the
 * entry carries one, else credentialLabel, else providerId. Keying
 * on the label alone would collapse two distinct credentials that
 * share (or lack) a display label (#2978 Codex finding). Rows are
 * returned only when they genuinely differ — in state or in percent
 * windows, with reset-drift tolerance — so two logins sharing one
 * pool read stay a single headline row, while two logins with
 * distinct quotas (or one read failing while the other succeeds)
 * each get a row.
 */
function credentialRows(entries: QuotaCurrentEntry[], nowMs: number): QuotaCredentialRow[] | undefined {
  const newest = [...entries].sort(newestFirst(nowMs));
  const byCred = new Map<string, QuotaCurrentEntry>();
  for (const e of newest) {
    const key = e.credentialId ?? e.credentialLabel ?? e.providerId;
    if (!byCred.has(key)) byCred.set(key, e);
  }
  if (byCred.size < 2) return undefined;
  const rows: QuotaCredentialRow[] = [...byCred.values()].map((e) => {
    const state = entryState(e, nowMs);
    const freshness: PoolFreshness = { measuredAt: e.measuredAt, source: e.source, state };
    if (state === "error") freshness.error = errorText(e);
    return {
      label: e.credentialLabel ?? e.providerName ?? e.providerId,
      ...(e.credentialId ? { credentialId: e.credentialId } : {}),
      providerId: e.providerId,
      windows: state === "ok" ? pctWindows(e) : [],
      freshness,
    };
  });
  if (rows.every((r) => sameQuotaRead(r, rows[0]))) return undefined;
  return rows;
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
 * Gatherer serviceIds that never become quota pools of their own:
 * "turso" is a SaaS usage row (its display surface is the SaaS cards,
 * via overlayTursoUsage), and "antigravity" / "opencode-go" are the
 * COMBINED account rows whose pool-level rendering is their split
 * rows (_antigravity.gemini / _antigravity.claude-gpt — pre-rename:
 * antigravity-direct.gemini / antigravity-direct.claude-gpt — and
 * opencode-go.drewrypope / opencode-go.popesdrew, produced by
 * entriesWithPoolSplits in quota-gather-map.ts). Synthesizing pools
 * for them would double-render the same read. Every OTHER serviceId
 * the read reports renders as a pool — see the synthesis below.
 */
const NON_POOL_SERVICE_IDS: ReadonlySet<string> = new Set(["turso", "antigravity", "_antigravity", "opencode-go"]);

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
    const freshOk = entries.filter((e) => entryState(e, nowMs) === "ok").sort(newestFirst(nowMs));
    if (freshOk.length > 0) {
      if (applyEntry(pool, freshOk[0], "ok")) anyFreshWindows = true;
      const rows = credentialRows(entries, nowMs);
      if (rows) pool.credentials = rows;
      return pool;
    }
    const newest = [...entries].sort(newestFirst(nowMs))[0];
    applyEntry(pool, newest, entryState(newest, nowMs), legErrorFor(file, newest));
    return pool;
  });

  // Unmapped-pool pass-through (Drew, 2026-11-03 — "the dashboard
  // should not be doing any of that"): a gatherer entry whose
  // serviceId matches no pool in the base view used to be silently
  // dropped here — that key mismatch is how the OpenDesign read
  // vanished in the morning of 2026-11-03, and the commandcode read
  // (serviceId "command-code" until quota-gather-map learned the
  // config name) never landed at all. The board is a straight display
  // of the OpenCode quota read: any reported pool the base view does
  // not know renders under the id the read reported, with the read's
  // own provider/plan labels and freshness. Only the documented
  // non-pool rows (NON_POOL_SERVICE_IDS) are exempt.
  const knownIds = new Set(pools.map((p) => p.id));
  const synthesized: QuotaPool[] = [];
  for (const serviceId of [...byService.keys()].sort()) {
    if (knownIds.has(serviceId) || NON_POOL_SERVICE_IDS.has(serviceId)) continue;
    const entries = byService.get(serviceId)!;
    const newest = [...entries].sort(newestFirst(nowMs))[0];
    const pool: QuotaPool = {
      id: serviceId,
      provider: newest.providerName,
      plan: newest.planLabel ?? "",
      state: "healthy",
      windows: [],
    };
    const freshOk = entries.filter((e) => entryState(e, nowMs) === "ok").sort(newestFirst(nowMs));
    if (freshOk.length > 0) {
      if (applyEntry(pool, freshOk[0], "ok")) anyFreshWindows = true;
      const rows = credentialRows(entries, nowMs);
      if (rows) pool.credentials = rows;
    } else {
      applyEntry(pool, newest, entryState(newest, nowMs), legErrorFor(file, newest));
    }
    synthesized.push(pool);
  }
  const allPools = [...pools, ...synthesized];

  const out: QuotaView = { ...view, pools: allPools, gatherer: summarize(file, nowMs) };
  // #6671: keeper alerts ride the same overlay; malformed rows are
  // dropped at this boundary like every other gatherer field.
  if (Array.isArray(file.providerAuth))
    out.providerAuth = reevaluateProviderAuth(file.providerAuth.filter(isProviderAuthAlert), now);
  const credentialLifecycle = file.providers
    .filter((entry) => entry.credentialWatch === true)
    .map((entry) => lifecycleForEntry(entry, now));
  if (credentialLifecycle.length) out.credentialLifecycle = credentialLifecycle;
  if (anyFreshWindows) out.choices = computeChoices(allPools, nowMs);
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
 * measuredAt, and cycle end — except metrics the fresh read does
 * not carry, which stay as stale-marked dated values (#6227); an
 * error or stale read leaves the
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
  const freshOk = entries.filter((e) => entryState(e, nowMs) === "ok").sort(newestFirst(nowMs));
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
        ...(usablePct(w.pctOfLimit) ? { percent: w.pctOfLimit } : {}),
        highlight: prev.get(w.name)?.highlight ?? true,
      });
    }
    if (metrics.length === 0) return services;
    // A partial fresh read must not silently erase the card's last
    // dated values (#6227): metrics the fresh entry does not carry
    // stay on the card, marked stale with the stamp they were read
    // at, instead of vanishing until a complete read returns. They
    // are evidence of the last measured state, never of recovery.
    const freshIds = new Set(metrics.map((m) => m.id));
    for (const m of svc.metrics) {
      if (freshIds.has(m.id)) continue;
      metrics.push({
        ...m,
        note: `stale: last measured ${svc.measuredAt}; absent from the fresh gatherer read at ${entry.measuredAt}`,
      });
    }
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
  const newest = [...entries].sort(newestFirst(nowMs))[0];
  const flag = `quota gatherer turso read ${entryState(newest, nowMs)} at ${newest.measuredAt}: ${errorText(newest)} — card keeps its last dated read`;
  if (svc.conflicts.includes(flag)) return services;
  out[idx] = { ...svc, conflicts: [...svc.conflicts, flag] };
  return out;
}
