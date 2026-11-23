// Escalation tick for the PM-stack SLO watchdog (#9991, design note
// doc/2026-11-14T12-54-00Z_design_pm_stack_slo_watchdog §4): the host
// glue that calls the §4 router from the server's watchdog tick.
// escalation.ts decides, escalation-input.ts adapts the live shapes,
// and this module owns the per-tick pipeline — summaries plus
// remediation state in, decisions routed, delivery through the
// injected sink — plus the one piece of state the pipeline needs:
// which decisions this process has already delivered.
//
// Repeat suppression: the watchdog tick runs every five minutes, but
// a decision is a statement about an incident, not about a tick. An
// unchanged decision (same facet, route, reason and attempt cap) is
// delivered once; it is delivered again only when the decision
// itself changes (a second facet goes bad and the route tightens to
// a correlated page, the budget crosses a §4 threshold, remediation
// exhausts) or when the facet recovers and later goes bad again —
// recovery clears the facet's delivered mark, so a recurrence is a
// new incident and escalates fresh. This mirrors the stack's own
// stable-key discipline (watchdog tier items keyed watchdog:<input>,
// activity deliveries deduped by stable source id): without it, a
// stateful outage would page Drew every five minutes until fixed.
// A delivery that reports an error is NOT marked delivered, so the
// next tick retries it; the journal therefore also records failed
// attempts (design §3: every verdict is logged).
//
// The remediation-state seam: buildEscalationInput reads the
// remediation executor's state, but the executor has no production
// host yet — #9989 delivered remediation.ts as a pure module whose
// state the host persists, and nothing in the repo calls
// decideRemediation/recordAttempt outside tests. The tick therefore
// takes the state from an injected reader; the server passes one
// returning the empty state, which is the stack's actual condition
// (no action attempted, nothing in flight, no breaker open), so
// phases read "detected" until an executor host lands and the
// reader is pointed at its persisted state (#9990's verification
// work presupposes the same host). The SLO side is fully live
// either way: stateful, correlated-failure and budget-exhausted
// pages all fire from the summaries alone.
import type { ActivityDelivery } from "./activity.ts";
import {
  deliverEscalations,
  routeEscalations,
  type DeliveredEscalation,
  type EscalationDecision,
  type EscalationJournalEntry,
  type EscalationSink,
} from "./escalation.ts";
import { buildEscalationInput } from "./escalation-input.ts";
import type { FacetId, RemediationState } from "./remediation.ts";
import type { SloFacetSummary } from "./slo.ts";
import type { SloJournalLine } from "./slo-journal.ts";

/** The journal facet an escalation line rides under: `escalation:<facet>`,
 *  the heartbeat's trick (facet "watchdog-heartbeat") — a facet key no
 *  SLO definition reads, so budget summaries never see these lines as
 *  samples, while the entry itself stays in the one append-only
 *  watchdog journal design §3 names. */
export function escalationJournalLine(entry: EscalationJournalEntry): SloJournalLine {
  return {
    facet: `escalation:${entry.facet}`,
    atMs: Date.parse(entry.ts),
    bad: true,
    value: null,
    escalation: entry,
  };
}

/** The actions-feed mirror of one escalation (design §3), as an
 *  activity delivery on the same surface #1148's watchdog reports
 *  into. The source id is stable per decision — facet, route and the
 *  router's reason text — so the activity store's dedupe also holds
 *  if a delivery is ever replayed. */
export function escalationDelivery(entry: EscalationJournalEntry): ActivityDelivery {
  const routeText = entry.route === "page" ? "page Drew" : "NEEDS ORCHESTRATOR";
  return {
    source: "watchdog",
    sourceId: `escalation:${entry.facet}@${entry.route}:${entry.reason}`,
    at: entry.ts,
    kind: "watchdog-event",
    ref: `escalation:${entry.facet}`,
    summary: `Watchdog escalation: ${entry.facet} → ${routeText} — ${entry.reason}`.slice(0, 500),
  };
}

/** Identity of a decision for repeat suppression: everything the
 *  delivery says, so any change in what Drew or the orchestrator
 *  would be told re-delivers. */
export function escalationDecisionKey(decision: EscalationDecision): string {
  return `${decision.route}|${decision.reason}|${decision.maxRemediationAttempts ?? ""}`;
}

export interface EscalationTickerDeps {
  /** The live SLO summaries (the server's sloSummaries). */
  summaries(now: number): Promise<readonly SloFacetSummary[]>;
  /** The remediation executor's state — see the seam note above. */
  remediationState(now: number): RemediationState;
  sink: EscalationSink;
  now?: () => number;
}

export interface EscalationTickResult {
  /** Every decision the router made this tick (delivered or suppressed). */
  decisions: EscalationDecision[];
  /** The deliveries actually attempted this tick. */
  delivered: DeliveredEscalation[];
}

export function createEscalationTicker(deps: EscalationTickerDeps): {
  tick(): Promise<EscalationTickResult>;
} {
  const deliveredKeys = new Map<FacetId, string>();
  let running = false;
  return {
    async tick(): Promise<EscalationTickResult> {
      if (running) return { decisions: [], delivered: [] };
      running = true;
      try {
        const now = deps.now?.() ?? Date.now();
        const summaries = await deps.summaries(now);
        const decisions = routeEscalations(
          buildEscalationInput(summaries, deps.remediationState(now), now),
        );
        // Recovery clears the delivered mark: a facet absent from
        // this tick's decisions has recovered (or gone unsourced),
        // so its next bad episode is a new incident.
        const present = new Set(decisions.map((decision) => decision.facet));
        for (const facet of [...deliveredKeys.keys()]) {
          if (!present.has(facet)) deliveredKeys.delete(facet);
        }
        const fresh = decisions.filter(
          (decision) => deliveredKeys.get(decision.facet) !== escalationDecisionKey(decision),
        );
        if (fresh.length === 0) return { decisions, delivered: [] };
        const delivered = await deliverEscalations(fresh, deps.sink, now);
        const freshByFacet = new Map(fresh.map((decision) => [decision.facet, decision]));
        for (const result of delivered) {
          if (result.error !== null) continue;
          const decision = freshByFacet.get(result.entry.facet);
          if (decision) deliveredKeys.set(decision.facet, escalationDecisionKey(decision));
        }
        return { decisions, delivered };
      } finally {
        running = false;
      }
    },
  };
}
