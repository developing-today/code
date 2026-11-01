// GitHub billing collector core (#1051; hub design §2.2/§5): maps the
// unified billing usage endpoint's items to §4 snapshot records.
//
//   GET /users/{user}/settings/billing/usage?year=YYYY&month=M
//
// answers with the account's existing gh OAuth token — the old
// per-product billing endpoints are gone (410, "This endpoint has been
// moved"), and no billing-scoped token is needed for the own-account
// usage read (verified live 2026-10-29, triage task 2315). Items are
// per-day, per-SKU rows:
//   { date, product, sku, unitType, quantity, pricePerUnit,
//     grossAmount, discountAmount, netAmount, repositoryName }
//
// Aggregation is a group-by on date: Actions minutes, Codespaces
// core-hours, storage GB-hours (the endpoint's own unit — GB-hours is
// NOT GB; the snapshot unit keeps them distinct), plus gross/net USD
// across all products. Records are stamped at the usage day (midday
// UTC, the store's day convention), never at collection time, so a
// re-run re-derives identical records and the store dedupes them.
//
// Pure functions, no I/O — scripts/collect-github-billing.ts owns the
// gh call and the outbox append; tests feed literals.
import type { SnapshotRecord } from "../model.ts";
import { dayTs } from "../snapshot-records.ts";
import { isIsoDate, isNonNegative } from "../validate.ts";

export interface BillingUsageItem {
  date?: string;
  product?: string;
  sku?: string;
  unitType?: string;
  quantity?: number;
  grossAmount?: number;
  discountAmount?: number;
  netAmount?: number;
  [key: string]: unknown;
}

export interface BillingDayAggregate {
  date: string;
  actionsMinutes: number;
  codespacesHours: number;
  actionsStorageGbHours: number;
  codespacesStorageGbHours: number;
  grossUsd: number;
  netUsd: number;
}

function round(n: number, dp: number): number {
  const f = 10 ** dp;
  return Math.round(n * f) / f;
}

function emptyDay(date: string): BillingDayAggregate {
  return {
    date,
    actionsMinutes: 0,
    codespacesHours: 0,
    actionsStorageGbHours: 0,
    codespacesStorageGbHours: 0,
    grossUsd: 0,
    netUsd: 0,
  };
}

/** Codespaces compute rows are machine-hours, but the hub's
 * `codespaces-hours` metric is core-hours (src/saas-extract.ts):
 * one hour on an N-core machine consumes N core-hours (GitHub's
 * Codespaces pricing multiplier). The core count is parsed from the
 * SKU ("Codespaces 2-core"); a SKU with no parseable core count is
 * taken at face value (x1) rather than dropped. */
function codespacesCoreMultiplier(sku: unknown): number {
  if (typeof sku !== "string") return 1;
  const m = /(\d+)\s*-core/i.exec(sku);
  return m ? Number(m[1]) : 1;
}

/** A numeric endpoint field contributes only when it is a finite,
 * non-negative number (#2493): a negative or non-finite quantity or
 * amount is not usage, and a negative derived record would pass
 * isSnapshotRecord and be stored. Anything else contributes 0,
 * extending the existing non-number fallback. */
function usageNumber(v: unknown): number {
  return isNonNegative(v) ? v : 0;
}

