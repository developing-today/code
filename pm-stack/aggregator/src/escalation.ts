// Escalation router for the PM-stack SLO watchdog (#9991, design note
// doc/2026-11-14T12-54-00Z_design_pm_stack_slo_watchdog §4).
//
// The watchdog DETECTS (watchdog.ts, #1148), REMEDIATES (remediation.ts,
// #9989) and VERIFIES (design §3). This module is the ESCALATE half: for
// every facet that is bad right now, decide between the two channels §4
// names — **page Drew** (a push through the existing page-health-push
// path: aggregator push.ts → alerts.ts) or **NEEDS ORCHESTRATOR** (a
// journal line + a feed event, no push) — and apply the §4 budget-aware
// tightening.
//
// The decision function is PURE: it takes the current incidents plus the
// budget figures slo.ts already computes and returns a route per facet.
// Delivery is a separate, dependency-injected step (deliverEscalations),
// so tests never touch the network and a push failure can never break
// the watchdog loop. This mirrors remediation.ts's split: the guarantees
// live in the decision, the host owns I/O.
//
// Page channel: §6 open question 1 (PWA/push vs the existing page path)
// has no Drew answer, and #9991's acceptance defaults to the existing
// page-health-push path until then — so a page here is a push through
// alerts.ts's existing sender, nothing new.
import type { FacetId } from "./remediation.ts";

/** §4 facet classes: the three that are user-facing, the stateful
 *  services that are always a page, and everything else. */
export type FacetClass = "user-facing" | "non-user-facing" | "stateful";

export const FACET_CLASSES: Readonly<Record<FacetId, FacetClass>> = {
  // §4: "a user-facing facet (Aggregator, Intake, Tunnel)".
  aggregator: "user-facing",
  dispatch: "user-facing",
  tunnel: "user-facing",
  // §4: "a stateful service (Pad, Grist, any database)" — never silent,
  // never an auto-action.
  pad: "stateful",
  grist: "stateful",
  database: "stateful",
  // Stateless producers and age facets.
  feed: "non-user-facing",
  quota: "non-user-facing",
  orchestration: "non-user-facing",
  projector: "non-user-facing",
};

/** Where an incident stands, as the executor/verifier reports it. */
export type IncidentPhase =
  /** Bad, no action taken yet (first detection). */
  | "detected"
  /** An action is running or its verification window is open. */
  | "remediating"
  /** An action's verification failed and the attempt cap is not reached. */
  | "verification-failed"
  /** Cap or circuit breaker reached and the facet is still bad. */
  | "exhausted";

export interface EscalationIncident {
  facet: FacetId;
  phase: IncidentPhase;
  /** Attempts left in the incident's catalogue cap (0 = none). */
  attemptsRemaining: number;
  /** §1 budget remaining percent from slo.ts; null = unsourced/unknown,
   *  which never tightens the threshold (silence must not be invented). */
  budgetRemainingPercent: number | null;
  /** §4's burn rate (bad minutes last hour ÷ budget ÷ window share);
   *  null = unknown. Only the ≤2× non-user-facing rule reads it. */
  burnRate1h: number | null;
  /** §2 circuit breaker currently open for this facet. */
  circuitBreakerOpen?: boolean;
}

export interface EscalationInput {
  /** Every facet that is bad right now (recovered facets are absent). */
  incidents: readonly EscalationIncident[];
}

export type EscalationRoute = "page" | "needs-orchestrator";

export interface EscalationDecision {
  facet: FacetId;
  facetClass: FacetClass;
  /** "page" = push Drew; "needs-orchestrator" = journal + feed, no push. */
  route: EscalationRoute;
  /** Why, one clause per matched rule; the first is the matching rule. */
  reason: string;
  /** Convenience for the delivery step: page ⇒ push. */
  push: boolean;
  /** §4 budget-exhausted tightening: at most one remediation attempt,
   *  then the router pages on detection. null = no override. */
  maxRemediationAttempts: number | null;
}

/** §4 budget tightening thresholds. */
export const BUDGET_TIGHTEN_PERCENT = 25;
export const BUDGET_EXHAUSTED_PERCENT = 0;
/** §4's "burns at ≤ 2× rate" bound for non-user-facing facets. */
export const NON_USER_FACING_BURN_MAX = 2;
/** §4 budget-exhausted rule: page on detection after at most one attempt. */
export const EXHAUSTED_BUDGET_MAX_ATTEMPTS = 1;

/**
 * The remediation-attempt override §4 implies, independent of which page
 * rule supplies the reason: a stateful service gets no auto-action, an
 * exhausted incident (cap or breaker) gets no further attempt, and an
 * exhausted budget allows at most one. The tightest applicable cap wins.
 */
