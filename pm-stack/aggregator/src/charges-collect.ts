// charges-collect (#1057): the collector-side glue around the pure
// matcher in charges-match.ts. The Muse-side producer
// (scripts/produce-charge-candidates.ts) writes the period's Plaid +
// Gmail charge candidates to a state file; the daily collector reads
// that file here, runs matchCharges, merges the auto-matched rows
// with the hand-fed registry/quota-view rows (auto wins on a
// duplicate — it is the fresher, structured read), and surfaces
// unmatched SaaS/AI-looking charges in the hub attention strip
// exactly once via a persisted surfaced-keys state file.
//
// Everything here is pure except the state the collector itself
// passes in/out; file IO lives in collect-fleet-feed.ts, following
// the same split as charges-match.ts (decisions here, state there).

import {
  DEDUPE_WINDOW_DAYS,
  normalizeMerchantName,
  type ChargeCandidate,
  type UnmatchedCharge,
} from "./charges-match.ts";
import type { ChargeRow } from "./model.ts";

/**
 * Structural validation of the producer's candidates file. Accepts
 * the canonical `{ candidates: [...] }` object or a bare array.
 * Malformed entries are dropped, never guessed at: a candidate needs
 * a known source, an ISO date, a merchant string, and a finite
 * amount. Returns [] for anything unusable (the collector logs and
 * falls back to hand-fed rows only).
 */
export function parseChargeCandidates(raw: unknown): ChargeCandidate[] {
  const list: unknown = Array.isArray(raw)
    ? raw
    : typeof raw === "object" && raw !== null && Array.isArray((raw as { candidates?: unknown }).candidates)
      ? (raw as { candidates: unknown[] }).candidates
      : null;
  if (!Array.isArray(list)) return [];
  const out: ChargeCandidate[] = [];
  for (const entry of list) {
    if (typeof entry !== "object" || entry === null) continue;
    const c = entry as Partial<ChargeCandidate>;
    if (c.source !== "plaid" && c.source !== "gmail") continue;
    if (typeof c.at !== "string" || !/^\d{4}-\d{2}-\d{2}/.test(c.at)) continue;
    if (typeof c.merchant !== "string" || !c.merchant.trim()) continue;
    if (typeof c.amountUsd !== "number" || !Number.isFinite(c.amountUsd)) continue;
    out.push({
      source: c.source,
      at: c.at.slice(0, 10),
      merchant: c.merchant,
      amountUsd: c.amountUsd,
      ...(typeof c.method === "string" && c.method ? { method: c.method } : {}),
    });
  }
  return out;
}

function squash(normalized: string): string {
  return normalized.replace(/\s+/g, "");
}

function daysBetween(a: string, b: string): number {
  const ms = Math.abs(Date.parse(a) - Date.parse(b));
  return Math.floor(ms / 86_400_000);
}

/**
 * Does a hand-fed row describe the same charge as an auto-matched
 * row? Same amount, dates within the matcher's dedupe window, and
 * merchant/label token strings that contain one another once
 * squashed ("Meta PAYG spill" vs "METAPAY" both squash around
 * METAPAY). Conservative: a hand-fed row is only dropped when an
 * auto row plausibly IS that charge.
 */
function sameCharge(handFed: ChargeRow, auto: ChargeRow): boolean {
  if (handFed.amountUsd !== auto.amountUsd) return false;
  if (daysBetween(handFed.at, auto.at) > DEDUPE_WINDOW_DAYS) return false;
  const a = squash(normalizeMerchantName(handFed.label));
  const b = squash(normalizeMerchantName(auto.label));
  if (!a || !b) return false;
  return a.includes(b) || b.includes(a);
}

/**
 * The feed's charges section: auto-matched rows first (source of
 * record), plus any hand-fed rows the auto read did not already
 * cover, date-ordered like matchCharges output. With no auto rows
 * this is exactly the hand-fed list, sorted — the pre-#1057 feed.
 */
export function mergeChargeRows(autoRows: ChargeRow[], handFedRows: ChargeRow[]): ChargeRow[] {
  const kept = handFedRows.filter((row) => !autoRows.some((auto) => sameCharge(row, auto)));
  return [...autoRows, ...kept].sort((x, y) => x.at.localeCompare(y.at) || x.label.localeCompare(y.label));
}

/** Stable identity of one unmatched charge for the surfaced-once state. */
export function unmatchedKey(u: UnmatchedCharge): string {
  return `${u.normalized}|${u.amountUsd.toFixed(2)}|${u.at}`;
}

/**
 * Split unmatched charges into those never surfaced before (they go
 * to the attention strip this run) and the full key set to persist.
 * A charge surfaces exactly once: once its key is in the persisted
 * set it never surfaces again, even though it stays in the charges
 * rows as evidence.
 */
export function filterNewUnmatched(
  unmatched: UnmatchedCharge[],
  surfaced: ReadonlySet<string>,
): { fresh: UnmatchedCharge[]; keys: string[] } {
  const keys = new Set(surfaced);
  const fresh: UnmatchedCharge[] = [];
  for (const u of unmatched) {
    const key = unmatchedKey(u);
    if (!keys.has(key)) {
      keys.add(key);
      fresh.push(u);
    }
  }
  return { fresh, keys: [...keys].sort() };
}
