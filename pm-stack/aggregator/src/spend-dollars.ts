// spend-dollars (#4826, #1259 slice 5): PAYG dollar attribution — the
// pure core that attaches real charges from the charges pipeline
// (#1057: charges-collect / charges-match / charges-catalog) to
// projects and epics in the spend rollups (design:
// doc/spend-rollups-design-1259.md §2, "PAYG dollars" row).
//
// Rules, from the design note and #4826 — stated once, here:
// (1) Only matched, real charges enter the dollars column. This
//     module consumes the charges pipeline's output; it never builds
//     a second money path and never re-decides what a charge is.
// (2) A charge attributes to a project/epic only through BOTH the
//     service it matched AND attributed session activity for that
//     service in the charge's period (its calendar month). Attributed
//     activity is an input — the slice-1 projection's facts (#4822),
//     supplied by the caller — never something this module infers
//     from raw sessions.
// (3) A matched charge with no attributed activity for its service
//     and month lands at stack level as unattributed dollars: shown,
//     with a reason, never hidden and never force-attributed. The
//     same holds when the month's activity for the service resolves
//     to more than one project/epic: a charge is never split across
//     scopes and never guessed onto one of them.
// (4) Unmatched charges never enter here at all: they stay in the
//     charges pipeline's own unmatched surfacing
//     (ChargeMatchResult.unmatchedSaas), absent from every rollup.
// (5) Dollars are money Drew actually paid. They are never converted
//     to or from tokens or share-of-window, and no input to this
//     module carries a per-session list-price estimate: the writer's
//     inputs are charge amounts and attribution facts only (guarded
//     by test/spend-dollars-4826.test.ts).
//
// Pure and deterministic, like charges-match: integer cents inside,
// so the reconciliation invariant — rollup dollars per month equal
// the charges pipeline's matched totals for that month — is exact,
// not approximate.

import type { ChargeMatchResult } from "./charges-match.ts";

/** One matched charge, reduced to what dollar attribution may read. */
export interface MatchedCharge {
  /** Hub service name the charges pipeline matched the charge to. */
  service: string;
  /** ISO date the charge posted (YYYY-MM-DD). */
  at: string;
  /** Money actually paid, USD. */
  amountUsd: number;
}

/**
 * One attributed-activity fact for a service in a month: in that
 * month, sessions on that service were attributed (slice-1
 * projection, #4822) to this project/epic. A fact with both refs
 * null carries no scope and attributes nothing.
 */
export interface AttributedActivity {
  service: string;
  /** Calendar month, YYYY-MM. */
  month: string;
  projectRef: string | null;
  epicRef: string | null;
}

export type UnattributedDollarsReason = "no-attributed-activity" | "ambiguous-activity";

/** A matched charge that landed at stack level, with its reason. */
export interface UnattributedCharge {
  service: string;
  at: string;
  amountUsd: number;
  reason: UnattributedDollarsReason;
}

/** Dollars attributed to one project/epic scope in a month. */
export interface ProjectDollars {
  projectRef: string | null;
  epicRef: string | null;
  dollarsUsd: number;
}

/** The spend-rollup dollars for one calendar month. */
export interface PaygDollarRollup {
  /** Calendar month, YYYY-MM. */
  month: string;
  /** Attributed dollars per project/epic, sorted by ref. */
  projectDollars: ProjectDollars[];
  /** Stack-level unattributed dollars: shown, never hidden. */
  unattributedDollarsUsd: number;
  /** The charges behind the unattributed dollars, with reasons. */
  unattributedCharges: UnattributedCharge[];
  /** The charges pipeline's matched total for the month. */
  matchedTotalUsd: number;
  /**
   * Attributed + unattributed dollars for the month. Invariant:
   * always equals matchedTotalUsd (reconciliation, #4826).
   */
  rollupTotalUsd: number;
}

const MONTH_RE = /^\d{4}-\d{2}$/;

/** The calendar month (YYYY-MM) a YYYY-MM-DD date falls in, or null. */
export function monthOf(at: string): string | null {
  const month = at.slice(0, 7);
  return MONTH_RE.test(month) ? month : null;
}

function toCents(amountUsd: number): number {
  return Math.round(amountUsd * 100);
}

function toDollars(cents: number): number {
  return cents / 100;
}

/**
 * The matched half of a charges-pipeline result: rows whose label is
 * a known hub service name. Unmatched rows (merchant-labelled) and
 * the unmatchedSaas surfacing are deliberately not returned — rule
 * (4): they are absent from the rollups by construction.
 */
