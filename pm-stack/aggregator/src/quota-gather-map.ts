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
import type { QuotaCurrentEntry, QuotaCurrentWindow, SnapshotRecord } from "./model.ts";
import { safeProviderReason } from "./credential-lifecycle.ts";
import {
  isFiniteNumber,
  isIsoDate,
  isNonEmptyString,
  isNonNegative,
  isPercentOf,
  isRecord,
  parseIsoDate,
} from "./validate.ts";

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
  credentialExpiresAt?: number | string | null;
  credentialSource?: "claude-code" | "opencode" | "environment";
  /** Provider-side failure classification (OpenChamber quota module:
   *  e.g. meta's "no-data" / "unauthorized" / "exhausted"). Preserved
   *  so an ok-but-null read can be told apart from a measurement. */
  failureClass?: string;
  /** Epoch ms of the provider's last successful read in-process, when
   *  the module carries one (meta does); null = never succeeded. */
  lastSuccessAt?: number | null;
  quotaExhausted?: boolean;
}

interface RawWindow {
  usedPercent?: unknown;
  windowSeconds?: unknown;
  resetAt?: unknown;
  valueLabel?: unknown;
}

const num = (v: unknown): number | null => (isFiniteNumber(v) ? v : null);

/** Provider reset stamps arrive as epoch ms (or s); normalize to ISO. */
export function normalizeResetAt(v: unknown): string | null {
  if (isFiniteNumber(v) && v > 0) {
    const ms = v < 1e12 ? v * 1000 : v;
    const d = new Date(ms);
    return Number.isFinite(d.getTime()) ? d.toISOString() : null;
  }
  if (typeof v === "string" && v !== "") {
    // Shared epoch/calendar primitive: an impossible date (2026-02-30)
    // is not silently rolled into the next month (#2383/#2470).
    const d = parseIsoDate(v);
    return d === null ? null : d.toISOString();
  }
  return null;
}

/**
 * Hub pool/snapshot service id for a provider result. Mirrors
 * POOL_SPECS (src/quota-extract.ts) where a pool exists. Pool ids are
 * the OpenCode config provider ids (2026-11-03 alignment — see the
 * POOL NAMING note in quota-extract.ts), so a renamed pool starts a
 * NEW snapshot series under its config name; the pre-rename series
 * stays in the store under the old colon id, unrewritten.
 * OpenCode Go answers one row per workspace credential (the module's
 * per-credential fetch); each row lands under the bare id, and
 * entriesWithPoolSplits below ALSO files attributable rows under
 * their split pool ids. Antigravity's per-model windows likewise keep
 * their flattened names here; the pool grouping is a split-row step.
 */
export function serviceIdFor(providerId: string): string {
  switch (providerId) {
    case "claude":
    case "_claude":
      return "anthropic:claude-pro";
    case "_openai":
      return "_openai.main";
    case "cline-pass":
      return "cline-pass";
    case "command-code":
      return "commandcode";
    case "_commandcode":
      return "_commandcode";
    case "deepseek":
      return "deepseek";
    case "meta":
    case "_muse":
      return "meta";
    case "opendesign":
      return "opendesign";
    default:
      return providerId;
  }
}

/**
 * Canonical provider ids for snapshot entries. The quota module answers
 * in its own spellings; the snapshot contract speaks the OpenCode config
 * provider ids — the router matches quota readings by exact provider id
 * (quota-driven skip re-admission), so an entry published under a module
 * spelling never reaches the provider it describes. The fleet-built
 * providers were renamed to leading-underscore config ids in the fork
 * (#1823, 2026-11-07): antigravity -> _antigravity, the claude-code door
 * (module: "claude") -> _claude, commandcode (module: "command-code")
 * -> _commandcode, and the muse-direct door (module: "meta", the Muse
 * Code High Usage reading) -> _muse. The OpenAI quota door (module:
 * "codex" before fork PR #2261) -> _openai by the same rule (2026-11-10,
 * code#8163). Normalize here, at the one mapping
 * point every gathered result passes through. Pool/service ids are a
 * separate contract and stay put: serviceIdFor carries the dual-spelling
 * cases (both "claude" and "_claude" land on anthropic:claude-pro, and
 * "_muse" keeps the "meta" pool).
 */
const CANONICAL_PROVIDER_IDS: Readonly<Record<string, string>> = {
  codex: "_openai",
  antigravity: "_antigravity",
  claude: "_claude",
  "command-code": "_commandcode",
  meta: "_muse",
};

export function canonicalProviderId(providerId: string): string {
  return CANONICAL_PROVIDER_IDS[providerId] ?? providerId;
}

export const GATHERER_SOURCE = "opencode-quota";

/**
 * Split pool service ids the gatherer emits alongside the combined rows
 * (#1331, epic #1149). The combined rows — bare `opencode-go` (one entry
 * per workspace credential) and the per-model `antigravity` windows —
 * keep working for existing consumers; the split rows carry the SAME
 * readings under the pool ids the /quota overlay maps
 * (overlayQuotaCurrent keys pools by entry serviceId, so a split row
 * lands on its hub pool with freshness + values on the 15-minute tick).
 * These MUST mirror POOL_SPECS (src/quota-extract.ts) — the overlay side
 * of this contract.
 */
export const GO_DREWRYPOPE_SERVICE_ID = "opencode-go.drewrypope";
export const GO_POPESDREW_SERVICE_ID = "opencode-go.popesdrew";
export const GO_DEVELOPING_TODAY_0_SERVICE_ID = "opencode-go.developing-today-0";
export const ANTIGRAVITY_GEMINI_SERVICE_ID = "_antigravity.gemini";
export const ANTIGRAVITY_CLAUDE_GPT_SERVICE_ID = "_antigravity.claude-gpt";
// Pre-rename (#1823) split ids: a gatherer still paired with the old
// fork binary reports providerId "antigravity" and its rows keep the
// historical series; the renamed provider starts the new series.
export const ANTIGRAVITY_LEGACY_GEMINI_SERVICE_ID = "antigravity-direct.gemini";
export const ANTIGRAVITY_LEGACY_CLAUDE_GPT_SERVICE_ID = "antigravity-direct.claude-gpt";