function remediationCap(
  incident: EscalationIncident,
  facetClass: FacetClass,
): number | null {
  const caps: number[] = [];
  if (facetClass === "stateful") caps.push(0);
  if (incident.phase === "exhausted" || incident.circuitBreakerOpen === true) {
    caps.push(0);
  }
  const budget = incident.budgetRemainingPercent;
  if (budget !== null && budget <= BUDGET_EXHAUSTED_PERCENT) {
    caps.push(EXHAUSTED_BUDGET_MAX_ATTEMPTS);
  }
  return caps.length === 0 ? null : Math.min(...caps);
}

function decide(
  incident: EscalationIncident,
  badCount: number,
): EscalationDecision {
  const decision = decideRoute(incident, badCount);
  decision.maxRemediationAttempts = remediationCap(
    incident,
    decision.facetClass,
  );
  return decision;
}

function decideRoute(
  incident: EscalationIncident,
  badCount: number,
): EscalationDecision {
  const facetClass = FACET_CLASSES[incident.facet];
  const budget = incident.budgetRemainingPercent;
  const budgetExhausted = budget !== null && budget <= BUDGET_EXHAUSTED_PERCENT;
  const budgetTightened = budget !== null && budget < BUDGET_TIGHTEN_PERCENT;
  const exhausted =
    incident.phase === "exhausted" || incident.circuitBreakerOpen === true;

  // §4 page rules, in design order. A page always wins; NEEDS
  // ORCHESTRATOR is the fallthrough.

  // "A stateful service (Pad, Grist, any database) is down — always a page."
  if (facetClass === "stateful") {
    return {
      facet: incident.facet,
      facetClass,
      route: "page",
      reason:
        "stateful service is bad: always a page, never an auto-action (design §4)",
      push: true,
      maxRemediationAttempts: 0,
    };
  }

  // "Two or more facets are bad simultaneously (correlated failure)."
  if (badCount >= 2) {
    return {
      facet: incident.facet,
      facetClass,
      route: "page",
      reason: `${badCount} facets are bad simultaneously: correlated failure, per-facet remediation is the wrong tool (design §4)`,
      push: true,
      maxRemediationAttempts: budgetExhausted
        ? EXHAUSTED_BUDGET_MAX_ATTEMPTS
        : null,
    };
  }

  // "When budget is exhausted, the watchdog pages on detection and
  // performs at most one remediation attempt before standing down."
  if (budgetExhausted) {
    return {
      facet: incident.facet,
      facetClass,
      route: "page",
      reason: `budget exhausted (${budget}% remaining): page on detection, at most ${EXHAUSTED_BUDGET_MAX_ATTEMPTS} remediation attempt (design §4)`,
      push: true,
      maxRemediationAttempts: EXHAUSTED_BUDGET_MAX_ATTEMPTS,
    };
  }

  // "When a facet's budget remaining drops below 25%, its page threshold
  // tightens to first failed verification regardless of facet class."
  if (budgetTightened && incident.phase === "verification-failed") {
    return {
      facet: incident.facet,
      facetClass,
      route: "page",
      reason: `budget remaining ${budget}% < ${BUDGET_TIGHTEN_PERCENT}%: first failed verification pages regardless of facet class (design §4)`,
      push: true,
      maxRemediationAttempts: null,
    };
  }

  // "A user-facing facet (Aggregator, Intake, Tunnel) is down and the
  // first verification fails — after one failed fix, not three."
  if (
    facetClass === "user-facing" &&
    incident.phase === "verification-failed"
  ) {
    return {
      facet: incident.facet,
      facetClass,
      route: "page",
      reason:
        "user-facing facet is bad and its first remediation failed verification: page after one failed fix (design §4)",
      push: true,
      maxRemediationAttempts: null,
    };
  }

  // "Remediation is exhausted (cap or circuit breaker) and the facet is
  // still bad."
  if (exhausted) {
    return {
      facet: incident.facet,
      facetClass,
      route: "page",
      reason: incident.circuitBreakerOpen
        ? "remediation exhausted: circuit breaker open and the facet is still bad (design §4)"
        : "remediation exhausted: attempt cap reached and the facet is still bad (design §4)",
      push: true,
      maxRemediationAttempts: 0,
    };
  }

  // §4 NEEDS ORCHESTRATOR conditions (no push).
  if (incident.phase === "remediating") {
    return {
      facet: incident.facet,
      facetClass,
      route: "needs-orchestrator",
      reason: "incident detected and remediation is in progress (design §4)",
      push: false,
      maxRemediationAttempts: null,
    };
  }

  if (incident.phase === "verification-failed") {
    return {
      facet: incident.facet,
      facetClass,
      route: "needs-orchestrator",
      reason: `an action failed verification with ${incident.attemptsRemaining} attempt(s) remaining (design §4)`,
      push: false,
      maxRemediationAttempts: null,
    };
  }

  // "A non-user-facing facet (projector, quota) burns at ≤ 2× rate."
  if (facetClass === "non-user-facing") {
    const burn = incident.burnRate1h;
    const burnText =
      burn === null
        ? "burn rate unknown"
        : `burning at ${burn.toFixed(2)}× the window-share budget`;
    const qualifying = burn === null || burn <= NON_USER_FACING_BURN_MAX;
    return {
      facet: incident.facet,
      facetClass,
      route: "needs-orchestrator",
      reason: qualifying
        ? `non-user-facing facet is ${burnText} (≤ ${NON_USER_FACING_BURN_MAX}×): ORCHESTRATOR, not a page (design §4)`
        : `non-user-facing facet is ${burnText}: above the §4 2× band, but no page rule matches yet — ORCHESTRATOR keeps it visible`,
      push: false,
      maxRemediationAttempts: null,
    };
  }

  // A user-facing facet that is bad with remediation not yet started has
  // no page rule (§4 pages after the first failed verification) — it is
  // orchestrator work until then.
  return {
    facet: incident.facet,
    facetClass,
    route: "needs-orchestrator",
    reason:
      "facet is bad but no §4 page rule matches yet: orchestrator work until the first verification (design §4)",
    push: false,
    maxRemediationAttempts: null,
  };
}