/** Per-day aggregates from raw endpoint items, oldest day first. */
export function aggregateBillingUsage(items: BillingUsageItem[]): BillingDayAggregate[] {
  const byDate = new Map<string, BillingDayAggregate>();
  for (const item of items) {
    if (typeof item.date !== "string" || item.date.length < 10) continue;
    const date = item.date.slice(0, 10);
    // The bucket date must be a real calendar date (#2493): an
    // impossible date (month 13, Feb 30) derives snapshot records
    // that fail isSnapshotRecord, and the collector throws on ANY
    // invalid record — discarding the whole month's collection.
    if (!isIsoDate(date)) continue;
    const day = byDate.get(date) ?? emptyDay(date);
    const qty = usageNumber(item.quantity);
    // Classification is case-insensitive: the endpoint's documented
    // response example spells product/unitType "Actions"/"minutes"
    // while live rows arrive as "actions"/"Minutes" — both must
    // classify identically, never silently drop to money-only.
    const product = typeof item.product === "string" ? item.product.toLowerCase() : "";
    const unit = typeof item.unitType === "string" ? item.unitType.toLowerCase() : "";
    if (product === "actions" && unit === "minutes") day.actionsMinutes += qty;
    else if (product === "actions" && unit === "gigabytehours") day.actionsStorageGbHours += qty;
    else if (product === "codespaces" && unit === "hours")
      day.codespacesHours += qty * codespacesCoreMultiplier(item.sku);
    else if (product === "codespaces" && unit === "gigabytehours") day.codespacesStorageGbHours += qty;
    // Gross/net span every product the endpoint reports, known or not.
    day.grossUsd += usageNumber(item.grossAmount);
    day.netUsd += usageNumber(item.netAmount);
    byDate.set(date, day);
  }
  return [...byDate.values()]
    .sort((a, b) => a.date.localeCompare(b.date))
    .map((d) => ({
      date: d.date,
      actionsMinutes: round(d.actionsMinutes, 3),
      codespacesHours: round(d.codespacesHours, 3),
      actionsStorageGbHours: round(d.actionsStorageGbHours, 3),
      codespacesStorageGbHours: round(d.codespacesStorageGbHours, 3),
      grossUsd: round(d.grossUsd, 2),
      netUsd: round(d.netUsd, 2),
    }));
}

/** Month totals over per-day aggregates — the collector's test target
 * ("reproduces the endpoint's aggregates within accrual"). */
export function summarizeBillingDays(days: BillingDayAggregate[]): Omit<BillingDayAggregate, "date"> {
  const total = {
    actionsMinutes: 0,
    codespacesHours: 0,
    actionsStorageGbHours: 0,
    codespacesStorageGbHours: 0,
    grossUsd: 0,
    netUsd: 0,
  };
  for (const d of days) {
    total.actionsMinutes += d.actionsMinutes;
    total.codespacesHours += d.codespacesHours;
    total.actionsStorageGbHours += d.actionsStorageGbHours;
    total.codespacesStorageGbHours += d.codespacesStorageGbHours;
    total.grossUsd += d.grossUsd;
    total.netUsd += d.netUsd;
  }
  return {
    actionsMinutes: round(total.actionsMinutes, 3),
    codespacesHours: round(total.codespacesHours, 3),
    actionsStorageGbHours: round(total.actionsStorageGbHours, 3),
    codespacesStorageGbHours: round(total.codespacesStorageGbHours, 3),
    grossUsd: round(total.grossUsd, 2),
    netUsd: round(total.netUsd, 2),
  };
}

/** §4 snapshot records for one month of endpoint items, under the
 * hub's existing `github` service and card metric ids where they
 * exist (`codespaces-hours`). Zero aggregates emit no record — a day
 * with no Codespaces use is an absence, not a 0 observation. */
export function recordsFromBillingUsage(items: BillingUsageItem[], observedBy: string): SnapshotRecord[] {
  const out: SnapshotRecord[] = [];
  for (const day of aggregateBillingUsage(items)) {
    const ts = dayTs(day.date);
    const push = (metricId: string, value: number, unit: SnapshotRecord["unit"]) => {
      if (value === 0) return;
      out.push({ ts, serviceId: "github", metricId, value, unit, source: "api", observedBy });
    };
    push("actions-minutes", day.actionsMinutes, "minutes");
    push("codespaces-hours", day.codespacesHours, "hours");
    push("actions-storage-gb-hours", day.actionsStorageGbHours, "gb-hours");
    push("codespaces-storage-gb-hours", day.codespacesStorageGbHours, "gb-hours");
    push("billing-gross", day.grossUsd, "usd");
    push("billing-net", day.netUsd, "usd");
  }
  return out;
}