export const OPENAI_MAIN_SERVICE_ID = "_openai.main";
export const OPENAI_ALTERNATE_SERVICE_ID = "_openai.alt";
export const OPENAI_SELF_SERVICE_ID = "_openai.main";

/**
 * Which OpenAI pool a folded `_openai` entry belongs to, from the
 * credential's human label / the instance name the fork stamps on the
 * folded result. The fork's _openai provider folds every OpenAI instance
 * credential into its per-credential answer under the bare provider id
 * "_openai" (foldInstanceCredentials), and serviceIdFor historically
 * filed ALL of those rows under the main pool — so the alternate
 * account's 5h window landed in the snapshot store under main's key
 * (its values mirror the alternate series tick-for-tick) and its
 * weekly competed under main's key in the #1624 max-collapse
 * (2026-11-04: main|weekly published 100 with the ALTERNATE window's
 * reset label while main's own read was ~17). Like goPoolServiceId,
 * the label is the only stable gatherer-side signal: instance ids are
 * config names, credential ids are opaque row ids the mapping would
 * have to hardcode. Unmatched entries yield null and keep the fallback
 * (serviceIdFor: _openai.main, the base provider's own pool) — never a
 * fabricated pool row.
 *
 * Service ids are the door ids (Drew, 2026-11-10, code#8163):
 * _openai.main (drewrypope) and _openai.alt (popesdrew). The
 * pre-rename ids (openai.main / openai.alternate) survive only in the
 * historical snapshot series, unrewritten; the dedicated _openai door
 * and the openai.main instance are the same account, so their rows
 * file together under _openai.main.
 */
export function openaiPoolServiceId(entry: {
  credentialLabel?: string | null;
  providerName?: string | null;
}): string | null {
  const text = `${entry.credentialLabel ?? ""} ${entry.providerName ?? ""}`.toLowerCase();
  if (text.includes("alternate") || text.includes("_openai.alt")) return OPENAI_ALTERNATE_SERVICE_ID;
  if (text.includes("_openai") || text.includes("self-contained") || text.includes("self contained"))
    return OPENAI_SELF_SERVICE_ID;
  if (text.includes("main")) return OPENAI_MAIN_SERVICE_ID;
  return null;
}

/**
 * Which split Go pool a per-workspace entry belongs to, from the
 * credential's human label. OpenCode credential labels are user-set; on
 * the fleet laptop the Go accounts are labeled "drewrypope", "popesdrew"
 * and "developing-today-0" (third account, added 2026-11-08)
 * (the "Home" workspace was removed 2026-11-03 and has no hub pool). Org ids are opaque per-machine strings, so they
 * cannot name pools — the label is the only stable gatherer-side
 * signal. Unmatched entries yield null: no fabricated pool row, the
 * combined row still carries the reading.
 */
export function goPoolServiceId(entry: QuotaCurrentEntry): string | null {
  const label = entry.credentialLabel?.trim().toLowerCase();
  if (label === "drewrypope") return GO_DREWRYPOPE_SERVICE_ID;
  if (label === "popesdrew") return GO_POPESDREW_SERVICE_ID;
  if (label === "developing-today-0") return GO_DEVELOPING_TODAY_0_SERVICE_ID;
  return null;
}

/**
 * Antigravity model -> hub pool. Mirrors poolFor in
 * scripts/collect-fleet-feed.ts (the hub's burn-attribution rule):
 * gemini-prefixed models burn the Gemini pool, everything else the
 * Claude+GPT pool. Quota grouping must use the same rule, or burn and
 * quota would disagree about which pool a model draws from.
 */
function antigravityPoolForModel(model: string, legacy = false): string {
  if (legacy)
    return model.startsWith("gemini") ? ANTIGRAVITY_LEGACY_GEMINI_SERVICE_ID : ANTIGRAVITY_LEGACY_CLAUDE_GPT_SERVICE_ID;
  return model.startsWith("gemini") ? ANTIGRAVITY_GEMINI_SERVICE_ID : ANTIGRAVITY_CLAUDE_GPT_SERVICE_ID;
}

/**
 * Derive one pool row per Antigravity hub pool from a single provider
 * entry's flattened "<model>:<window>" windows. Each pool row carries,
 * per window name, the hottest (max pctOfLimit) reading in its group —
 * a spent model can never hide behind a fresh sibling — with that
 * window's own stamps. Candidate windows are name-sorted and
 * replacement is strict-greater, so ties resolve deterministically
 * regardless of the module's model order. Label-only windows (no
 * pctOfLimit) and non-model windows (no "<model>:" prefix) stay
 * combined-only; an empty group yields no row — never a zero.
 */
