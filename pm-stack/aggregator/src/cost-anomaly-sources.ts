// #4825 certified sources, charges side: the first in-repository
// implementations of the cost-anomaly producer's auxiliary seams
// (cost-anomaly-producer.ts). This module supplies the two charges-side
// seams — `monthlyCharges` and `serviceActivity` — from data the
// repository already produces:
//
// - Monthly matched-charge totals come from the charges pipeline's own
//   output (#1057), reduced by `matchedChargesFrom` (spend-dollars.ts):
//   matched, deduplicated, real charges only. This module never builds
//   a money path and never re-decides what a charge is (spend-dollars
//   rule (1)); it only totals the pipeline's rows, in integer cents,
//   per service and calendar month.
// - Service activity comes from a per-service-month activity census
//   supplied by the caller: the caller's complete count of sessions
//   and quota activity for that service in that month, together with
//   the source-coverage spans that certify the census.
//
// Certification rule (the spend-rollups coverage rule, applied to
// calendar months): a total or census record is `complete` only when
// an ok coverage span covers the whole month as instants. Anything
// else is `partial`, and partial suppresses the anomaly in the
// detector — an absent census, a filtered view, or a stale source
// never certifies a zero (#4825 rules; COST-ANOMALY.md). A census with
// no entry for a charge's service and month is uncertified: the
// activity seam returns null, which also suppresses. Conflicting
// census entries for the same service and month (same key, different
// counts) certify nothing: the seam returns null rather than picking
// one — the intake does not reconcile conflicting raw observations.
//
// Pure and deterministic, like the modules it feeds: no I/O, no clock
// reads; the same inputs always produce the same records. The
// rollup-side seams (closure counts, busiest root session,
// source-population totals) and production activation remain later
// slices — see COST-ANOMALY.md.
import type { MonthlyChargeTotal, ServiceActivityRecord } from "./cost-anomaly-producer.ts";
import { monthOf, type MatchedCharge } from "./spend-dollars.ts";
import { windowRange, type RollupCoverage, type SourceCoverageSpan } from "./spend-rollups.ts";

/** One service's complete activity census for one calendar month. */
export interface ServiceActivityCensusEntry {
  service: string;
  /** Calendar month, YYYY-MM. */
  month: string;
  /** Sessions on this service in this month (whole census, not a sample). */
  sessions: number;
  /** Quota activity on this service in this month (same census). */
  quotaActivity: number;
  /** Source-coverage spans certifying this census entry. */
  coverage: readonly SourceCoverageSpan[];
}

const MONTH_RE = /^\d{4}-(0[1-9]|1[0-2])$/;

/** `complete` only when an ok span covers the whole calendar month,
 * compared as instants (the spend-rollups coverageFor rule). */
function monthCoverage(month: string, spans: readonly SourceCoverageSpan[]): RollupCoverage {
  if (!MONTH_RE.test(month)) return "partial";
  const { start, end } = windowRange("month", `${month}-01`);
  const ws = Date.parse(start);
  const we = Date.parse(end);
  const full = spans.some((s) => {
    if (s.status !== "ok") return false;
    const from = Date.parse(s.from);
    const to = Date.parse(s.to);
    return Number.isFinite(from) && Number.isFinite(to) && from <= ws && to >= we;
  });
  return full ? "complete" : "partial";
}

function toCents(amountUsd: number): number {
  return Math.round(amountUsd * 100);
}

/**
 * Deduplicated monthly totals from the charges pipeline's matched
 * charges: one record per (service, month), exact integer-cent sums,
 * sorted by service then month. Charges whose posting date has no
 * calendar month are skipped (the pipeline's rows always carry one;
 * a dateless row certifies nothing). `coverage` is certified per
 * month from the charges source's coverage spans — the same spans
 * for every service, because the pipeline's completeness is a
 * property of the source, not of one service.
 */
export function monthlyChargeTotals(
  matched: readonly MatchedCharge[],
  coverage: readonly SourceCoverageSpan[],
): MonthlyChargeTotal[] {
  const byKey = new Map<string, { service: string; month: string; cents: number }>();
  for (const charge of matched) {
    const month = monthOf(charge.at);
    if (month === null || !charge.service.trim()) continue;
    if (!Number.isFinite(charge.amountUsd)) continue;
    const key = `${charge.service}|${month}`;
    const entry = byKey.get(key) ?? { service: charge.service, month, cents: 0 };
    entry.cents += toCents(charge.amountUsd);
    byKey.set(key, entry);
  }
  return [...byKey.values()]
    .sort((a, b) => a.service.localeCompare(b.service) || a.month.localeCompare(b.month))
    .map(({ service, month, cents }) => ({
      service,
      month,
      totalUsd: cents / 100,
      coverage: monthCoverage(month, coverage),
    }));
}

/** Producer seam: `monthlyCharges` backed by the charges pipeline. */
export function certifiedMonthlyCharges(
  matched: readonly MatchedCharge[],
  coverage: readonly SourceCoverageSpan[],
): () => MonthlyChargeTotal[] {
  return () => monthlyChargeTotals(matched, coverage);
}

/**
 * The certified activity record for one charge total, from a
 * per-service-month census. Returns null — uncertified, suppressing
 * the anomaly — when the census has no entry for the charge's service
 * and month, when its entries conflict, or when an entry's counts are
 * not finite non-negative integers. A present, consistent entry is
 * returned with its counts as counted and its coverage certified
 * from the entry's own spans: a complete census that counted real
 * activity suppresses by its counts, never by relabelling.
 */
export function certifiedServiceActivity(
  census: readonly ServiceActivityCensusEntry[],
): (charge: MonthlyChargeTotal) => ServiceActivityRecord | null {
  return (charge) => {
    const entries = census.filter((e) => e.service === charge.service && e.month === charge.month);
    if (entries.length === 0) return null;
    const first = entries[0]!;
    for (const entry of entries) {
      if (entry.sessions !== first.sessions || entry.quotaActivity !== first.quotaActivity) return null;
      if (!Number.isInteger(entry.sessions) || entry.sessions < 0) return null;
      if (!Number.isInteger(entry.quotaActivity) || entry.quotaActivity < 0) return null;
    }
    return {
      service: charge.service,
      month: charge.month,
      sessions: first.sessions,
      quotaActivity: first.quotaActivity,
      coverage: monthCoverage(charge.month, first.coverage),
    };
  };
}
