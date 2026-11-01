// The fleet feed: a JSON file produced on the laptop by
// scripts/collect-fleet-feed.ts, run by the laptop's fleet-feed-collector
// user timer (declared in the pm-stack NixOS module, #3363/#3368) and
// written straight to the aggregator's state dir. (The collector
// originally ran on the sandbox and pushed the feed here; that sandbox
// timer is retired — a stale sandbox copy overwriting this file was
// code#3362.) The aggregator renders the feed as-is; a stale feed keeps
// rendering with its age stated in provenance.
import { readFile } from "node:fs/promises";
import type { UnmatchedCharge } from "./charges-match.ts";
import type { CheckoutDrift } from "./checkout-drift.ts";
import { isProjectPoolItem } from "./projects.ts";
import { isRepoCensus, type RepoCensus } from "./coverage.ts";
import { arrayOf, isFiniteNumber, isIsoDate, isRecord, isStringArray, isTimestampNotFuture } from "./validate.ts";
import type {
  AaFeedSection,
  BurnSegment,
  ChargeRow,
  DnsSection,
  GoalMeasurement,
  JulesSummary,
  OrchestrationProvenance,
  OrchestrationSection,
  PaygLine,
  ProjectPoolItem,
  QuotaPool,
  QuotaView,
  QuotaWindow,
  SaasService,
  SessionFeedEntry,
  SessionRecord,
  SnapshotRecord,
} from "./model.ts";

export interface FleetFeed {
  version: 1;
  /** ISO timestamp of collection on the sandbox. */
  generatedAt: string;
  /** Collector checkout drift vs origin/main (#1116 item 3), measured
   *  by the collector in its own checkout at collection time. Absent
   *  in feeds written before the drift check existed. */
  checkout?: CheckoutDrift;
  quota: QuotaView;
  /** Element drops loadFeed's quota sanitization made (#2429), set
   * only on feeds returned by loadFeed; absent on raw parsed feeds. */
  quotaDropped?: { pools: number; windows: number; burn: number };
  /** Element drops loadFeed's chargesAttention sanitization made
   * (#2439), set only when at least one entry dropped; absent on raw
   * parsed feeds and on feeds whose entries all survived. */
  chargesAttentionDropped?: number;
  /** Subscription rows loadFeed's sanitization dropped (#2446), set
   * only when at least one row (or a non-array section) was dropped;
   * absent on raw parsed feeds and on clean loads. */
  subscriptionsDropped?: number;
  /** Rows loadFeed's charges sanitization dropped (#2445), set only
   * when drops occurred; absent on raw parsed feeds and clean loads. */
  chargesDropped?: number;
  /** Sandbox-server + Jules sessions, pre-classification (render classifies). */
  attention: SessionFeedEntry[];
  /** Jules keeper totals (slice 2); absent when the keeper state is unreadable. */
  julesSummary?: JulesSummary;
  /** The orchestration job's latest section (#899), when one exists. */
  orchestration?: OrchestrationSection;
  /** The DNS/domain register (#902), parsed by the collector (slice 3). */
  dns?: DnsSection;
  /** Identity-registry records (#906 §3): sandbox OpenCode + Jules + Muse launches. */
  registry?: SessionRecord[];
  /** Usage & billing hub (design §2.3): family-2 services extracted from
   * quota-view.md Part (b) by the collector; absent in older feeds. */
  saas?: SaasService[];
  /** Hub PAYG lines (§1.2), from quota-view.md + the provider registry. */
  payg?: PaygLine[];
  /** Hub charges evidence (§2.4), from the same dated records. */
  charges?: ChargeRow[];
  /** Charges auto-matching (#1057): unmatched SaaS/AI-looking charges
   * surfacing for the FIRST time this run — the collector filters out
   * anything its persisted surfaced-state already showed, so a charge
   * appears here exactly once and the live source turns each entry
   * into a hub attention flag. Absent when nothing new surfaced. */
  chargesAttention?: UnmatchedCharge[];
  /** Hub subscriptions strip rows (Q5: renewal-only). */
  subscriptions?: { name: string; plan: string; renewsAt: string | null }[];
  /** Usage snapshot records (hub §4 writers): derived by the collector
   * from this feed's own hub sections plus the writers' outbox, for
   * the laptop aggregator to ingest into its local snapshot store.
   * Absent in older feeds; the store dedupes, so republication of the
   * same observations appends nothing. */
  snapshots?: SnapshotRecord[];
  /** GitHub Project pool (#910 stage 5): every item of the org Project
   * "Developing Today" (#4) with its Project fields and linked issue
   * state, read by the collector via GraphQL. Absent when the
   * collector had no Projects token or the read failed. */
  projects?: ProjectPoolItem[];
  /** Project #4 coverage census (#1220): per-repo open-issue and
   * gate-label counts from the collector's Search API reads, embedded
   * beside the projects section. Absent when the collector had no
   * GitHub token or the census failed. */
  coverage?: RepoCensus[];
  /** AA cost explorer section: the ainventory (AA import) data read
   * by the collector from Supabase, plus pool/catalog coverage computed
   * against the repo's opencode config. Present-but-absent
   * ({status:"absent", reason}) when Supabase was unreachable — the
   * section degrades, the feed never fails over it. */
  aa?: AaFeedSection;
  /** Optional collector outcomes. Older feeds have no metadata and keep
   *  their historical presence-only health/provenance behavior. */
  meta?: FeedMetadata;
}

