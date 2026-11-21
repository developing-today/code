import { describe, expect, test } from "bun:test";
import {
  deliverEscalations,
  pagePayload,
  routeEscalations,
  type EscalationDecision,
  type EscalationIncident,
} from "../src/escalation.ts";

function incident(over: Partial<EscalationIncident> = {}): EscalationIncident {
  return {
    facet: "projector",
    phase: "detected",
    attemptsRemaining: 2,
    budgetRemainingPercent: 80,
    burnRate1h: 1,
    ...over,
  };
}

function route(over: Partial<EscalationIncident> = {}) {
  return routeEscalations({ incidents: [incident(over)] })[0]!;
}

describe("SLO watchdog escalation routing (#9991, design §4)", () => {
  test("remediation in progress routes to orchestrator without push", () => {
    const result = route({ phase: "remediating" });
    expect(result.route).toBe("needs-orchestrator");
    expect(result.push).toBe(false);
    expect(result.reason).toContain("in progress");
  });

  test("failed verification with attempts remaining routes to orchestrator", () => {
    const result = route({
      phase: "verification-failed",
      attemptsRemaining: 2,
    });
    expect(result.route).toBe("needs-orchestrator");
    expect(result.push).toBe(false);
    expect(result.reason).toContain("2 attempt(s) remaining");
  });

  test("a non-user-facing facet burning at exactly 2x routes to orchestrator", () => {
    const result = route({ facet: "quota", burnRate1h: 2 });
    expect(result.route).toBe("needs-orchestrator");
    expect(result.push).toBe(false);
    expect(result.reason).toContain("≤ 2×");
  });

  test("an exhausted remediation cap pages", () => {
    const result = route({ phase: "exhausted", attemptsRemaining: 0 });
    expect(result.route).toBe("page");
    expect(result.push).toBe(true);
    expect(result.reason).toContain("attempt cap reached");
  });

  test("an open circuit breaker pages even if the phase was not updated", () => {
    const result = route({ circuitBreakerOpen: true });
    expect(result.route).toBe("page");
    expect(result.reason).toContain("circuit breaker");
  });

  test.each(["aggregator", "dispatch", "tunnel"] as const)(
    "%s pages after its first failed verification",
    (facet) => {
      const result = route({
        facet,
        phase: "verification-failed",
        attemptsRemaining: 2,
      });
      expect(result.route).toBe("page");
      expect(result.push).toBe(true);
      expect(result.reason).toContain("first remediation failed verification");
    },
  );

  test.each(["pad", "grist", "database"] as const)(
    "stateful %s always pages",
    (facet) => {
      const result = route({ facet, phase: "detected" });
      expect(result.route).toBe("page");
      expect(result.push).toBe(true);
      expect(result.maxRemediationAttempts).toBe(0);
    },
  );

  test("two simultaneous bad facets page both, regardless of class", () => {
    const results = routeEscalations({
      incidents: [incident(), incident({ facet: "aggregator" })],
    });
    expect(results.map((result) => result.route)).toEqual(["page", "page"]);
    expect(results.every((result) => result.reason.includes("2 facets"))).toBe(
      true,
    );
  });

  test("budget below 25% pages on the first failed verification for any facet class", () => {
    const result = route({
      facet: "projector",
      phase: "verification-failed",
      budgetRemainingPercent: 24.99,
    });
    expect(result.route).toBe("page");
    expect(result.reason).toContain("< 25%");
  });

  test("25% is not below the tightening threshold", () => {
    expect(
      route({ phase: "verification-failed", budgetRemainingPercent: 25 }).route,
    ).toBe("needs-orchestrator");
  });

  test("exhausted budget pages on detection and caps remediation at one attempt", () => {
    const result = route({ phase: "detected", budgetRemainingPercent: 0 });
    expect(result.route).toBe("page");
    expect(result.push).toBe(true);
    expect(result.maxRemediationAttempts).toBe(1);
    expect(result.reason).toContain("page on detection");
  });

  test("unknown budget does not tighten; other incidents remain visible without invented pages", () => {
    const result = route({
      phase: "verification-failed",
      budgetRemainingPercent: null,
    });
    expect(result.route).toBe("needs-orchestrator");
  });

  test("routing invariants hold across every facet class and incident phase", () => {
    const facets = [
      "feed",
      "quota",
      "orchestration",
      "aggregator",
      "dispatch",
      "projector",
      "tunnel",
      "pad",
      "grist",
      "database",
    ] as const;
    const phases = [
      "detected",
      "remediating",
      "verification-failed",
      "exhausted",
    ] as const;
    for (const facet of facets) {
      for (const phase of phases) {
        for (const budgetRemainingPercent of [null, 50, 24, 0]) {
          const decision = route({ facet, phase, budgetRemainingPercent });
          // The route is the delivery oracle: a decision can never ask
          // both channels, nor produce a page without requesting push.
          expect(decision.push).toBe(decision.route === "page");
          expect(decision.reason.length).toBeGreaterThan(0);
          expect(decision.facet).toBe(facet);
        }
      }
    }
  });

  // Model-based: expected routes and caps come from an independent model
  // of design §4, written as the note states it (any page condition ⇒
  // page; otherwise NEEDS ORCHESTRATOR), not from the router's rule order.
  test("routes and remediation caps match the §4 model for every generated case", () => {
    const USER_FACING = new Set(["aggregator", "dispatch", "tunnel"]); // Aggregator, Intake, Tunnel
    const STATEFUL = new Set(["pad", "grist", "database"]); // Pad, Grist, any database
    const facets = [
      "feed",
      "quota",
      "orchestration",
      "aggregator",
      "dispatch",
      "projector",
      "tunnel",
      "pad",
      "grist",
      "database",
    ] as const;
    const phases = [
      "detected",
      "remediating",
      "verification-failed",
      "exhausted",
    ] as const;
    const model = (i: EscalationIncident, badCount: number) => {
      const b = i.budgetRemainingPercent;
      const remediationExhausted =
        i.phase === "exhausted" || i.circuitBreakerOpen === true;
      const budgetExhausted = b !== null && b <= 0;
      const pages =
        remediationExhausted ||
        (USER_FACING.has(i.facet) && i.phase === "verification-failed") ||
        badCount >= 2 ||
        STATEFUL.has(i.facet) ||
        (b !== null && b < 25 && i.phase === "verification-failed") ||
        budgetExhausted;
      const caps = [
        ...(STATEFUL.has(i.facet) ? [0] : []),
        ...(remediationExhausted ? [0] : []),
        ...(budgetExhausted ? [1] : []),
      ];
      return {
        route: pages ? "page" : "needs-orchestrator",
        cap: caps.length === 0 ? null : Math.min(...caps),
      };
    };
    let cases = 0;
    for (const facet of facets) {
      for (const phase of phases) {
        for (const budgetRemainingPercent of [null, 80, 25, 24.9, 1, 0, -10]) {
          for (const circuitBreakerOpen of [false, true]) {
            for (const burnRate1h of [null, 1, 2, 3]) {
              for (const others of [0, 1, 2]) {
                const subject = incident({
                  facet,
                  phase,
                  budgetRemainingPercent,
                  circuitBreakerOpen,
                  burnRate1h,
                });
                const incidents = [
                  subject,
                  ...Array.from({ length: others }, () =>
                    incident({ facet: "feed", phase: "remediating" }),
                  ),
                ];
                const decision = routeEscalations({ incidents })[0]!;
                const want = model(subject, incidents.length);
                // The case's inputs ride along so a mismatch names it.
                const label = JSON.stringify({ facet, phase, budgetRemainingPercent, circuitBreakerOpen, others });
                expect(`${label} ${decision.route}`).toBe(`${label} ${want.route}`);
                expect(decision.maxRemediationAttempts).toBe(want.cap);
                expect(decision.push).toBe(want.route === "page");
                cases++;
              }
            }
          }
        }
      }
    }
    expect(cases).toBe(10 * 4 * 7 * 2 * 4 * 3);
  });

  test("page notifications use the existing concise health-push payload shape", () => {
    const result = route({ facet: "aggregator", phase: "verification-failed" });
    expect(pagePayload(result)).toEqual({
      title: "PM stack: aggregator needs you",
      body: result.reason,
    });
  });

  test("delivery journals every decision, pushes pages, and feeds orchestrator routes without push", async () => {
    // Deliver separate decision batches: two simultaneous bad facets
    // correctly become the correlated-failure page rule, so these
    // represent independent incidents from separate watchdog passes.
    const decisions: EscalationDecision[] = [
      ...routeEscalations({
        incidents: [
          incident({ facet: "aggregator", phase: "verification-failed" }),
        ],
      }),
      ...routeEscalations({
        incidents: [incident({ facet: "quota", phase: "remediating" })],
      }),
    ];
    const journal: unknown[] = [];
    const pushes: unknown[] = [];
    const events: unknown[] = [];
    const delivered = await deliverEscalations(
      decisions,
      {
        journal: async (entry) => void journal.push(entry),
        push: async (payload) => void pushes.push(payload),
        feedEvent: async (entry) => void events.push(entry),
      },
      Date.parse("2026-11-20T12:00:00Z"),
    );
    expect(journal).toHaveLength(2);
    expect(pushes).toHaveLength(1);
    expect(events).toHaveLength(1);
    expect(delivered.map((result) => result.pushed)).toEqual([true, false]);
    expect(delivered.every((result) => result.error === null)).toBe(true);
  });

  test("a failed sink is reported and does not prevent later incidents from being delivered", async () => {
    const decisions = routeEscalations({
      incidents: [
        incident({ facet: "aggregator", phase: "verification-failed" }),
        incident({ facet: "pad" }),
      ],
    });
    let pushes = 0;
    const delivered = await deliverEscalations(
      decisions,
      {
        journal: async () => {},
        push: async () => {
          pushes++;
          if (pushes === 1) throw new Error("push unavailable");
        },
        feedEvent: async () => {},
      },
      1_800_000_000_000,
    );
    expect(delivered[0]!.error).toContain("push: push unavailable");
    expect(delivered[1]!.pushed).toBe(true);
    expect(pushes).toBe(2);
  });

  test("a push that resolves with ok: false is reported as not delivered", async () => {
    const decisions = routeEscalations({
      incidents: [incident({ facet: "aggregator", phase: "verification-failed" })],
    });
    const delivered = await deliverEscalations(
      decisions,
      {
        journal: async () => {},
        // push.ts's sendPush shape for a missing VAPID key.
        push: async () => ({ ok: false, gone: false, error: "VAPID private key unavailable (bws)" }),
        feedEvent: async () => {},
      },
      1_800_000_000_000,
    );
    expect(delivered[0]!.pushed).toBe(false);
    expect(delivered[0]!.error).toBe("push: VAPID private key unavailable (bws)");
  });

  test("a push that resolves with ok: true is delivered", async () => {
    const decisions = routeEscalations({ incidents: [incident({ facet: "pad" })] });
    const delivered = await deliverEscalations(
      decisions,
      { journal: async () => {}, push: async () => ({ ok: true, status: 201 }), feedEvent: async () => {} },
      1_800_000_000_000,
    );
    expect(delivered[0]!.pushed).toBe(true);
    expect(delivered[0]!.error).toBeNull();
  });
});