/**
 * Route every bad facet to a channel. Pure: same input, same decisions.
 * Order follows the input incident order, so callers can present it
 * stably.
 */
export function routeEscalations(input: EscalationInput): EscalationDecision[] {
  const badCount = input.incidents.length;
  return input.incidents.map((incident) => decide(incident, badCount));
}

// --- Delivery -------------------------------------------------------------

/** One line of the watchdog journal, in the §3/§4 shape. */
export interface EscalationJournalEntry {
  ts: string;
  facet: FacetId;
  route: EscalationRoute;
  reason: string;
  maxRemediationAttempts: number | null;
}

export interface PagePayload {
  title: string;
  body: string;
}

/** The side effects a delivery needs, injected so tests use fakes and
 *  the host can wire the real push/alerts/events surfaces. */
export interface EscalationSink {
  /** The page-health-push path (push.ts sendPush via alerts.ts). */
  push(page: PagePayload): Promise<unknown> | unknown;
  /** The append-only watchdog journal (design §3 logging). */
  journal(entry: EscalationJournalEntry): Promise<unknown> | unknown;
  /** The actions feed event (§4: "journal + feed event"). */
  feedEvent(entry: EscalationJournalEntry): Promise<unknown> | unknown;
}

export interface DeliveredEscalation {
  entry: EscalationJournalEntry;
  pushed: boolean;
  /** A push or feed failure, reported but never thrown (the watchdog
   *  loop must survive a dead push endpoint). */
  error: string | null;
}

/** A push result that reports failure, as push.ts's PushOutcome does,
 *  turned into its error text; null when the push was delivered (or the
 *  sink returned nothing, which a throwing sink uses to mean success). */
function pushRefusal(outcome: unknown): string | null {
  if (
    outcome !== null &&
    typeof outcome === "object" &&
    "ok" in outcome &&
    (outcome as { ok: unknown }).ok === false
  ) {
    const err = (outcome as { error?: unknown }).error;
    return typeof err === "string" && err.length > 0
      ? err
      : "push was not delivered";
  }
  return null;
}

/** One page for a facet, worded for a phone notification. */
export function pagePayload(decision: EscalationDecision): PagePayload {
  return {
    title: `PM stack: ${decision.facet} needs you`,
    body: decision.reason,
  };
}

/**
 * Deliver the decisions: every decision goes to the journal; a page also
 * goes to push (the interim channel from §6 question 1); a
 * NEEDS ORCHESTRATOR route also goes to the actions feed. Failures are
 * caught per decision so one dead channel cannot stop the rest, and a
 * failed page is reported in the result rather than silently dropped.
 */
export async function deliverEscalations(
  decisions: readonly EscalationDecision[],
  sink: EscalationSink,
  now: number,
): Promise<DeliveredEscalation[]> {
  const out: DeliveredEscalation[] = [];
  for (const decision of decisions) {
    const entry: EscalationJournalEntry = {
      ts: new Date(now).toISOString(),
      facet: decision.facet,
      route: decision.route,
      reason: decision.reason,
      maxRemediationAttempts: decision.maxRemediationAttempts,
    };
    let pushed = false;
    let error: string | null = null;
    try {
      await sink.journal(entry);
    } catch (err) {
      error = `journal: ${err instanceof Error ? err.message : String(err)}`;
    }
    if (decision.push) {
      try {
        const outcome = await sink.push(pagePayload(decision));
        // push.ts's sendPush resolves { ok: false, error } instead of
        // throwing (missing VAPID key, expired subscription, endpoint
        // error), so a resolved push is only delivered when it says so.
        const refused = pushRefusal(outcome);
        if (refused === null) {
          pushed = true;
        } else {
          error = `push: ${refused}`;
        }
      } catch (err) {
        error = `push: ${err instanceof Error ? err.message : String(err)}`;
      }
    } else {
      try {
        await sink.feedEvent(entry);
      } catch (err) {
        error = `feed: ${err instanceof Error ? err.message : String(err)}`;
      }
    }
    out.push({ entry, pushed, error });
  }
  return out;
}
