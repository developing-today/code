// The fleet feed: a JSON file produced on the sandbox host by
// scripts/collect-fleet-feed.ts and pushed to the laptop (the sandbox
// OpenCode server binds its own loopback, and the quota sources of
// record live on the sandbox). The aggregator renders the feed as-is;
// a stale feed keeps rendering with its age stated in provenance.
import { readFile } from "node:fs/promises";
import type { UnmatchedCharge } from "./charges-match.ts";
import type { CheckoutDrift } from "./checkout-drift.ts";
import { isProjectPoolItem } from "./projects.ts";
import type {
  AaFeedSection,
  ChargeRow,
  DnsSection,
  GoalMeasurement,
  JulesSummary,
  OrchestrationProvenance,
  OrchestrationSection,
  PaygLine,
  ProjectPoolItem,
  QuotaView,
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
  /** AA cost explorer section: the ainventory (AA workbook) data read
   * by the collector from Supabase, plus pool/catalog coverage computed
   * against the repo's opencode config. Present-but-absent
   * ({status:"absent", reason}) when Supabase was unreachable — the
   * section degrades, the feed never fails over it. */
  aa?: AaFeedSection;
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
  if (typeof s.generatedAt !== "string" || !Number.isFinite(Date.parse(s.generatedAt))) return "generatedAt";
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

export function isFleetFeed(v: unknown): v is FleetFeed {
  if (typeof v !== "object" || v === null) return false;
  const f = v as Partial<FleetFeed>;
  return (
    f.version === 1 &&
    typeof f.generatedAt === "string" &&
    Number.isFinite(Date.parse(f.generatedAt)) &&
    typeof f.quota === "object" &&
    f.quota !== null &&
    Array.isArray((f.quota as QuotaView).pools) &&
    Array.isArray(f.attention)
  );
}

export async function loadFeed(path: string): Promise<FleetFeed | null> {
  try {
    const parsed: unknown = JSON.parse(await readFile(path, "utf8"));
    return isFleetFeed(parsed) ? parsed : null;
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