function antigravityPoolRows(entry: QuotaCurrentEntry): QuotaCurrentEntry[] {
  if (entry.state === "unconfigured") return [];
  if (entry.state !== "ok") {
    // A failed provider read lapses both pools loudly — history
    // retained, no stale % presented as current — instead of leaving
    // the pools on their dated reads with no signal the source died.
    const ids =
      entry.providerId === "antigravity"
        ? [ANTIGRAVITY_LEGACY_GEMINI_SERVICE_ID, ANTIGRAVITY_LEGACY_CLAUDE_GPT_SERVICE_ID]
        : [ANTIGRAVITY_GEMINI_SERVICE_ID, ANTIGRAVITY_CLAUDE_GPT_SERVICE_ID];
    return ids.map((serviceId) => ({
      ...entry,
      serviceId,
    }));
  }
  type Win = QuotaCurrentEntry["windows"][number];
  const best = new Map<string, Map<string, Win>>();
  const candidates = [...entry.windows].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  for (const w of candidates) {
    const sep = w.name.lastIndexOf(":");
    if (sep <= 0 || sep === w.name.length - 1) continue;
    if (w.pctOfLimit == null) continue;
    const pool = antigravityPoolForModel(w.name.slice(0, sep), entry.providerId === "antigravity");
    const winName = w.name.slice(sep + 1);
    let group = best.get(pool);
    if (!group) {
      group = new Map<string, Win>();
      best.set(pool, group);
    }
    const cur = group.get(winName);
    if (!cur || (w.pctOfLimit ?? -1) > (cur.pctOfLimit ?? -1)) group.set(winName, w);
  }
  const rows: QuotaCurrentEntry[] = [];
  for (const pool of [...best.keys()].sort()) {
    const group = best.get(pool)!;
    if (group.size === 0) continue;
    rows.push({
      ...entry,
      serviceId: pool,
      windows: [...group.entries()]
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([winName, w]) => ({ ...w, name: winName })),
    });
  }
  return rows;
}

/**
 * Append the split pool rows (#1331) to one tick's entries. Pure and
 * additive: every input entry is returned verbatim first and split rows
 * follow, so the combined rows keep working for existing consumers
 * while the snapshot writer also sees the pool-level series. Go lapse
 * states (#1118) stay representable: an errored workspace row splits
 * into a pool-level error, and retained last-known rows (#1268) split
 * into stale-classified pool rows via their preserved
 * retainedFromFailedTick stamp.
 */