export function matchedChargesFrom(result: ChargeMatchResult, serviceNames: ReadonlySet<string>): MatchedCharge[] {
  return result.rows
    .filter((row) => serviceNames.has(row.label))
    .map((row) => ({ service: row.label, at: row.at, amountUsd: row.amountUsd }));
}

interface Scope {
  projectRef: string | null;
  epicRef: string | null;
}

function scopeKey(scope: Scope): string {
  return `${scope.projectRef ?? ""}|${scope.epicRef ?? ""}`;
}

/**
 * Attribute matched charges to projects/epics for the spend rollups,
 * one rollup per calendar month present in the charges, months in
 * ascending order. See the header for the rules; the reconciliation
 * invariant (rollupTotalUsd === matchedTotalUsd per month) holds for
 * every returned rollup.
 */
export function attributePaygDollars(matched: MatchedCharge[], activity: AttributedActivity[]): PaygDollarRollup[] {
  // service|month -> the distinct scopes its attributed activity
  // resolves to. Facts with no scope attribute nothing.
  const scopesByServiceMonth = new Map<string, Map<string, Scope>>();
  for (const fact of activity) {
    if (fact.projectRef === null && fact.epicRef === null) continue;
    if (!MONTH_RE.test(fact.month)) continue;
    const key = `${fact.service}|${fact.month}`;
    let scopes = scopesByServiceMonth.get(key);
    if (!scopes) {
      scopes = new Map();
      scopesByServiceMonth.set(key, scopes);
    }
    const scope: Scope = { projectRef: fact.projectRef, epicRef: fact.epicRef };
    scopes.set(scopeKey(scope), scope);
  }

  interface MonthWork {
    projectCents: Map<string, { scope: Scope; cents: number }>;
    unattributedCents: number;
    unattributedCharges: UnattributedCharge[];
    matchedCents: number;
  }
  const byMonth = new Map<string, MonthWork>();
  const workFor = (month: string): MonthWork => {
    let work = byMonth.get(month);
    if (!work) {
      work = { projectCents: new Map(), unattributedCents: 0, unattributedCharges: [], matchedCents: 0 };
      byMonth.set(month, work);
    }
    return work;
  };

  for (const charge of matched) {
    const month = monthOf(charge.at);
    if (month === null) continue;
    const work = workFor(month);
    const cents = toCents(charge.amountUsd);
    work.matchedCents += cents;

    const scopes = scopesByServiceMonth.get(`${charge.service}|${month}`);
    if (!scopes || scopes.size === 0) {
      work.unattributedCents += cents;
      work.unattributedCharges.push({
        service: charge.service,
        at: charge.at,
        amountUsd: toDollars(cents),
        reason: "no-attributed-activity",
      });
      continue;
    }
    if (scopes.size > 1) {
      // Never split a charge across scopes, never guess one of them.
      work.unattributedCents += cents;
      work.unattributedCharges.push({
        service: charge.service,
        at: charge.at,
        amountUsd: toDollars(cents),
        reason: "ambiguous-activity",
      });
      continue;
    }
    const scope = [...scopes.values()][0]!;
    const key = scopeKey(scope);
    const entry = work.projectCents.get(key) ?? { scope, cents: 0 };
    entry.cents += cents;
    work.projectCents.set(key, entry);
  }

  return [...byMonth.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([month, work]) => {
      const projectDollars: ProjectDollars[] = [...work.projectCents.values()]
        .map(({ scope, cents }) => ({
          projectRef: scope.projectRef,
          epicRef: scope.epicRef,
          dollarsUsd: toDollars(cents),
        }))
        .sort(
          (a, b) =>
            (a.projectRef ?? "").localeCompare(b.projectRef ?? "") || (a.epicRef ?? "").localeCompare(b.epicRef ?? ""),
        );
      const attributedCents = [...work.projectCents.values()].reduce((sum, e) => sum + e.cents, 0);
      return {
        month,
        projectDollars,
        unattributedDollarsUsd: toDollars(work.unattributedCents),
        unattributedCharges: work.unattributedCharges.sort(
          (a, b) => a.at.localeCompare(b.at) || a.service.localeCompare(b.service),
        ),
        matchedTotalUsd: toDollars(work.matchedCents),
        rollupTotalUsd: toDollars(attributedCents + work.unattributedCents),
      };
    });
}
