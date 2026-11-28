// #4825 certified sources, rollup side: in-repository implementations
// of the cost-anomaly producer's three rollup-side auxiliary seams
// (cost-anomaly-producer.ts) — `closedChildren`, `busiestRoot`, and
// `sourceTotal`. The charges-side seams live in
// cost-anomaly-sources.ts; this module is their mirror for facts about
// rollup windows. Each seam is backed by a caller-supplied census: the
// caller's complete count for one scope/window, together with the
// source-coverage spans that certify the census.
//
// - Closure counts come from a per-scope-window closure census: the
//   complete count of child issues closed in that window for that
//   project or epic.
// - Busiest root comes from a per-epic-window root-session census:
//   one entry per root session with its token total for the window,
//   sub-sessions already folded into their root by the census (never
//   listed as competing roots). The certified value is the maximum
//   root total in the census.
// - Source-population totals come from a per-window census of the
//   total source population before attribution, keyed by unit and
//   pool — never a sum of overlapping project/epic rollup rows
//   (COST-ANOMALY.md).
//
// Certification rule (the spend-rollups coverage rule, applied to the
// rollup's own window): a record is `complete` only when an ok
// coverage span covers the whole window as instants, and the window
// is the canonical calendar window for its kind and start. Anything
// else is `partial`, and partial suppresses the anomaly in the
// detector — an absent census, a filtered view, or a stale source
// never certifies a zero (#4825 rules; COST-ANOMALY.md). A census
// with no entry answering the queried rollup is uncertified: the seam
// returns null, which also suppresses. Conflicting census entries
// (same key, different values) certify nothing: the seam returns null
// rather than picking one — the intake does not reconcile conflicting
// raw observations. For the busiest root, the maximum is certified
// only when every root entry in the census is whole-window certified:
// a partially covered root set could conceal a larger root.
//
// Each returned record is stamped with the queried rollup's window,
// scope, and attribution version: the census certifies the window's
// facts and the record is issued as the answer for that rollup, which
// is the pairing the detector's same-window/same-scope checks require.
//
// Pure and deterministic, like the modules it feeds: no I/O, no clock
// reads; the same inputs always produce the same records. Production
// rollup ingestion and activation of the tick remain later slices —
// see COST-ANOMALY.md.
import type { BusiestRootRecord, ClosedChildrenRecord, SourceTotalRecord } from "./cost-anomaly-producer.ts";
import {
  windowRange,
  type RollupCoverage,
  type RollupUnit,
  type RollupWindowKind,
  type SourceCoverageSpan,
  type SpendRollup,
} from "./spend-rollups.ts";

/** One scope's complete child-closure census for one rollup window. */
export interface ClosureCensusEntry {
  scopeKind: SpendRollup["scopeKind"];
  scopeId: string;
  windowKind: RollupWindowKind;
  /** Inclusive window start (UTC, ISO). */
  windowStart: string;
  /** Exclusive window end (UTC, ISO). */
  windowEnd: string;
  /** Child issues closed in this window (whole census, not a sample). */
  closedChildren: number;
  /** Source-coverage spans certifying this census entry. */
  coverage: readonly SourceCoverageSpan[];
}

/** One root session's token census for one epic rollup window and
 * pool. Sub-sessions are folded into their root by the census
 * producer; each root appears as census entries under its own id. */
export interface RootSessionCensusEntry {
  /** Epic ref (the rollup's scopeId). */
  scopeId: string;
  windowKind: RollupWindowKind;
  windowStart: string;
  windowEnd: string;
  pool: string;
  rootSessionId: string;
  /** Tokens consumed by this root session in this window and pool. */
  tokens: number;
  /** Source-coverage spans certifying this census entry. */
  coverage: readonly SourceCoverageSpan[];
}

/** The total source population for one window, before attribution. */
export interface SourcePopulationCensusEntry {
  windowKind: RollupWindowKind;
  windowStart: string;
  windowEnd: string;
  unit: RollupUnit;
  /** Pool for tokens / share_pct measures; null for usd. */
  pool: string | null;
  /** Total source-population value for the window and measure. */
  value: number;
  /** Source-coverage spans certifying this census entry. */
  coverage: readonly SourceCoverageSpan[];
}

/** `complete` only when the window is the canonical calendar window
 * for its kind/start and an ok span covers it whole, compared as
 * instants (the spend-rollups coverageFor rule). */
function certifiedCoverage(
  windowKind: RollupWindowKind,
  windowStart: string,
  windowEnd: string,
  spans: readonly SourceCoverageSpan[],
): RollupCoverage {
  let range: { start: string; end: string };
  try {
    range = windowRange(windowKind, windowStart);
  } catch {
    return "partial";
  }
  if (range.start !== windowStart || range.end !== windowEnd) return "partial";
  const ws = Date.parse(windowStart);
  const we = Date.parse(windowEnd);
  const full = spans.some((s) => {
    if (s.status !== "ok") return false;
    const from = Date.parse(s.from);
    const to = Date.parse(s.to);
    return Number.isFinite(from) && Number.isFinite(to) && from <= ws && to >= we;
  });
  return full ? "complete" : "partial";
}

