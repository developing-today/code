// The babysitter's pool-state.json, projected from the quota
// gatherer's quota-current.json (#1338, epic #1334). The claimant's
// babysitter (pm-stack/opencode-claimant/babysitter.py) reads pool
// wall state as an input file in its state dir —
//
//   {"pools": {"<providerID>": {"walled": bool,
//                               "percent": number|null,
//                               "state": str|null}}}
//
// — and this module is the writer side of that contract: a pure
// projection over the parsed quota-current file, so the semantics
// are pinned by tests and gather-quota.ts only does the file IO.
//
// Keying: pools are keyed by the entry's providerId, which the
// gatherer already canonicalizes to the OpenCode config provider id
// (quota-gather-map.ts canonicalProviderId) — the same id an
// OpenCode session names as model.providerID, which is what the
// babysitter looks up. The aggregator's own pool/service ids are a
// separate contract and never appear here.
//
// Semantics, mirroring the house rules of the two consumers'
// neighbourhoods:
// - A wall is asserted ONLY from a fresh "ok" entry (freshness as
//   quota-current.ts defines it: measuredAt within
//   QUOTA_CURRENT_FRESH_MS, future skew bounded by
//   QUOTA_CURRENT_FUTURE_SKEW_MS). A stale or errored entry can
//   retain a last-known window at its limit after the quota has
//   reset (quota-exhaustion.ts's stale_source rule), so it never
//   asserts a wall and never contributes a percent: stale surfaces
//   as state, never as numbers.
// - Walled is derive.ts poolState's rule: any window of a fresh ok
//   entry at >= 100% of its limit. Several entries can share a
//   providerId (one per credential): any fresh ok credential at
//   its limit walls the pool — the babysitter cannot tell which
//   credential a session rides, and a missed wall misreads quota
//   silence as a stall, the exact error the verdict exists to
//   avoid.
// - percent is the highest pctOfLimit across the fresh ok
//   entries' windows (the most-exhausted window is the binding
//   one), or null when no fresh ok entry reports one.
// - state is "walled" / "near" (>= NEAR_PCT, derive.ts) /
//   "healthy" when a fresh ok entry decides it; otherwise the
//   newest entry's state, with a stale "ok" reported as "stale".
import { NEAR_PCT } from "./derive.ts";
import { QUOTA_CURRENT_FRESH_MS, QUOTA_CURRENT_FUTURE_SKEW_MS } from "./quota-current.ts";
import type { QuotaCurrentEntry, QuotaCurrentFile } from "./model.ts";

export interface PoolStateEntry {
  walled: boolean;
  percent: number | null;
  state: string | null;
}

export interface PoolStateFile {
  version: 1;
  generatedAt: string;
  source: "gather-quota";
  pools: Record<string, PoolStateEntry>;
}

/** quota-current.ts's freshness test, applied to one entry. */
function isFreshOk(entry: QuotaCurrentEntry, nowMs: number): boolean {
  if (entry.state !== "ok") return false;
  const t = Date.parse(entry.measuredAt);
  return Number.isFinite(t) && t - nowMs <= QUOTA_CURRENT_FUTURE_SKEW_MS && nowMs - t <= QUOTA_CURRENT_FRESH_MS;
}

/** Project a parsed quota-current file into the babysitter's pool-state shape. */
export function projectPoolState(file: QuotaCurrentFile, now = new Date()): PoolStateFile {
  const nowMs = now.getTime();
  const byProvider = new Map<string, QuotaCurrentEntry[]>();
  for (const entry of file.providers) {
    const list = byProvider.get(entry.providerId) ?? [];
    list.push(entry);
    byProvider.set(entry.providerId, list);
  }

  const pools: Record<string, PoolStateEntry> = {};
  for (const [providerId, entries] of byProvider) {
    const freshOk = entries.filter((e) => isFreshOk(e, nowMs));
    if (freshOk.length > 0) {
      let percent: number | null = null;
      let walled = false;
      for (const entry of freshOk) {
        for (const w of entry.windows) {
          if (typeof w.pctOfLimit === "number" && Number.isFinite(w.pctOfLimit)) {
            if (percent === null || w.pctOfLimit > percent) percent = w.pctOfLimit;
            if (w.pctOfLimit >= 100) walled = true;
          }
        }
      }
      pools[providerId] = {
        walled,
        percent,
        state: walled ? "walled" : percent !== null && percent >= NEAR_PCT ? "near" : "healthy",
      };
      continue;
    }
    // No fresh ok entry: the newest entry's state rides, with a
    // stale "ok" named as what it is. No wall, no percent — the
    // retained windows assert nothing about the current quota.
    const newest = entries.reduce((a, b) => (Date.parse(b.measuredAt) > Date.parse(a.measuredAt) ? b : a));
    pools[providerId] = {
      walled: false,
      percent: null,
      state: newest.state === "ok" ? "stale" : newest.state,
    };
  }

  return { version: 1, generatedAt: file.generatedAt, source: "gather-quota", pools };
}
