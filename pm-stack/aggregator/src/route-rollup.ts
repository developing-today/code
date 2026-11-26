// Client for the OpenCode server's route-attempt rollup read
// (GET /api/route/attempts/rollup, developing-today-forks/opencode#2795).
//
// The /quota analytics used to open opencode.db and run its own SQL over
// the route_attempt ledger. The server now owns that ledger's read
// surface, so the aggregator asks it instead. The server never serves a
// credential id: it serves `credentialKey`, the first 16 hex characters
// of the SHA-256 of the id. The aggregator already holds the ids it
// attributes pools by (quota-current.json), so it hashes them the same
// way and matches keys.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import type { OpenCodeTarget } from "./sources/opencode.ts";

export type RollupBucket = "none" | "day" | "hour" | "hourOfDay" | "dayOfWeek";
export type RollupDimension = "provider" | "model" | "variant" | "credential";

export interface RollupRow {
  bucket: string | number | null;
  providerID?: string;
  modelID?: string;
  variant?: string | null;
  credentialKey?: string | null;
  attempts: number;
  successes: number;
  tokensInput: number;
  tokensOutput: number;
  tokensReasoning: number;
  tokensCacheRead: number;
  tokensCacheWrite: number;
  /** input + output + cache read + cache write over rows that record all four. */
  tokensCombined: number;
  costUsd: number;
  costRows: number;
  credentialRows: number;
  first: number | null;
  last: number | null;
}

export interface RollupQuery {
  since: number;
  /** Exclusive. */
  until?: number;
  bucket?: RollupBucket;
  group?: RollupDimension[];
}

/** The digest the server serves in place of a credential id. Must match
 * ModelRouteLog.credentialKey in the OpenCode server. */
export function credentialKey(credentialId: string): string {
  return createHash("sha256").update(credentialId).digest("hex").slice(0, 16);
}

/** The standing OpenCode server's url + password, from the state file it
 * rewrites on every restart. Null when the file is missing or partial. */
export function readRouteApiTarget(stateFile: string): OpenCodeTarget | null {
  try {
    const state = JSON.parse(readFileSync(stateFile, "utf8")) as { url?: string; password?: string };
    if (state.url && state.password) return { baseUrl: state.url, password: state.password };
  } catch {
    // fail-soft: no target means the analytics section says it is unavailable
  }
  return null;
}

/** One rollup read. Buckets use the server's local clock, which is the
 * laptop's (the aggregator and the server run on the same host), so day
 * and hour buckets line up with the aggregator's own local days. Throws on
 * any failure so the caller's refresh keeps the stale copy. */
export async function fetchRollup(target: OpenCodeTarget, q: RollupQuery, timeoutMs = 30_000): Promise<RollupRow[]> {
  const params = new URLSearchParams({ since: String(q.since), clock: "local" });
  if (q.until !== undefined) params.set("until", String(q.until));
  if (q.bucket) params.set("bucket", q.bucket);
  if (q.group?.length) params.set("group", q.group.join(","));
  const res = await fetch(`${target.baseUrl}/api/route/attempts/rollup?${params}`, {
    headers: { Authorization: `Basic ${Buffer.from(`opencode:${target.password}`).toString("base64")}` },
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!res.ok) throw new Error(`route rollup: HTTP ${res.status}`);
  const body = (await res.json()) as { data?: unknown };
  if (!Array.isArray(body.data)) throw new Error("route rollup: response has no data array");
  return body.data as RollupRow[];
}