function sameWindowFields(
  entry: { windowKind: RollupWindowKind; windowStart: string; windowEnd: string },
  spend: SpendRollup,
): boolean {
  return entry.windowKind === spend.windowKind && entry.windowStart === spend.windowStart && entry.windowEnd === spend.windowEnd;
}

/**
 * Producer seam: certified child-closure counts from a closure
 * census. Returns null — uncertified, suppressing the anomaly — when
 * the census has no entry for the rollup's scope and window, when
 * its entries conflict, or when an entry's count is not a finite
 * non-negative integer. A present, consistent entry is returned with
 * its count as counted and its coverage certified from the entry's
 * own spans: a complete census that counted real closures suppresses
 * by its count, never by relabelling.
 */
export function certifiedClosedChildren(
  census: readonly ClosureCensusEntry[],
): (spend: SpendRollup) => ClosedChildrenRecord | null {
  return (spend) => {
    const entries = census.filter(
      (e) => e.scopeKind === spend.scopeKind && e.scopeId === spend.scopeId && sameWindowFields(e, spend),
    );
    if (entries.length === 0) return null;
    const first = entries[0]!;
    for (const entry of entries) {
      if (entry.closedChildren !== first.closedChildren) return null;
      if (!Number.isInteger(entry.closedChildren) || entry.closedChildren < 0) return null;
    }
    return {
      windowKind: spend.windowKind,
      windowStart: spend.windowStart,
      windowEnd: spend.windowEnd,
      coverage: certifiedCoverage(spend.windowKind, spend.windowStart, spend.windowEnd, first.coverage),
      attributionVersion: spend.attributionVersion,
      scopeKind: spend.scopeKind,
      scopeId: spend.scopeId,
      count: first.closedChildren,
    };
  };
}

/**
 * Producer seam: certified busiest root-session token counts from a
 * root-session census. Only epic token rollups have a busiest root;
 * any other rollup is uncertified (null). Returns null when the
 * census has no entry for the rollup's epic, window, and pool, when
 * two entries give one root conflicting totals, or when an entry's
 * tokens are not a finite non-negative number. Identical duplicate
 * entries for one root collapse — a root is counted once, at its
 * folded total. The record's coverage is `complete` only when every
 * root entry in the census is whole-window certified.
 */
export function certifiedBusiestRoot(
  census: readonly RootSessionCensusEntry[],
): (spend: SpendRollup) => BusiestRootRecord | null {
  return (spend) => {
    if (spend.scopeKind !== "epic" || spend.unit !== "tokens" || spend.pool === null) return null;
    const entries = census.filter(
      (e) => e.scopeId === spend.scopeId && e.pool === spend.pool && sameWindowFields(e, spend),
    );
    if (entries.length === 0) return null;
    const byRoot = new Map<string, number>();
    for (const entry of entries) {
      if (!entry.rootSessionId.trim()) return null;
      if (!Number.isFinite(entry.tokens) || entry.tokens < 0) return null;
      const prev = byRoot.get(entry.rootSessionId);
      if (prev !== undefined && prev !== entry.tokens) return null;
      byRoot.set(entry.rootSessionId, entry.tokens);
    }
    const allCertified = entries.every(
      (e) => certifiedCoverage(spend.windowKind, spend.windowStart, spend.windowEnd, e.coverage) === "complete",
    );
    return {
      windowKind: spend.windowKind,
      windowStart: spend.windowStart,
      windowEnd: spend.windowEnd,
      coverage: allCertified ? "complete" : "partial",
      attributionVersion: spend.attributionVersion,
      scopeKind: spend.scopeKind,
      scopeId: spend.scopeId,
      pool: spend.pool,
      tokens: Math.max(...byRoot.values()),
    };
  };
}

/**
 * Producer seam: certified source-population totals from a
 * population census, matched on the rollup's window, unit, and pool.
 * Returns null — uncertified, suppressing the anomaly — when the
 * census has no entry for that key, when its entries conflict, or
 * when an entry's value is not a finite non-negative number. A
 * present, consistent entry is returned with its value as counted
 * and its coverage certified from the entry's own spans.
 */
export function certifiedSourceTotal(
  census: readonly SourcePopulationCensusEntry[],
): (rollup: SpendRollup) => SourceTotalRecord | null {
  return (rollup) => {
    const entries = census.filter(
      (e) => e.unit === rollup.unit && e.pool === rollup.pool && sameWindowFields(e, rollup),
    );
    if (entries.length === 0) return null;
    const first = entries[0]!;
    for (const entry of entries) {
      if (entry.value !== first.value) return null;
      if (!Number.isFinite(entry.value) || entry.value < 0) return null;
    }
    return {
      windowKind: rollup.windowKind,
      windowStart: rollup.windowStart,
      windowEnd: rollup.windowEnd,
      coverage: certifiedCoverage(rollup.windowKind, rollup.windowStart, rollup.windowEnd, first.coverage),
      attributionVersion: rollup.attributionVersion,
      unit: rollup.unit,
      pool: rollup.pool,
      value: first.value,
    };
  };
}
