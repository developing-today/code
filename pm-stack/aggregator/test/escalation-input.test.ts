import { describe, expect, test } from "bun:test";
import { routeEscalations } from "../src/escalation.ts";
import { buildEscalationInput, SLO_FACET_MAP } from "../src/escalation-input.ts";
import { emptyState, recordAttempt, setInFlight, type RemediationState } from "../src/remediation.ts";
import type { SloFacetSummary } from "../src/slo.ts";

const NOW = Date.parse("2026-11-22T12:00:00.000Z");

function summary(over: Partial<SloFacetSummary> = {}): SloFacetSummary {
  return {
    id: "quota-age",
    label: "Quota read age",
    currentValue: 100,
    target: "≤ 75 min at p99",
    budgetRemainingPercent: 80,
    burnRate1h: 1,
    state: "burning",
    badMinutes: 10,
    budgetMinutes: 432,
    unknownSamples: 0,
    sourced: true,
    latestBad: true,
    ...over,
  };
}

/** Run the adapter and the router together, as the watchdog tick will. */
function routed(summaries: SloFacetSummary[], state: RemediationState) {
  return routeEscalations(buildEscalationInput(summaries, state, NOW));
}

describe("escalation input adapter (#9991, design §4)", () => {
  test("every SLO definition id maps to facets; database is never sourced", () => {
    expect(Object.keys(SLO_FACET_MAP).sort()).toEqual([
      "aggregator-up",
      "feed-age",
      "intake-up",
      "orchestration-age",
      "pad-grist-up",
      "projector-up",
      "quota-age",
      "tunnel-up",
    ]);
    const allBad = Object.keys(SLO_FACET_MAP).map((id) => summary({ id }));
    const input = buildEscalationInput(allBad, emptyState(NOW), NOW);
    expect(input.incidents.map((i) => i.facet)).toEqual([
      "feed",
      "quota",
      "orchestration",
      "aggregator",
      "dispatch",
      "projector",
      "pad",
      "grist",
      "tunnel",
    ]);
    expect(input.incidents.some((i) => i.facet === "database")).toBe(false);
  });

  test("only a facet whose latest sample is bad becomes an incident", () => {
    const input = buildEscalationInput(
      [
        summary({ id: "quota-age", latestBad: false }),
        summary({ id: "feed-age", latestBad: null, sourced: false }),
        summary({ id: "unknown-row", latestBad: true }),
        summary({ id: "tunnel-up", latestBad: true }),
      ],
      emptyState(NOW),
      NOW,
    );
    expect(input.incidents.map((i) => i.facet)).toEqual(["tunnel"]);
  });

  test("a fresh incident is detected, with the full catalogue cap remaining", () => {
    const input = buildEscalationInput([summary({ id: "quota-age" })], emptyState(NOW), NOW);
    expect(input.incidents).toEqual([
      {
        facet: "quota",
        phase: "detected",
        attemptsRemaining: 2,
        budgetRemainingPercent: 80,
        burnRate1h: 1,
        circuitBreakerOpen: false,
      },
    ]);
  });

  test("the joint pad/grist row fans out to both stateful facets, which page", () => {
    const decisions = routed([summary({ id: "pad-grist-up" })], emptyState(NOW));
    expect(decisions.map((d) => d.facet)).toEqual(["pad", "grist"]);
    for (const decision of decisions) {
      expect(decision.route).toBe("page");
      expect(decision.push).toBe(true);
      expect(decision.maxRemediationAttempts).toBe(0);
      expect(decision.reason).toContain("stateful");
    }
    const input = buildEscalationInput([summary({ id: "pad-grist-up" })], emptyState(NOW), NOW);
    expect(input.incidents.every((i) => i.attemptsRemaining === 0)).toBe(true);
  });

  test("an in-flight remediation routes to orchestrator without a page", () => {
    const state = setInFlight(emptyState(NOW), "feed", "restart-collector", NOW);
    const decisions = routed([summary({ id: "feed-age" })], state);
    expect(decisions[0]!.route).toBe("needs-orchestrator");
    expect(decisions[0]!.push).toBe(false);
    expect(decisions[0]!.reason).toContain("in progress");
  });

  test("an unverified attempt on a still-bad facet is verification-failed", () => {
    let state = emptyState(NOW);
    state = recordAttempt(state, "feed", "restart-collector", "inc-1", NOW, { verified: false });
    const input = buildEscalationInput([summary({ id: "feed-age" })], state, NOW);
    expect(input.incidents[0]!.phase).toBe("verification-failed");
    // restart-collector has 2 of 3 left; rerun-feed-push is untouched at 3.
    expect(input.incidents[0]!.attemptsRemaining).toBe(3);
    const decisions = routed([summary({ id: "feed-age" })], state);
    expect(decisions[0]!.route).toBe("needs-orchestrator");
    expect(decisions[0]!.reason).toContain("3 attempt(s) remaining");
  });

  test("a verified attempt closes the incident tracking: a later bad sample is a fresh detection", () => {
    let state = emptyState(NOW);
    state = recordAttempt(state, "feed", "restart-collector", "inc-1", NOW, { verified: false });
    state = recordAttempt(state, "feed", "restart-collector", "inc-1", NOW + 1, { verified: true });
    const input = buildEscalationInput([summary({ id: "feed-age" })], state, NOW + 2);
    expect(input.incidents[0]!.phase).toBe("detected");
    expect(input.incidents[0]!.attemptsRemaining).toBe(3);
  });

  test("a user-facing facet pages after its first failed verification", () => {
    let state = emptyState(NOW);
    state = recordAttempt(state, "aggregator", "restart-aggregator", "inc-1", NOW, {
      verified: false,
    });
    const decisions = routed([summary({ id: "aggregator-up" })], state);
    expect(decisions[0]!.route).toBe("page");
    expect(decisions[0]!.reason).toContain("one failed fix");
  });

  test("exhausting every catalogue attempt pages, with none remaining", () => {
    let state = emptyState(NOW);
    state = recordAttempt(state, "projector", "restart-projector", "inc-1", NOW, {
      verified: false,
    });
    state = recordAttempt(state, "projector", "restart-projector", "inc-1", NOW + 1, {
      verified: false,
    });
    const input = buildEscalationInput([summary({ id: "projector-up" })], state, NOW + 2);
    expect(input.incidents[0]!.phase).toBe("exhausted");
    expect(input.incidents[0]!.attemptsRemaining).toBe(0);
    const decisions = routed([summary({ id: "projector-up" })], state);
    expect(decisions[0]!.route).toBe("page");
    expect(decisions[0]!.reason).toContain("attempt cap reached");
  });

  test("three exhausted incidents open the circuit breaker, which pages", () => {
    let state = emptyState(NOW);
    for (const incidentId of ["inc-1", "inc-2", "inc-3"]) {
      state = recordAttempt(state, "tunnel", "restart-tunnel", incidentId, NOW, {
        verified: false,
      });
      state = recordAttempt(state, "tunnel", "restart-tunnel", incidentId, NOW, {
        verified: false,
      });
    }
    const input = buildEscalationInput([summary({ id: "tunnel-up" })], state, NOW);
    expect(input.incidents[0]!.circuitBreakerOpen).toBe(true);
    expect(input.incidents[0]!.phase).toBe("exhausted");
    const decisions = routed([summary({ id: "tunnel-up" })], state);
    expect(decisions[0]!.route).toBe("page");
    expect(decisions[0]!.reason).toContain("circuit breaker");
  });

  test("a budget under 25% tightens a non-user-facing facet to page on first failed verification", () => {
    let state = emptyState(NOW);
    state = recordAttempt(state, "projector", "restart-projector", "inc-1", NOW, {
      verified: false,
    });
    const decisions = routed([summary({ id: "projector-up", budgetRemainingPercent: 20 })], state);
    expect(decisions[0]!.route).toBe("page");
    expect(decisions[0]!.reason).toContain("< 25%");
  });

  test("an exhausted budget pages on detection and caps remediation at one attempt", () => {
    const decisions = routed([summary({ id: "feed-age", budgetRemainingPercent: 0 })], emptyState(NOW));
    expect(decisions[0]!.route).toBe("page");
    expect(decisions[0]!.maxRemediationAttempts).toBe(1);
    expect(decisions[0]!.reason).toContain("budget exhausted");
  });

  test("two bad facets page together as a correlated failure", () => {
    const decisions = routed([summary({ id: "feed-age" }), summary({ id: "quota-age" })], emptyState(NOW));
    expect(decisions.map((d) => d.route)).toEqual(["page", "page"]);
    expect(decisions[0]!.reason).toContain("correlated failure");
  });

  test("an unknown budget never tightens: a detected non-user-facing facet stays orchestrator work", () => {
    const decisions = routed(
      [summary({ id: "quota-age", budgetRemainingPercent: null, burnRate1h: null })],
      emptyState(NOW),
    );
    expect(decisions[0]!.route).toBe("needs-orchestrator");
    expect(decisions[0]!.push).toBe(false);
  });
});
