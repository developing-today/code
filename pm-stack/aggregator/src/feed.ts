// The fleet feed: a JSON file produced on the sandbox host by
// scripts/collect-fleet-feed.ts and pushed to the laptop (the sandbox
// OpenCode server binds its own loopback, and the quota sources of
// record live on the sandbox). The aggregator renders the feed as-is;
// a stale feed keeps rendering with its age stated in provenance.
import { readFile } from "node:fs/promises";
import type { QuotaView, SessionFeedEntry } from "./model.ts";

export interface FleetFeed {
  version: 1;
  /** ISO timestamp of collection on the sandbox. */
  generatedAt: string;
  quota: QuotaView;
  /** Sandbox-server sessions, pre-classification (render classifies). */
  attention: SessionFeedEntry[];
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