export function entriesWithPoolSplits(entries: readonly QuotaCurrentEntry[]): QuotaCurrentEntry[] {
  const out = [...entries];
  for (const entry of entries) {
    if (entry.providerId === "opencode-go") {
      // Unconfigured rows are host state, not pool state: the pools keep
      // their dated reads until a real read (ok, error, or lapsed) arrives.
      // Lapsed rows split exactly like error rows — one terminal row per
      // workspace pool — so the hub names the planned lapse instead of a failure.
      const serviceId = goPoolServiceId(entry);
      if (serviceId && entry.state !== "unconfigured") out.push({ ...entry, serviceId });
    } else if (entry.providerId === "antigravity" || entry.providerId === "_antigravity") {
      out.push(...antigravityPoolRows(entry));
    }
  }
  return out;
}

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
 * Map one file-export document onto a quota-current entry. The v1
 * producer was Meta (service meta, ~/atlas/meta-quota-
 * export/latest.json on the laptop) — retired 2026-11-03 when the
 * gatherer began asking the OpenCode quota module for Meta
 * directly (PR #5040); the mapper stays as the #1301 convention's
 * tested shape for the next producer the module cannot reach.
 * `doc` null = the file could not be read or parsed; `stale` = its
 * mtime is past the staleness bound. Both are error entries — never
 * zeros.
 */
export function entryFromFileExport(
  doc: FileExportDoc | null,
  opts: { fileMtimeIso: string | null; stale: boolean },
): QuotaCurrentEntry {
  const entry: QuotaCurrentEntry = {
    providerId: canonicalProviderId("meta"),
    providerName: "Meta",
    serviceId: "meta",
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
  const measured = parseIsoDate(doc.measuredAt);
  if (doc.status !== "ok" || measured === null) {
    entry.error = `file-export status: ${doc.status ?? "unknown"}`;
    return entry;
  }
  // Fail-closed shape check (#2551): a window container or element that
  // is not what the document promises is an unparseable document, so the
  // caller retains the previous read. A null element used to throw out of
  // the mapper; a non-string name was accepted into the entry.
  if (!Array.isArray(doc.windows)) {
    entry.error = "file-export unparseable: windows missing or not an array";
    return entry;
  }
  for (const w of doc.windows) {
    if (!isRecord(w) || !isNonEmptyString(w.name)) {
      entry.error = "file-export unparseable: malformed window element";
      return entry;
    }
  }
  const measuredAt = measured.toISOString();
  const mapped: QuotaCurrentEntry["windows"] = [];
  for (const w of doc.windows) {
    const name = w.name as string;
    const pct = isFiniteNumber(w.usedPct) ? w.usedPct : null;
    // A percent outside [0, its limit] is not data (#2571): drop the
    // window rather than publish it as a live reading.
    if (pct === null || !isPercentOf(100)(pct)) continue;
    mapped.push({
      name,
      value: pct,
      pctOfLimit: pct,
      valueLabel: null,
      // Own-key rule (#2781, design #2792): a window name that is not an
      // own key — including prototype spellings (constructor/__proto__/
      // toString) — is an unknown window and gets null, never a prototype
      // member as its windowSeconds.
      windowSeconds: Object.hasOwn(FILE_EXPORT_WINDOW_SECONDS, name) ? FILE_EXPORT_WINDOW_SECONDS[name] : null,
      resetsAt: normalizeResetAt(w.resetAt),
      measuredAt,
      source: entry.source,
      state: "ok",
    });
  }
  entry.state = "ok";
  entry.measuredAt = measuredAt;
  entry.windows = mapped;
  const labelWindow = (name: string, usd: number | null | undefined): void => {
    if (!isNonNegative(usd)) return;
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

/**
 * A configured OpenCode Go result whose Console answers "no active
 * subscription" is a planned lapse (#1118, Drew ruled both plans stay
 * cancelled) — a terminal state of its own, never a provider failure.
 */
export function isGoLapseResult(result: QuotaProviderResult): boolean {
  return (
    canonicalProviderId(result.providerId) === "opencode-go" &&
    !result.ok &&
    result.configured &&
    typeof result.error === "string" &&
    result.error.includes("No active OpenCode Go subscription")
  );
}

/** The remedy every Meta session-lapse surface must name (#8294). */
export const META_SESSION_LAPSE_REMEDY =
  "dev.meta.ai portal session is logged out; re-auth (re-login) in the hub browser profile to restore Meta quota reads";

/**
 * A configured Meta result that failed because the dev.meta.ai portal
 * session lapsed (#8294). The Meta read rides a live browser session
 * on the hub (there is no stored credential — see meta-renewal.ts);
 * when that session expires, the provider leg fails session-class
 * ("unauthorized", or the exporter's "session-expired" shape) rather
 * than with a provider error. A lapsed session has a known remedy
 * (re-auth on the hub) and is not a provider failure, so — following
 * the OpenCode Go lapse template (#1161 / PR #7898) — it classifies
 * as "lapsed", never "error". Unlike the Go lapse this one is
 * temporary: the gatherer retains the pool's last good windows as
 * stale rows alongside the lapsed entry (see gather-quota.ts).
 */
export function isMetaSessionLapseResult(result: QuotaProviderResult): boolean {
  if (canonicalProviderId(result.providerId) !== "_muse" || result.ok || !result.configured) {
    return false;
  }
  if (result.failureClass === "unauthorized") return true;
  const error = typeof result.error === "string" ? result.error : "";
  return /unauthorized|session-expired|session expired|logged out/i.test(error);
}

/** Map one provider result onto a quota-current entry. */
export function entryFromResult(result: QuotaProviderResult, tickIso: string): QuotaCurrentEntry {
  let measuredAt = tickIso;
  if (typeof result.fetchedAt === "number" && Number.isFinite(result.fetchedAt) && result.fetchedAt > 0) {
    const fetchedDate = new Date(result.fetchedAt);
    if (Number.isFinite(fetchedDate.getTime())) measuredAt = fetchedDate.toISOString();
  }
  const state: QuotaCurrentEntry["state"] = result.ok
    ? "ok"
    : isGoLapseResult(result) || isMetaSessionLapseResult(result)
      ? "lapsed"
      : result.configured
        ? "error"
        : "unconfigured";
  const providerId = canonicalProviderId(result.providerId);
  // Folded _openai rows carry one credential per OpenAI instance; file
  // each under its own pool (openaiPoolServiceId) instead of the base
  // provider's pool, or a sibling account's windows contaminate
  // _openai.main's rows and snapshot series.
  const serviceId =
    providerId === "_openai"
      ? (openaiPoolServiceId({ credentialLabel: result.credentialLabel, providerName: result.providerName }) ??
        serviceIdFor(providerId))
      : serviceIdFor(providerId);
  const entry: QuotaCurrentEntry = {
    providerId,
    providerName: result.providerName ?? providerId,
    serviceId,
    state,
    measuredAt,
    source: GATHERER_SOURCE,
    credentialWatch: true,
    windows: [],
  };
  if (result.credentialId) entry.credentialId = result.credentialId;
  if (result.credentialLabel) entry.credentialLabel = result.credentialLabel;
  const expiry = result.credentialExpiresAt;
  if (typeof expiry === "number" && Number.isFinite(expiry)) {
    const date = new Date(expiry);
    if (Number.isFinite(date.getTime())) entry.credentialExpiresAt = date.toISOString();
  } else if (isIsoDate(expiry)) {
    entry.credentialExpiresAt = new Date(expiry).toISOString();
  }
  if (result.credentialSource) entry.credentialSource = result.credentialSource;
  if (result.planLabel) entry.planLabel = result.planLabel;
  if (result.error) entry.error = safeProviderReason(result.error);
  if (isMetaSessionLapseResult(result)) {
    // Name the remedy on the entry itself: this text is what the pool
    // note surfaces (quota-current freshness.error), so a lapsed Meta
    // session reads as an actionable re-auth state, not a bare error.
    entry.error = `session lapsed: ${entry.error ?? "unauthorized"} — ${META_SESSION_LAPSE_REMEDY}`;
  }
  if (typeof result.failureClass === "string" && result.failureClass !== "") entry.failureClass = result.failureClass;
  if (typeof result.lastSuccessAt === "number" && Number.isFinite(result.lastSuccessAt) && result.lastSuccessAt > 0) {
    entry.lastSuccessAt = new Date(result.lastSuccessAt).toISOString();
  }
  if (result.usage != null) entry.usage = result.usage;

  if (state === "lapsed") {
    // The provider may leave old usage alongside its lapse signal.
    // Preserve the raw response for evidence, but never publish those
    // percentages as a fresh quota window.
    return entry;
  }

  const pushWindow = (name: string, raw: RawWindow | null | undefined): void => {
    const pct = isFiniteNumber(raw?.usedPercent) ? raw.usedPercent : null;
    // Percent is bounded by the window's own 100-point limit (#2571);
    // out-of-range values are dropped, not republished as live data.
    // Turso is the exception (#6227): its windows are usage-vs-limit
    // ratios over counts (storage bytes, rows read), which legitimately
    // exceed 100 when the org is over its Starter limits — dropping
    // them erased exactly the over-limit state the card exists to
    // show (2026-11-05: storage ~147% and rows-read ~165% vanished
    // from the live row while the under-limit windows survived).
    const overLimitOk = result.providerId === "turso";
    if (pct !== null && !(overLimitOk ? isNonNegative(pct) : isPercentOf(100)(pct))) return;
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
  const usageObj = result.usage;
  if (isRecord(usageObj) && isRecord(usageObj.windows)) {
    for (const [name, raw] of Object.entries(usageObj.windows as Record<string, RawWindow>)) {
      pushWindow(name, raw);
    }
  }
  // Per-model usage (Claude's model-scoped windows, Antigravity's
  // whole shape): flatten to "<model>:<window>" so the data lands in
  // the same contract. Pool attribution across models is the hub's
  // modeling step, not the gatherer's.
  if (isRecord(usageObj) && isRecord(usageObj.models)) {
    for (const [model, mu] of Object.entries(usageObj.models as Record<string, unknown>)) {
      if (!isRecord(mu) || !isRecord(mu.windows)) continue;
      for (const [name, raw] of Object.entries(mu.windows as Record<string, RawWindow>)) {
        pushWindow(`${model}:${name}`, raw);
      }
    }
  }
  // Ok-but-null is NOT a measurement. A provider can answer ok:true
  // with every window's percentage unknown — Meta's plan endpoint
  // does exactly this on a 200 / is_subs_active:true response with no
  // subs_usage (the provider's "post-roll empty state", meta.js in
  // the OpenChamber quota module) — and since 2026-11-03 that shape
  // has replaced Meta's last good percentage read (100% weekly,
  // 2026-11-03T21:00Z) with fresh-looking null rows on every tick.
  // Name it on the entry so the gatherer can degrade the run to an
  // error and #1268 retention can keep the last-known rows, stale
  // with this reason, instead of publishing null as current data.
  if (result.ok && !entryHasMeasuredWindow(entry) && !entry.error) {
    entry.error = `no-data: provider reported ok but no window carried a percentage${
      entry.failureClass ? ` (failureClass=${entry.failureClass})` : ""
    }${entry.lastSuccessAt ? ` (provider lastSuccessAt=${entry.lastSuccessAt})` : ""}`;
  }
  return entry;
}

/**
 * Whether an entry carries at least one measured window: a
 * percentage window, or the CNY balance label that recordsFromEntries
 * treats as a measurement in its own unit (#1052, the DeepSeek
 * reserve — a label-only provider must not degrade on this rule).
 * Plain USD/credit balance labels do NOT count: a balance is not a
 * plan reading.
 */
export function entryHasMeasuredWindow(entry: QuotaCurrentEntry): boolean {
  return entry.windows.some(
    (w) => w.pctOfLimit != null || (w.valueLabel != null && /^¥\s*[\d,]+(?:\.\d+)?/.test(w.valueLabel.trim())),
  );
}

/**
 * Snapshot records for one tick: ONE pct record per serviceId|window
 * key whose VALUE changed since the gatherer last appended it (the
 * state map is serviceId|metricId -> value), so the store gains a
 * point when a reading moves, not every 15 minutes. The stamp is the
 * value alone: several providers recompute the reset instant from a
 * duration on every fetch (sub-second drift, or "now + window" for
 * idle 5h windows), so keying on resetsAt would append noise points
 * for readings that did not move. A window roll still shows up — as
 * the value change it causes. The record itself carries the current
 * resetsAt. Label-only USD windows (balances) still yield no
 * record; a label carrying a yuan balance (#1052) yields a unit
 * "cny" record with the same value-moved stamping, stored in yuan
 * exactly as read.
 *
 * Pinned rule (#1624): the gatherer emits one entry per credential
 * but series identity is pool-level (serviceId, metricId), so all
 * same-tick candidates for one key collapse to a single record —
 * the max pctOfLimit, tie-broken by lowest credentialId (then
 * earliest measuredAt/resetsAt, so entry order never matters). The
 * dedup check reads both the input state map AND this call's
 * appended map, so two same-tick credentials can never emit two
 * records for one key. A non-max credential moving while the max is
 * unchanged therefore appends nothing: the pool-level series did
 * not move.
 */
/**
 * Snapshot units for Turso windows whose metric ALSO has a native-unit
 * series (recordsFromSaasServices writes turso/{databases,storage,sync}
 * in count/GB): the gatherer's records for those windows carry the
 * native value + unit so both writers land on ONE axis. Rows windows
 * have no native series (the store has no "rows" unit) and keep pct.
 * Limits are plan-aware (storage/sync in GB): they resolve from the
 * entry's planLabel via resolveTursoLimits below, whose source of
 * truth is the Turso /plans document and whose fallback is the static
 * per-plan table — a Scaler entry's native records carry the Scaler
 * limits (1e9 databases, 24 GB storage, 24 GB sync), never the
 * Starter ones (code#5468).
 */
const tursoNativeRecords = (
  planLabel: string | null | undefined,
): Record<string, { unit: SnapshotRecord["unit"]; limit: number }> => {
  const limits = resolveTursoLimits(planLabel ?? undefined, null);
  return {
    databases: { unit: "count", limit: limits.databases },
    storage: { unit: "gb", limit: limits.storageBytes / 1e9 },
    sync: { unit: "gb", limit: limits.bytesSynced / 1e9 },
  };
};

export function recordsFromEntries(
  entries: QuotaCurrentEntry[],
  lastAppended: ReadonlyMap<string, string>,
  observedBy: string,
): { records: SnapshotRecord[]; appended: Map<string, string> } {
  const records: SnapshotRecord[] = [];
  const appended = new Map<string, string>();
  // One candidate per pool-level key: the max pct wins, lowest
  // credentialId breaks ties so the pick is order-independent.
  const best = new Map<string, { entry: QuotaCurrentEntry; w: QuotaCurrentEntry["windows"][number] }>();
  const credOf = (e: QuotaCurrentEntry): string => e.credentialId ?? "";
  for (const entry of entries) {
    if (entry.state !== "ok") continue;
    for (const w of entry.windows) {
      // Own-key rule (#2781, design #2792): only an own key takes the
      // native-record branch; a prototype spelling falls through to the
      // normal pct path like any unknown window name.
      const nativeRecords = entry.serviceId === "turso" ? tursoNativeRecords(entry.planLabel) : undefined;
      const native = nativeRecords && Object.hasOwn(nativeRecords, w.name) ? nativeRecords[w.name] : undefined;
      if (native) {
        if (w.value == null) continue;
        const key = `${entry.serviceId}|${w.name}`;
        const stamp = `${w.value}:${native.unit}`;
        if (lastAppended.get(key) === stamp) continue;
        appended.set(key, stamp);
        records.push({
          ts: w.measuredAt,
          serviceId: entry.serviceId,
          metricId: w.name,
          value: w.value,
          unit: native.unit,
          limit: native.limit,
          ...(w.resetsAt ? { resetsAt: w.resetsAt } : {}),
          source: entry.source.startsWith("file-export") ? "console" : "api",
          observedBy,
        });
        continue;
      }
      if (w.pctOfLimit == null) {
        // CNY balance (#1052): the DeepSeek reserve's label is
        // "¥44.29" — a yuan figure is its own unit, never converted.
        // Same value-moved dedupe as pct windows (checked against
        // both the input state map and this call's appended map, per
        // the #1624 pinned rule), so the store gains a point when
        // the balance actually moves.
        const cny = w.valueLabel == null ? null : /^¥\s*([\d,]+(?:\.\d+)?)/.exec(w.valueLabel.trim());
        if (!cny) continue;
        const value = Number(cny[1].replace(/,/g, ""));
        if (!Number.isFinite(value)) continue;
        const key = `${entry.serviceId}|${w.name}`;
        const stamp = `${value}`;
        if (lastAppended.get(key) === stamp || appended.get(key) === stamp) continue;
        appended.set(key, stamp);
        records.push({
          ts: w.measuredAt,
          serviceId: entry.serviceId,
          metricId: w.name,
          value,
          unit: "cny",
          source: entry.source.startsWith("file-export") ? "console" : "api",
          observedBy,
        });
        continue;
      }
      const key = `${entry.serviceId}|${w.name}`;
      const cur = best.get(key);
      if (!cur) {
        best.set(key, { entry, w });
        continue;
      }
      const pct = w.pctOfLimit;
      const curPct = cur.w.pctOfLimit;
      if (
        curPct == null ||
        pct > curPct ||
        (pct === curPct &&
          (credOf(entry) < credOf(cur.entry) ||
            (credOf(entry) === credOf(cur.entry) &&
              (w.measuredAt < cur.w.measuredAt ||
                (w.measuredAt === cur.w.measuredAt && (w.resetsAt ?? "") < (cur.w.resetsAt ?? ""))))))
      ) {
        best.set(key, { entry, w });
      }
    }
  }
  // Deterministic emission order regardless of input order.
  for (const key of [...best.keys()].sort()) {
    const cand = best.get(key);
    if (!cand) continue;
    const { entry, w } = cand;
    if (w.pctOfLimit == null) continue;
    const stamp = `${w.pctOfLimit}`;
    if (lastAppended.get(key) === stamp || appended.get(key) === stamp) continue;
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
  return { records, appended };
}

// ---------------------------------------------------------------------
// Turso (#1049): the org usage route the v2 collector reads —
// GET /v1/organizations/{slug}/usage + /subscription on
// api.turso.tech, authorized by the platform token the gatherer
// resolves from Bitwarden by name (never stored here). The route was
// verified live 2026-10-29 (triage task 2311): usage answers
// organization.usage {rows_read, rows_written, storage_bytes,
// bytes_synced, databases, locations, groups} as current-period
// totals (no daily series exists — /usage/daily is 404; daily
// granularity is the snapshot store's, via recordsFromEntries), and
// subscription answers {plan, current_billing_period_start/end},
// whose period end is the
// card's cycle end. Mapping is tolerant of the envelope: usage may
// sit at organization.usage or top-level usage, subscription fields
// at subscription.* or top level.
// ---------------------------------------------------------------------

export interface TursoUsageDoc {
  organization?: { usage?: Record<string, unknown> | null } | null;
  usage?: Record<string, unknown> | null;
}

export interface TursoSubscriptionDoc {
  subscription?: Record<string, unknown> | null;
  plan?: unknown;
  current_period_end?: unknown;
  period_end?: unknown;
}

export interface TursoLimits {
  databases: number;
  storageBytes: number;
  rowsRead: number;
  rowsWritten: number;
  bytesSynced: number;
}

/** Starter Free limits from #1049 (bytes are decimal, as billed) —
 * now only the fallback for an unknown/unreadable plan. */
export const TURSO_LIMITS = {
  databases: 100,
  storageBytes: 5_000_000_000,
  rowsRead: 500_000_000,
  rowsWritten: 10_000_000,
  bytesSynced: 3_000_000_000,
} as const;

/**
 * FALLBACK ONLY — static per-plan limits, keyed by plan name, with the
 * values the /plans endpoint published on 2026-11-08. The source of
 * truth is the live /plans document (resolveTursoLimits prefers it,
 * exactly as the opencode turso provider does); this table exists so
 * that when no plans doc is at hand, a Scaler (or Developer/Pro)
 * entry still divides by its own plan's limits instead of silently
 * regressing to Starter denominators — the stale-denominator artifact
 * behind the false over-limit alarms on #5468 (org on Scaler since
 * 2026-11-04; Starter caps made rows-read read ~235%). The /plans
 * document carries no `databases` quota for pro; the fallback uses
 * the same 1e9 ceiling the other paid plans publish.
 */
export const TURSO_PLAN_LIMITS: Record<string, TursoLimits> = {
  starter: { ...TURSO_LIMITS },
  developer: {
    databases: 1_000_000_000,
    storageBytes: 9_000_000_000,
    rowsRead: 2_500_000_000,
    rowsWritten: 25_000_000,
    bytesSynced: 10_000_000_000,
  },
  scaler: {
    databases: 1_000_000_000,
    storageBytes: 24_000_000_000,
    rowsRead: 100_000_000_000,
    rowsWritten: 100_000_000,
    bytesSynced: 24_000_000_000,
  },
  pro: {
    databases: 1_000_000_000,
    storageBytes: 50_000_000_000,
    rowsRead: 250_000_000_000,
    rowsWritten: 250_000_000,
    bytesSynced: 100_000_000_000,
  },
};

/** The /plans document: GET /v1/organizations/{slug}/plans answers
 * { plans: [{ name, quotas: { rowsRead, rowsWritten, databases,
 * locations, storage, groups, bytesSynced } }] } — quotas are the
 * plan's limits in native units (storage/bytesSynced in bytes). */
export interface TursoPlansDoc {
  plans?: unknown;
}

const positiveQuota = (v: unknown): number | null => (isFiniteNumber(v) && v > 0 ? v : null);

/**
 * Resolve the limits a Turso org's usage divides by. Dynamic first:
 * the /plans entry whose name matches the plan label
 * (case-insensitive) supplies the quotas; any quota the matched plan
 * omits falls back per-field to the static plan table. Without a
 * plans doc (or a match), the static table keyed by plan name
 * applies; an unknown plan gets Starter (TURSO_LIMITS, the previous
 * behaviour).
 */
export function resolveTursoLimits(planLabel?: string, plansDoc?: TursoPlansDoc | null): TursoLimits {
  const fallback: TursoLimits = (planLabel ? TURSO_PLAN_LIMITS[planLabel.toLowerCase()] : undefined) ?? {
    ...TURSO_LIMITS,
  };
  if (!plansDoc || typeof planLabel !== "string" || planLabel === "" || !Array.isArray(plansDoc.plans)) {
    return fallback;
  }
  const wanted = planLabel.toLowerCase();
  for (const entry of plansDoc.plans) {
    if (!isRecord(entry) || typeof entry.name !== "string" || entry.name.toLowerCase() !== wanted) continue;
    if (!isRecord(entry.quotas)) return fallback;
    const q = entry.quotas;
    return {
      databases: positiveQuota(q.databases) ?? fallback.databases,
      storageBytes: positiveQuota(q.storage) ?? fallback.storageBytes,
      rowsRead: positiveQuota(q.rowsRead) ?? fallback.rowsRead,
      rowsWritten: positiveQuota(q.rowsWritten) ?? fallback.rowsWritten,
      bytesSynced: positiveQuota(q.bytesSynced) ?? fallback.bytesSynced,
    };
  }
  return fallback;
}

export const TURSO_SOURCE = "turso-api";

const gb = (bytes: number): number => Math.round((bytes / 1e9) * 1e6) / 1e6;
const pct3 = (used: number, limit: number): number => Math.round((used / limit) * 100 * 1000) / 1000;

/** Turso usage counts are counts: a negative value is impossible, so it is
 * treated exactly like a metric the response does not carry (#2553) — the
 * window is omitted, never published negative. Local to entryFromTurso:
 * the shared num() keeps its semantics for the other mappers. */
const numNonNegative = (v: unknown): number | null => (isNonNegative(v) ? v : null);

/**
 * Map one Turso usage + subscription read onto a quota-current
 * entry (serviceId "turso", the SaaS card's snapshot id, so the
 * gatherer's pct records continue the series recordsFromSaasServices
 * and the backfill started). Windows carry the used value in the
 * metric's own unit (counts/rows raw, storage + sync in GB), the
 * percent of the org's plan limit (dynamic via opts.plansDoc, with
 * the per-plan fallback — see resolveTursoLimits), and the
 * subscription period end as resetsAt. A metric the response does not carry is omitted — never
 * zeroed; a read with no usable metrics (or an explicit error) is an
 * error entry, so the hub keeps the previous dated value flagged
 * stale instead of showing a fresh-looking zero.
 */
export function entryFromTurso(
  usageDoc: TursoUsageDoc | null,
  subscriptionDoc: TursoSubscriptionDoc | null,
  opts: { measuredAt: string; error?: string; plansDoc?: TursoPlansDoc | null },
): QuotaCurrentEntry {
  const entry: QuotaCurrentEntry = {
    providerId: "turso",
    providerName: "Turso",
    serviceId: "turso",
    state: "error",
    measuredAt: opts.measuredAt,
    source: TURSO_SOURCE,
    windows: [],
  };
  const sub = (subscriptionDoc?.subscription ?? subscriptionDoc) as Record<string, unknown> | null | undefined;
  const plan = sub?.plan ?? sub?.name;
  if (typeof plan === "string" && plan !== "") entry.planLabel = plan;
  const limits = resolveTursoLimits(entry.planLabel, opts.plansDoc ?? null);
  if (opts.error) {
    entry.error = opts.error;
    return entry;
  }
  const usage = usageDoc?.organization?.usage ?? usageDoc?.usage;
  if (!isRecord(usage)) {
    entry.error = "turso usage response missing organization.usage";
    return entry;
  }
  const periodEnd = sub?.current_billing_period_end ?? sub?.current_period_end ?? sub?.period_end ?? sub?.ends_at;
  const periodStart =
    sub?.current_billing_period_start ?? sub?.current_period_start ?? sub?.period_start ?? sub?.starts_at;
  const resetsAt = normalizeResetAt(periodEnd);
  // Gate both stamps with parseIsoDate (#5091): Date.parse rolls impossible
  // dates (2026-02-30 -> Mar 2) and parses non-ISO strings timezone-
  // dependently, so a malformed period would silently publish a window
  // length days off. A present-but-invalid stamp yields null.
  const startMs = parseIsoDate(periodStart)?.getTime() ?? Number.NaN;
  const endMs = parseIsoDate(periodEnd)?.getTime() ?? Number.NaN;
  const windowSeconds =
    Number.isFinite(startMs) && Number.isFinite(endMs) && endMs > startMs ? Math.round((endMs - startMs) / 1000) : null;

  const push = (name: string, rawUsed: number | null, limit: number, value: number, valueLabel: string): void => {
    if (rawUsed == null) return;
    const rawPctOfLimit = (rawUsed / limit) * 100;
    const pctOfLimit = pct3(rawUsed, limit);
    // The raw count can be finite while scaling it to a percentage
    // overflows; a non-finite ratio is not a usable live window
    // (#2579). A ratio ABOVE 100 is different (#6227): these windows
    // are usage-vs-limit ratios over counts, and an org over its
    // Starter limits is a real, reportable state (2026-11-05 live:
    // storage ~147%, rows-read ~165%) — the window is kept with its
    // native value and its >100 pctOfLimit, never dropped.
    if (!isNonNegative(rawPctOfLimit) || !isNonNegative(pctOfLimit)) return;
    entry.windows.push({
      name,
      value,
      pctOfLimit,
      valueLabel,
      windowSeconds,
      resetsAt,
      measuredAt: opts.measuredAt,
      source: TURSO_SOURCE,
      state: "ok",
    });
  };
  const databases = numNonNegative(usage.databases);
  push("databases", databases, limits.databases, databases ?? 0, `${databases} / ${limits.databases}`);
  const storageBytes = numNonNegative(usage.storage_bytes);
  push(
    "storage",
    storageBytes,
    limits.storageBytes,
    storageBytes == null ? 0 : gb(storageBytes),
    `${storageBytes == null ? "?" : gb(storageBytes)} GB / ${gb(limits.storageBytes)} GB`,
  );
  const rowsRead = numNonNegative(usage.rows_read);
  push(
    "rows-read",
    rowsRead,
    limits.rowsRead,
    rowsRead ?? 0,
    `${rowsRead?.toLocaleString("en-US")} / ${limits.rowsRead.toLocaleString("en-US")}`,
  );
  const rowsWritten = numNonNegative(usage.rows_written);
  push(
    "rows-written",
    rowsWritten,
    limits.rowsWritten,
    rowsWritten ?? 0,
    `${rowsWritten?.toLocaleString("en-US")} / ${limits.rowsWritten.toLocaleString("en-US")}`,
  );
  const bytesSynced = numNonNegative(usage.bytes_synced);
  push(
    "sync",
    bytesSynced,
    limits.bytesSynced,
    bytesSynced == null ? 0 : gb(bytesSynced),
    `${bytesSynced == null ? "?" : gb(bytesSynced)} GB / ${gb(limits.bytesSynced)} GB`,
  );
  if (entry.windows.length === 0) {
    entry.error = "turso usage response carried no known metrics";
    return entry;
  }
  entry.state = "ok";
  entry.usage = usage;
  return entry;
}

/**
 * #1268: map over items with at most `limit` invocations in flight,
 * preserving input order in the results. The gatherer fetches ~30
 * catalogue providers per tick; an unbounded Promise.all fires all
 * of them at once. A non-positive or non-finite limit degrades to
 * sequential (1) rather than to unbounded. Errors propagate like
 * Promise.all: the first rejection rejects the whole map (callers
 * that fail soft per item catch inside their worker, as runProvider
 * does).
 */
export async function mapWithConcurrency<T, R>(
  items: readonly T[],
  limit: number,
  fn: (item: T, index: number) => Promise<R>,
): Promise<R[]> {
  const cap = Number.isFinite(limit) && limit >= 1 ? Math.floor(limit) : 1;
  const results = new Array<R>(items.length);
  let next = 0;
  const workers = Array.from({ length: Math.min(cap, items.length) }, async () => {
    while (next < items.length) {
      const i = next++;
      results[i] = await fn(items[i], i);
    }
  });
  await Promise.all(workers);
  return results;
}

/**
 * #1268: the last-known entries for a provider — the previous tick's
 * `ok` entries, verbatim. When this tick's run for a provider errors
 * out (timeout, fetch failure), the gatherer retains these instead
 * of writing fresh error rows over them: the entries keep their
 * original measuredAt, so the hub's freshness classification
 * (quota-current.ts) surfaces them as stale — last-known data with
 * visible staleness, never fresh-looking zeros and never a vanished
 * provider. Error/unconfigured previous entries are not data and are
 * never retained.
 */
export function lastKnownEntries(prev: readonly QuotaCurrentEntry[], providerId: string): QuotaCurrentEntry[] {
  const wanted = canonicalProviderId(providerId);
  return prev.filter((e) => canonicalProviderId(e.providerId) === wanted && e.state === "ok");
}

/**
 * #1268: stamp retained entries with the failed tick that kept them.
 * Copies, never the previous tick's objects; measuredAt is preserved
 * (it is the honest age of the data) — the stamp is the explicit
 * signal readers use to classify the entry stale immediately,
 * instead of it reading as fresh until the freshness bound lapses.
 */
export function markRetained(entries: readonly QuotaCurrentEntry[], failedTickIso: string): QuotaCurrentEntry[] {
  return entries.map((e) => ({
    ...e,
    providerId: canonicalProviderId(e.providerId),
    retainedFromFailedTick: failedTickIso,
  }));
}
