// The fleet feed: a JSON file produced on the sandbox host by
// scripts/collect-fleet-feed.ts and pushed to the laptop (the sandbox
// OpenCode server binds its own loopback, and the quota sources of
// record live on the sandbox). The aggregator renders the feed as-is;
// a stale feed keeps rendering with its age stated in provenance.
import { readFile } from "node:fs/promises";
import type { DnsSection, JulesSummary, OrchestrationSection, QuotaView, SessionFeedEntry, SessionRecord } from "./model.ts";

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
}

/**
 * Minimal structural check for an orchestration section, shared by the
 * collector (which embeds the orchestrator's file fail-soft) and the
 * live source (which serves it). Absent or malformed = "not measured".
 */
export function isOrchestrationSection(v: unknown): v is OrchestrationSection {
  if (typeof v !== "object" || v === null) return false;
  const s = v as Partial<OrchestrationSection>;
  return (
    s.version === 1 &&
    typeof s.generatedAt === "string" &&
    typeof s.runId === "string" &&
    typeof s.tiers === "object" &&
    s.tiers !== null &&
    Array.isArray(s.tiers.needsDrew) &&
    Array.isArray(s.tiers.needsOrchestrator) &&
    Array.isArray(s.dependencies) &&
    Array.isArray(s.goals) &&
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