export type FeedSectionStatus = "ok" | "error" | "skipped";

export interface FeedSectionCollectionStatus {
  status: FeedSectionStatus;
  /** When this collection attempt finished (ISO timestamp). */
  at: string;
  /** Sanitized diagnostic for an error outcome only. */
  error?: string;
}

export interface FeedMetadata {
  sections: Record<string, FeedSectionCollectionStatus>;
}

/** Keep collector diagnostics useful without allowing credentials or
 *  response payloads to leak into the shared feed. */
export function sanitizeCollectionError(value: unknown): string {
  let message = value instanceof Error ? value.message : typeof value === "string" ? value : "collection failed";
  message = message
    .replace(/\bBearer\s+[^\s,;]+/gi, "Bearer [redacted]")
    .replace(/\bBasic\s+[A-Za-z0-9+/=]+/gi, "Basic [redacted]")
    .replace(/\b(?:gh[pousr]_[A-Za-z0-9_]{8,}|github_pat_[A-Za-z0-9_]+|xox[baprs]-[A-Za-z0-9-]+)/gi, "[redacted]")
    .replace(/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g, "[redacted]")
    .replace(/\b((?:access[_-]?token|refresh[_-]?token|token|api[_-]?key|password|secret|authorization)\s*[:=]\s*)(?:"[^"]*"|'[^']*'|[^\s,;&]+)/gi, "$1[redacted]")
    .replace(/(https?:\/\/)[^/@\s:]+(?::[^/@\s]*)?@/gi, "$1[redacted]@")
    .replace(/\b[A-Za-z0-9_-]{40,}\b/g, "[redacted]")
    .replace(/[\r\n\t]+/g, " ")
    .slice(0, 240);
  return message || "collection failed";
}

export function feedSectionStatus(
  status: FeedSectionStatus,
  at: string,
  error?: unknown,
): FeedSectionCollectionStatus {
  return status === "error"
    ? { status, at, error: sanitizeCollectionError(error) }
    : { status, at };
}

/** Tolerate malformed optional metadata without rejecting an otherwise
 *  usable feed. The same boundary also sanitizes error text from older
 *  or externally-written collectors. */
export function sanitizeFeedMetadata(value: unknown): FeedMetadata | undefined {
  if (!isRecord(value) || !isRecord(value.sections)) return undefined;
  const sections: FeedMetadata["sections"] = {};
  for (const [key, raw] of Object.entries(value.sections)) {
    if (!/^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(key) || !isRecord(raw)) continue;
    const at = raw.at;
    if (
      (raw.status !== "ok" && raw.status !== "error" && raw.status !== "skipped") ||
      typeof at !== "string" ||
      !isTimestampNotFuture(at)
    ) {
      continue;
    }
    sections[key] = feedSectionStatus(raw.status, at, raw.error);
  }
  return Object.keys(sections).length ? { sections } : undefined;
}

/**
 * Minimal structural check for an orchestration section, shared by the
 * collector (which embeds the orchestrator's file fail-soft) and the
 * live source (which serves it). Absent or malformed = "not measured".
 */
/**
 * Per-goal gate (#949): `todayFromSection` dereferences `linedUp`,
 * `stalled`, and `gaps` on every goal, so a goal missing any of them must
 * fail validation here — this validator is the only gate on every path,
 * and a rejected section falls back to the derived view by design.
 * `progressedSinceLastRun` is deliberately string-or-array: `asStringList`
 * in derive.ts normalizes the single-string form the orchestrator
 * sometimes publishes, so requiring strictly an array would reject real
 * sections.
 */
function isGoalMeasurement(v: unknown): boolean {
  if (typeof v !== "object" || v === null) return false;
  const g = v as Partial<GoalMeasurement>;
  return (
    Array.isArray(g.linedUp) &&
    Array.isArray(g.stalled) &&
    Array.isArray(g.gaps) &&
    (typeof g.progressedSinceLastRun === "string" || Array.isArray(g.progressedSinceLastRun))
  );
}

/**
 * The section's provenance (#1110). The contract is the publisher's
 * object form — `{ sources: string[], runId?: string }` — which every
 * orchestration run actually emits. The legacy plain-string form is
 * also accepted here and normalized at the boundary (see
 * normalizeOrchestrationSection), the same string-or-richer tolerance
 * derive.ts gives `progressedSinceLastRun`. Anything else — a number,
 * an object without a sources array, non-string sources — fails.
 */
function isProvenance(v: unknown): boolean {
  if (typeof v === "string") return v.length > 0;
  if (typeof v !== "object" || v === null) return false;
  const p = v as { sources?: unknown; runId?: unknown };
  return (
    Array.isArray(p.sources) &&
    p.sources.every((x) => typeof x === "string") &&
    (p.runId === undefined || typeof p.runId === "string")
  );
}

/**
 * The first structural check an orchestration section fails, as a short
 * field name — or null when the section is valid (#1140: a present-but-
 * invalid section must be diagnosable, not silently "absent"). This is
 * the single source of truth behind isOrchestrationSection, so the
 * diagnosis can never drift from the gate itself.
 */
export function orchestrationSectionProblem(v: unknown): string | null {
  if (typeof v !== "object" || v === null) return "not an object";
  const s = v as Partial<OrchestrationSection>;
  if (s.version !== 1) return "version";
  if (!isIsoDate(s.generatedAt)) return "generatedAt";
  if (typeof s.runId !== "string") return "runId";
  if (!isProvenance(s.provenance)) return "provenance";
  if (typeof s.tiers !== "object" || s.tiers === null) return "tiers";
  if (!Array.isArray(s.tiers.needsDrew)) return "tiers.needsDrew";
  if (!Array.isArray(s.tiers.needsOrchestrator)) return "tiers.needsOrchestrator";
  if (!Array.isArray(s.dependencies)) return "dependencies";
  if (!Array.isArray(s.goals)) return "goals";
  if (!s.goals.every(isGoalMeasurement)) return "goals (per-goal shape)";
  if (!Array.isArray(s.actions)) return "actions";
  return null;
}

export function isOrchestrationSection(v: unknown): v is OrchestrationSection {
  return orchestrationSectionProblem(v) === null;
}

/**
 * A section older than this at the hourly cadence is stale (#1139,
 * #1148's threshold): still served, but every surface that renders its
 * numbers must say the data is old instead of presenting it as current.
 */
export const ORCHESTRATION_STALE_MS = 90 * 60_000;

/** Future-skew tolerance for generatedAt stamps (#1564; the aggregator-wide 60s convention ruled for #1541). */
export const GENERATED_AT_FUTURE_SKEW_MS = 60_000;

/**
 * The diagnosis behind the honesty surfaces (#1139/#1140): a section
 * the aggregator holds is in exactly one of these states. "invalid"
 * names the failing checks (from orchestrationSectionProblem — the
 * gate itself, so the diagnosis can never drift from the validation)
 * and is never collapsed into "absent": a publisher whose section is
 * being rejected has a bug to fix, not a run that never happened.
 */
export interface SectionDiagnosis {
  state: "live" | "absent" | "invalid" | "stale";
  generatedAt?: string;
  runId?: string;
  ageMs?: number;
  /** Names of the failing structural checks; never values. */
  failedChecks?: string[];
}

export function diagnoseOrchestrationSection(v: unknown, now = Date.now()): SectionDiagnosis {
  if (v === null || v === undefined) return { state: "absent" };
  const problem = orchestrationSectionProblem(v);
  if (problem !== null) return { state: "invalid", failedChecks: [problem] };
  const s = v as OrchestrationSection;
  const parsedAt = Date.parse(s.generatedAt);
  if (parsedAt - now > GENERATED_AT_FUTURE_SKEW_MS) {
    return { state: "invalid", failedChecks: ["generatedAt"], generatedAt: s.generatedAt, runId: s.runId };
  }
  const ageMs = Math.max(0, now - parsedAt);
  return {
    state: ageMs > ORCHESTRATION_STALE_MS ? "stale" : "live",
    generatedAt: s.generatedAt,
    runId: s.runId,
    ageMs,
  };
}

/**
 * A valid section with its provenance in the canonical object form:
 * a legacy plain-string provenance becomes `{ sources: [it] }`.
 * Returns null for anything isOrchestrationSection rejects. The
 * collector embeds this normalized form in the feed and the live
 * source serves it, so downstream consumers see exactly one shape.
 */
export function normalizeOrchestrationSection(v: unknown): OrchestrationSection | null {
  if (!isOrchestrationSection(v)) return null;
  const s = v as OrchestrationSection & { provenance: OrchestrationProvenance | string };
  if (typeof s.provenance !== "string") return s;
  return { ...s, provenance: { sources: [s.provenance] } };
}

/**
 * The first structural problem in a feed's GitHub Project pool section
 * (#2399, part 1 of #1122) — or null when the section is valid. The
 * per-item rule is isProjectPoolItem, the same predicate the collector
 * parses against and the live source filters with, so the feed check
 * and the collector can never drift: edge fields (parentRef,
 * blockedBy, blocks, epicRef) are absence-friendly but shape-strict
 * when present. A present-but-invalid section must be diagnosable, not
 * silently "absent" (#1140) — but a single malformed item still only
 * drops that item (see poolFromFeed), never the pool.
 */
export function projectPoolSectionProblem(v: unknown): string | null {
  if (v === undefined) return null;
  if (!Array.isArray(v)) return "not an array";
  for (let i = 0; i < v.length; i++) {
    if (!isProjectPoolItem(v[i])) return `item ${i}`;
  }
  return null;
}

/** A valid projects section: absent (undefined) or an array whose
 * every item passes the shared pool-item gate. */
export function isProjectPoolSection(v: unknown): v is ProjectPoolItem[] | undefined {
  return projectPoolSectionProblem(v) === null;
}

/** Fail-soft read of the pool from a parsed feed: absent or malformed
 * sections yield no items, and malformed items drop individually — the
 * single gate both live-source call sites share. */
export function poolFromFeed(feed: Partial<FleetFeed> | null | undefined): ProjectPoolItem[] {
  const raw = feed?.projects;
  if (raw === undefined) return [];
  if (!Array.isArray(raw)) return [];
  return raw.filter(isProjectPoolItem);
}

/** Whether the feed actually carries a projects section. The collector
 * omits the section entirely when its Project read fails, so presence
 * is the availability signal: an absent (or malformed) section means
 * the pool is unavailable, which coverage must not render as zero
 * items (#1220, PR #3309). A present array is available even when
 * individual malformed items drop in poolFromFeed. */
export function poolAvailableFromFeed(feed: Partial<FleetFeed> | null | undefined): boolean {
  return Array.isArray(feed?.projects);
}

/** Fail-soft read of the coverage census from a parsed feed: absent
 * or malformed sections yield null (the panel then renders backlog
 * counts as unavailable), malformed rows drop individually. */
export function coverageFromFeed(feed: Partial<FleetFeed> | null | undefined): RepoCensus[] | null {
  const raw = feed?.coverage;
  if (raw === undefined) return null;
  if (!Array.isArray(raw)) return null;
  const rows = raw.filter(isRepoCensus);
  return rows.length ? rows : null;
}

/** One quota window as the renderers dereference it (#2429): `limit`
 * is read arithmetically by windowPercent, `used` is rendered, and a
 * non-object element crashes both renderQuota and buildAttentionFlags. */
export function isQuotaWindow(v: unknown): v is QuotaWindow {
  if (!isRecord(v)) return false;
  return (
    typeof v.name === "string" &&
    (v.unit === "requests" || v.unit === "tokens" || v.unit === "usd" || v.unit === "messages" || v.unit === "other") &&
    (v.limit === null || isFiniteNumber(v.limit)) &&
    isFiniteNumber(v.used) &&
    (v.resetsAt === null || typeof v.resetsAt === "string") &&
    (v.percent === undefined || isFiniteNumber(v.percent))
  );
}

const QUOTA_STATES = ["healthy", "near", "walled", "error", "stale"];

/** One quota pool whose own shape is safe to hand to the renderers
 * (#2429): an object with a `windows` ARRAY — a non-array windows value
 * must never be iterated as a collection of windows (it fabricated
 * attention flags). Window ELEMENTS are filtered by sanitizeQuotaView,
 * so a pool survives one bad window. Credential rows, when present,
 * must at least be records holding a windows array; their elements are
 * filtered the same way. */
export function isQuotaPool(v: unknown): v is QuotaPool {
  if (!isRecord(v)) return false;
  if (typeof v.id !== "string" || typeof v.provider !== "string" || typeof v.plan !== "string") return false;
  if (typeof v.state !== "string" || !QUOTA_STATES.includes(v.state)) return false;
  if (!Array.isArray(v.windows)) return false;
  if (v.credentials !== undefined) {
    if (!Array.isArray(v.credentials)) return false;
    for (const row of v.credentials) {
      if (!isRecord(row) || !Array.isArray(row.windows)) return false;
    }
  }
  return true;
}

/** One historical burn segment as renderQuota maps it (#2429). */
export function isBurnSegment(v: unknown): v is BurnSegment {
  if (!isRecord(v)) return false;
  return (
    typeof v.provider === "string" &&
    typeof v.pool === "string" &&
    typeof v.from === "string" &&
    typeof v.to === "string" &&
    isFiniteNumber(v.tokens)
  );
}

/** One unmatched-charge entry as the live source dereferences it
 * (#2439): merchant and at are interpolated, amountUsd is formatted
 * with toFixed and used as a severity, so those three must be a
 * string, a finite number, and a string. normalized/source are not
 * gate-required: the consumer tolerates any source value and never
 * dereferences normalized. */
export function isUnmatchedCharge(v: unknown): v is UnmatchedCharge {
  if (!isRecord(v)) return false;
  return typeof v.merchant === "string" && isFiniteNumber(v.amountUsd) && typeof v.at === "string";
}

/** One charges-evidence row as the /quota charges strip renders it
 * (#2445): a record with string at/label/method/source and a
 * finite-number amountUsd (ChargeRow, model.ts). Built on the shared
 * validate.ts primitives, like the quota element validators above. */
export function isChargeRow(v: unknown): v is ChargeRow {
  if (!isRecord(v)) return false;
  return (
    typeof v.at === "string" &&
    typeof v.label === "string" &&
    isFiniteNumber(v.amountUsd) &&
    typeof v.method === "string" &&
    typeof v.source === "string"
  );
}

/** The choices block renderQuota dereferences (hasHeadroom et al.). */
function isQuotaChoices(v: unknown): boolean {
  if (!isRecord(v)) return false;
  return isStringArray(v.hasHeadroom) && isStringArray(v.burnBeforeReset) && isStringArray(v.doNotStart);
}

export interface QuotaSanitization {
  view: QuotaView;
  droppedPools: number;
  droppedWindows: number;
  droppedBurn: number;
}

/** Drop-and-count sanitization of a gate-passed quota section (#2429,
 * following poolFromFeed's per-element pattern, built on the shared
 * arrayOf primitive from #2577): malformed pools and burn segments
 * drop individually, malformed windows drop out of their pool, and the
 * well-formed remainder renders. The counts ride out on the result so
 * the caller surfaces them, never swallows them. */
export function sanitizeQuotaView(quota: QuotaView): QuotaSanitization {
  const pools = arrayOf(isQuotaPool)(quota.pools);
  let droppedWindows = 0;
  const keptPools: QuotaPool[] = (pools?.values ?? []).map((pool) => {
    const windows = arrayOf(isQuotaWindow)(pool.windows);
    droppedWindows += windows?.dropped ?? 0;
    let out: QuotaPool = { ...pool, windows: windows?.values ?? [] };
    if (pool.credentials !== undefined) {
      const rows = pool.credentials.map((row) => {
        const rowWindows = arrayOf(isQuotaWindow)(row.windows);
        droppedWindows += rowWindows?.dropped ?? 0;
        return { ...row, windows: rowWindows?.values ?? [] };
      });
      out = { ...out, credentials: rows };
    }
    return out;
  });
  const burn = arrayOf(isBurnSegment)(quota.burn);
  const droppedPools = pools?.dropped ?? 0;
  const droppedBurn = burn?.dropped ?? 0;
  // Guidance must describe the surviving usable pools only: the
  // retained choices were computed over the pre-sanitization pools,
  // and computeChoices() (quota-guidance.ts) gives no guidance for a
  // pool with no windows — so filter every list to kept pools that
  // still have windows. A pool whose data was discarded must not
  // stay recommended under hasHeadroom or burnBeforeReset.
  const usable = new Set(keptPools.filter((p) => p.windows.length > 0).map((p) => p.id));
  const choices = {
    hasHeadroom: quota.choices.hasHeadroom.filter((id) => usable.has(id)),
    burnBeforeReset: quota.choices.burnBeforeReset.filter((id) => usable.has(id)),
    doNotStart: quota.choices.doNotStart.filter((id) => usable.has(id)),
  };
  const view: QuotaView = { ...quota, pools: keptPools, burn: burn?.values ?? [], choices };
  // Surface the loss on the view itself (its consumers project the
  // view, e.g. /api/quota.json), not only on the feed wrapper.
  if (droppedPools + droppedWindows + droppedBurn > 0) {
    view.dropped = { pools: droppedPools, windows: droppedWindows, burn: droppedBurn };
  }
  return {
    view,
    droppedPools,
    droppedWindows,
    droppedBurn,
  };
}

export function isFleetFeed(v: unknown): v is FleetFeed {
  if (typeof v !== "object" || v === null) return false;
  const f = v as Partial<FleetFeed>;
  if (!(typeof f.quota === "object" && f.quota !== null)) return false;
  const q = f.quota as QuotaView;
  return (
    f.version === 1 &&
    // A generatedAt implausibly in the future is invalid, not
    // "maximally fresh" (#2242, via the shared primitive).
    isTimestampNotFuture(f.generatedAt) &&
    Array.isArray(q.pools) &&
    // burn and choices are mandatory in QuotaView and dereferenced
    // unguarded by renderQuota (#2429): a feed without them is an old
    // collector's shape and is rejected whole, like other invalid
    // feeds, rather than crashing /quota.
    Array.isArray(q.burn) &&
    isQuotaChoices(q.choices) &&
    Array.isArray(f.attention) &&
    // chargesAttention is optional, but a present non-array value is
    // a shape failure, not data (#2439): the live source maps it
    // unguarded, so — like burn/choices above — the feed is rejected
    // whole. Malformed ELEMENTS inside an array are loadFeed's
    // drop-and-count concern, not the gate's.
    (f.chargesAttention === undefined || Array.isArray(f.chargesAttention))
  );
}

export interface SubscriptionSanitization {
  /** Sanitized rows, or undefined when the section is absent (or was
   * dropped whole) — consumers already treat absence as an empty strip. */
  rows?: { name: string; plan: string; renewsAt: string | null }[];
  dropped: number;
}

/** One subscription row safe to hand to the renderers (#2446): the
 * strip maps esc(s.name)/esc(s.plan) and reads renewsAt unguarded, so
 * a null row or a non-string field 500s /quota and is served verbatim
 * by /api/quota.json. */
export function isSubscriptionRow(v: unknown): v is { name: string; plan: string; renewsAt: string | null } {
  if (!isRecord(v)) return false;
  return (
    typeof v.name === "string" && typeof v.plan === "string" && (typeof v.renewsAt === "string" || v.renewsAt === null)
  );
}

/** Drop malformed subscription rows at the loadFeed boundary (#2446,
 * the #2429 pattern): the section is optional and unvalidated by
 * isFleetFeed, so element-level damage is dropped here rather than
 * rejecting the whole feed for one bad row. A non-array section is
 * dropped whole (treated as absent, counted as one drop). */
export function sanitizeSubscriptions(v: unknown): SubscriptionSanitization {
  if (v === undefined) return { dropped: 0 };
  if (!Array.isArray(v)) return { dropped: 1 };
  const rows = v.filter(isSubscriptionRow);
  return { rows, dropped: v.length - rows.length };
}

export async function loadFeed(path: string): Promise<FleetFeed | null> {
  try {
    const parsed: unknown = JSON.parse(await readFile(path, "utf8"));
    if (!isFleetFeed(parsed)) return null;
    // The gate pins the section-level shape; element-level damage is
    // dropped here so every loadFeed consumer (live source, server)
    // sees only renderer-safe pools/windows (#2429). Drop counts are
    // attached to the feed so they are visible, not swallowed.
    const san = sanitizeQuotaView(parsed.quota);
    const meta = sanitizeFeedMetadata(parsed.meta);
    // Same drop-and-count treatment for chargesAttention elements
    // (#2439): the gate has already pinned the section to array-or-
    // absent, so arrayOf cannot return null here; malformed entries
    // drop individually and well-formed siblings still surface.
    const charges =
      parsed.chargesAttention === undefined ? undefined : arrayOf(isUnmatchedCharge)(parsed.chargesAttention);
    const subs = sanitizeSubscriptions(parsed.subscriptions);
    // Charges rows get the same drop-and-count treatment at this same
    // shared gate (#2445): the section is optional, so absence stays
    // absent; a non-array section drops whole (counted as one dropped
    // section) instead of silently emptying the charges strip or
    // crashing a consumer; malformed rows drop individually. The
    // count rides out on the feed when drops occur, never swallowed.
    let chargesPart: { charges?: ChargeRow[]; chargesDropped?: number } = {};
    if (parsed.charges !== undefined) {
      const rows = arrayOf(isChargeRow)(parsed.charges);
      if (rows === null) {
        chargesPart = { charges: [], chargesDropped: 1 };
      } else {
        chargesPart = { charges: rows.values, ...(rows.dropped > 0 ? { chargesDropped: rows.dropped } : {}) };
      }
    }
    return {
      ...parsed,
      meta,
      quota: san.view,
      quotaDropped: { pools: san.droppedPools, windows: san.droppedWindows, burn: san.droppedBurn },
      ...(charges ? { chargesAttention: charges.values } : {}),
      ...(charges && charges.dropped > 0 ? { chargesAttentionDropped: charges.dropped } : {}),
      subscriptions: subs.rows,
      ...(subs.dropped > 0 ? { subscriptionsDropped: subs.dropped } : {}),
      ...chargesPart,
    };
  } catch {
    return null;
  }
}

/** "3m old", "2h old", "fresh" — for an age in milliseconds. */
export function ageMsText(ageMs: number): string {
  if (!Number.isFinite(ageMs)) return "unknown";
  const ms = Math.max(0, ageMs);
  if (ms < 90_000) return "fresh";
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)}m old`;
  if (ms < 86_400_000) return `${(ms / 3_600_000).toFixed(1)}h old`;
  return `${Math.round(ms / 86_400_000)}d old`;
}

/** "3m old", "2h old", "fresh" — for provenance strings. */
export function feedAgeText(generatedAt: string, now = Date.now()): string {
  return ageMsText(now - Date.parse(generatedAt));
}
