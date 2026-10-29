// charges-collect (#1057): the collector-side half of the charges
// auto-matcher. The pure matching core lives in src/charges-match.ts;
// this module owns everything stateful or parsing-shaped around it:
//
//   - the charges export contract: a Muse-side producer (an agent
//     with the Plaid and Gmail connectors, which the collector process
//     cannot reach in-process) writes ChargeCandidate[] JSON to
//     ~/atlas/charges-export/latest.json — the file-export convention
//     gather-quota.ts family 2 established (#1301). The collector
//     ingests that file; an absent or malformed export changes
//     nothing (the prose extractor stays in charge).
//   - building the matcher's service refs from the hub's own SaaS
//     service list, so a charge matches the services the hub tracks.
//   - the persisted "already surfaced" state for unmatched SaaS
//     charges: an unmatched charge joins the feed's chargesUnmatched
//     section exactly once; the collector records its key in a state
//     file and never surfaces the same charge again. Re-running the
//     collector for the same period is a no-op for surfacing.
//
// No balances or debt data ever enter here: candidates are charge
// evidence only (date, merchant, amount, payment-method label).

import type { ChargeCandidate, ChargeServiceRef, UnmatchedCharge } from "./charges-match.ts";
import type { SaasService } from "./model.ts";

/**
 * Parse a charges export document into candidates. Returns null when
 * the document is not JSON at all (the caller keeps its fallback);
 * a JSON document with malformed entries yields the valid subset —
 * one bad row from a producer must not sink the whole period.
 */
export function parseChargeCandidatesExport(text: string): ChargeCandidate[] | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return null;
  }
  const list = Array.isArray(parsed) ? parsed : (parsed as { candidates?: unknown })?.candidates;
  if (!Array.isArray(list)) return null;
  const out: ChargeCandidate[] = [];
  for (const entry of list) {
    if (typeof entry !== "object" || entry === null) continue;
    const e = entry as Record<string, unknown>;
    if (e.source !== "plaid" && e.source !== "gmail") continue;
    if (typeof e.at !== "string" || !Number.isFinite(Date.parse(e.at))) continue;
    if (typeof e.merchant !== "string" || !e.merchant.trim()) continue;
    if (typeof e.amountUsd !== "number" || !Number.isFinite(e.amountUsd)) continue;
    out.push({
      source: e.source,
      at: e.at,
      merchant: e.merchant,
      amountUsd: e.amountUsd,
      ...(typeof e.method === "string" && e.method.trim() ? { method: e.method } : {}),
    });
  }
  return out;
}

/** The matcher's service refs: the hub's tracked SaaS services by name. */
export function serviceRefsFromSaas(saas: SaasService[]): ChargeServiceRef[] {
  return saas.map((s) => ({ name: s.name }));
}

/**
 * Identity of one unmatched charge for surfacing purposes: the same
 * charge re-observed next period (a new posting date) is a new
 * charge and may surface again; the same posting re-collected is not.
 */
export function unmatchedChargeKey(u: UnmatchedCharge): string {
  return `${u.normalized}|${u.amountUsd.toFixed(2)}|${u.at}`;
}

export interface SurfacingPlan {
  /** Unmatched charges not present in the persisted state. */
  fresh: UnmatchedCharge[];
  /** The full key set to persist (prior keys plus the fresh ones). */
  nextKeys: string[];
}

/**
 * Decide which unmatched charges surface this run. Pure: the
 * collector loads the persisted keys, plans, publishes `fresh`, and
 * writes `nextKeys` back — in that order, so a crash between publish
 * and persist can re-surface (visible, harmless) but never lose a
 * surfacing silently.
 */
export function planUnmatchedSurfacing(unmatched: UnmatchedCharge[], surfacedKeys: string[]): SurfacingPlan {
  const seen = new Set(surfacedKeys);
  const fresh: UnmatchedCharge[] = [];
  for (const u of unmatched) {
    const key = unmatchedChargeKey(u);
    if (seen.has(key)) continue;
    seen.add(key);
    fresh.push(u);
  }
  return { fresh, nextKeys: [...seen] };
}

/** Parse the persisted surfaced-keys state file; anything odd = empty state. */
export function parseSurfacedState(text: string): string[] {
  try {
    const parsed: unknown = JSON.parse(text);
    const keys = (parsed as { surfacedKeys?: unknown })?.surfacedKeys;
    return Array.isArray(keys) ? keys.filter((k): k is string => typeof k === "string") : [];
  } catch {
    return [];
  }
}
