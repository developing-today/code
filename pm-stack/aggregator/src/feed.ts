// The fleet feed: a JSON file produced on the sandbox host by
// scripts/collect-fleet-feed.ts and pushed to the laptop (the sandbox
// OpenCode server binds its own loopback, and the quota sources of
// record live on the sandbox). The aggregator renders the feed as-is;
// a stale feed keeps rendering with its age stated in provenance.
import { readFile } from "node:fs/promises";
import type {
  ChargeRow,
  DnsSection,
  GoalMeasurement,
  JulesSummary,
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

export function isOrchestrationSection(v: unknown): v is OrchestrationSection {
  if (typeof v !== "object" || v === null) return false;
  const s = v as Partial<OrchestrationSection>;
  return (
    s.version === 1 &&
    typeof s.generatedAt === "string" &&
    typeof s.runId === "string" &&
    typeof s.provenance === "string" &&
    typeof s.tiers === "object" &&
    s.tiers !== null &&
    Array.isArray(s.tiers.needsDrew) &&
    Array.isArray(s.tiers.needsOrchestrator) &&
    Array.isArray(s.dependencies) &&
    Array.isArray(s.goals) &&
    s.goals.every(isGoalMeasurement) &&
    Array.isArray(s.actions)
  );
}

export function isFleetFeed(v: unknown): v is FleetFeed {
  if (typeof v !== "object" || v === null) return false;
  const f = v as Partial<FleetFeed>;
  return (
    f.version === 1 &&
    typeof f.generatedAt === "string" &&
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

/** "3m old", "2h old", "fresh" — for provenance strings. */
export function feedAgeText(generatedAt: string, now = Date.now()): string {
  const ageMs = Math.max(0, now - Date.parse(generatedAt));
  if (ageMs < 90_000) return "fresh";
  if (ageMs < 3_600_000) return `${Math.round(ageMs / 60_000)}m old`;
  if (ageMs < 86_400_000) return `${(ageMs / 3_600_000).toFixed(1)}h old`;
  return `${Math.round(ageMs / 86_400_000)}d old`;
}
