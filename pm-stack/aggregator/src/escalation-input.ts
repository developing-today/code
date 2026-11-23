// Escalation input adapter for the PM-stack SLO watchdog (#9991,
// design note doc/2026-11-14T12-54-00Z_design_pm_stack_slo_watchdog §4).
//
// escalation.ts's router takes an EscalationInput built from "the
// current incidents plus the budget figures slo.ts already computes",
// and remediation.ts owns the executor state those incidents' phases
// come from — but until this adapter existed, nothing built that
// input from the two production shapes: the router was only ever fed
// hand-built incidents in its own tests, so no production caller
// could use it. This module is that bridge. It is pure, like the two
// modules it joins; the server's watchdog tick calls it through
// escalation-tick.ts, which routes the built input and delivers it
// with the real journal/feed/push sinks.
import type { EscalationIncident, EscalationInput, IncidentPhase } from "./escalation.ts";
import {
  CATALOGUE,
  isCircuitBreakerOpen,
  type FacetId,
  type RemediationState,
} from "./remediation.ts";
import type { SloFacetSummary } from "./slo.ts";

/**
 * slo.ts definition id → the escalation facets that row sources. The
 * SLO table's ids are signal names, not facet ids: "intake-up" is the
 * pm-dispatch facet, and "pad-grist-up" is one joint row for the two
 * stateful services — both page unconditionally under §4, so the
 * joint row fans out to both rather than picking one arbitrarily.
 * The "database" facet has no SLO row and is never sourced here.
 */
export const SLO_FACET_MAP: Readonly<Record<string, readonly FacetId[]>> = {
  "feed-age": ["feed"],
  "quota-age": ["quota"],
  "orchestration-age": ["orchestration"],
  "aggregator-up": ["aggregator"],
  "intake-up": ["dispatch"],
  "projector-up": ["projector"],
  "pad-grist-up": ["pad", "grist"],
  "tunnel-up": ["tunnel"],
};

interface FacetProgress {
  phase: IncidentPhase;
  attemptsRemaining: number;
  circuitBreakerOpen: boolean;
}

/**
 * Where one bad facet's incident stands, derived from the remediation
 * executor's state. Only an action state carrying a current incident
 * counts: recordAttempt resets an action's tracking when its attempt
 * verifies (the incident closed), and a new incident restarts its
 * counter — so a stale exhausted flag from a closed incident must not
 * leak into a fresh one.
 *
 * - circuit breaker open, or every catalogue action for the facet
 *   exhausted in its current incident → "exhausted"
 * - an action for the facet is in flight → "remediating"
 * - some current-incident attempt was recorded and the facet is
 *   still bad, so its verification did not clear it →
 *   "verification-failed"
 * - otherwise → "detected"
 *
 * attemptsRemaining is the most attempts any one catalogue action
 * for the facet still has in its current incident (a fresh action
 * has its full cap); facets with no catalogue entries (stateful)
 * get 0.
 */
function facetProgress(state: RemediationState, facet: FacetId, now: number): FacetProgress {
  const circuitBreakerOpen = isCircuitBreakerOpen(state, facet, now);
  const entries = CATALOGUE.filter((entry) => entry.facet === facet);
  const live = entries.map((entry) => {
    const actionState = state.actions[`${facet}:${entry.action}`];
    return {
      entry,
      current: actionState && actionState.currentIncidentId !== null ? actionState : null,
    };
  });
  const attemptsRemaining = live.reduce(
    (max, { entry, current }) =>
      Math.max(
        max,
        current === null
          ? entry.attemptCap
          : current.exhausted
            ? 0
            : Math.max(0, entry.attemptCap - current.attemptsUsed),
      ),
    0,
  );
  let phase: IncidentPhase;
  if (circuitBreakerOpen || (live.length > 0 && live.every(({ current }) => current !== null && current.exhausted))) {
    phase = "exhausted";
  } else if (state.inFlight?.facet === facet) {
    phase = "remediating";
  } else if (live.some(({ current }) => current !== null && current.attemptsUsed > 0)) {
    phase = "verification-failed";
  } else {
    phase = "detected";
  }
  return { phase, attemptsRemaining, circuitBreakerOpen };
}

/**
 * Build the router's input from the live shapes: one incident per
 * currently-bad facet. A facet is bad when its SLO summary's latest
 * sample is bad; unsourced facets report latestBad null and never
 * escalate — silence must not be invented (escalation.ts's budget
 * rule). Summaries whose id has no facet mapping are skipped.
 * Incident order follows the summaries' order, fanning the joint
 * pad/grist row out in place.
 */
export function buildEscalationInput(
  summaries: readonly SloFacetSummary[],
  state: RemediationState,
  now: number,
): EscalationInput {
  const incidents: EscalationIncident[] = [];
  for (const summary of summaries) {
    if (summary.latestBad !== true) continue;
    const facets = SLO_FACET_MAP[summary.id];
    if (!facets) continue;
    for (const facet of facets) {
      const progress = facetProgress(state, facet, now);
      incidents.push({
        facet,
        phase: progress.phase,
        attemptsRemaining: progress.attemptsRemaining,
        budgetRemainingPercent: summary.budgetRemainingPercent,
        burnRate1h: summary.burnRate1h,
        circuitBreakerOpen: progress.circuitBreakerOpen,
      });
    }
  }
  return { incidents };
}
