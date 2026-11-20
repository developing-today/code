import { describe, expect, test } from "bun:test";
import {
  CATALOGUE,
  FACET_DAILY_CAP,
  GLOBAL_DAILY_CAP,
  CIRCUIT_BREAKER_EXHAUSTED_THRESHOLD,
  STATEFUL_FACETS,
  emptyState,
  findCatalogueEntry,
  decideRemediation,
  recordAttempt,
  serializeState,
  deserializeState,
  setInFlight,
  type RemediationState,
  type FacetId,
  type ActionId,
  type CatalogueEntry,
} from "../src/remediation.ts";

const NOW = Date.parse("2026-11-19T12:00:00Z");
const iso = (ms: number) => new Date(ms).toISOString();

describe("remediation catalogue (#9989)", () => {
  test("catalogue contains only stateless actions; no pad/grist/database", () => {
    for (const e of CATALOGUE) {
      expect(STATEFUL_FACETS.has(e.facet)).toBe(false);
    }
    // Every entry has positive caps/cooldowns.
    for (const e of CATALOGUE) {
      expect(e.attemptCap).toBeGreaterThan(0);
      expect(e.cooldownMs).toBeGreaterThan(0);
      expect(e.verificationMs).toBeGreaterThan(0);
    }
  });

  test("stateful facets are never in the catalogue", () => {
    for (const f of ["pad", "grist", "database", "orchestration"]) {
      const entries = CATALOGUE.filter((e) => e.facet === f);
      expect(entries.length).toBe(0);
    }
  });

  test("findCatalogueEntry resolves known entries", () => {
    const e = findCatalogueEntry("feed", "restart-collector");
    expect(e).toBeDefined();
    expect(e!.attemptCap).toBe(3);
    expect(e!.cooldownMs).toBe(15 * 60_000);
  });
});

describe("decideRemediation — deploy lock (§2 precondition)", () => {
  test("refused when deploy lock present, with deploy-lock blockedBy and journal entry", () => {
    const state = emptyState(NOW);
    const d = decideRemediation("feed", "restart-collector", state, NOW, true, "inc-1");
    expect(d.allowed).toBe(false);
    expect(d.blockedBy).toBe("deploy-lock");
    expect(d.journalEntry.decision).toBe("refused");
    expect(d.journalEntry.blockedBy).toBe("deploy-lock");
    expect(d.journalEntry.deployLocked).toBe(true);
  });

  test("allowed when no deploy lock", () => {
    const state = emptyState(NOW);
    const d = decideRemediation("feed", "restart-collector", state, NOW, false, "inc-1");
    expect(d.allowed).toBe(true);
    expect(d.blockedBy).toBeUndefined();
    expect(d.journalEntry.decision).toBe("allowed");
  });
});

describe("decideRemediation — stateful services (§5 never-do)", () => {
  test("pad refused as stateful", () => {
    const state = emptyState(NOW);
    const d = decideRemediation("pad", "restart-collector" as any, state, NOW, false, "inc-1");
    expect(d.allowed).toBe(false);
    expect(d.blockedBy).toBe("stateful");
  });

  test("grist refused as stateful", () => {
    const state = emptyState(NOW);
    const d = decideRemediation("grist", "restart-collector" as any, state, NOW, false, "inc-1");
    expect(d.allowed).toBe(false);
    expect(d.blockedBy).toBe("stateful");
  });

  test("database refused as stateful", () => {
    const state = emptyState(NOW);
    const d = decideRemediation("database", "restart-collector" as any, state, NOW, false, "inc-1");
    expect(d.allowed).toBe(false);
    expect(d.blockedBy).toBe("stateful");
  });
});

describe("decideRemediation — not in catalogue", () => {
  test("unknown action refused", () => {
    const state = emptyState(NOW);
    const d = decideRemediation("feed", "restart-tunnel" as any, state, NOW, false, "inc-1");
    expect(d.allowed).toBe(false);
    expect(d.blockedBy).toBe("not-in-catalogue");
  });
});

describe("decideRemediation — cooldown", () => {
  test("refused while cooldown active", () => {
    let state = emptyState(NOW);
    // record an attempt (failed) to start cooldown
    state = recordAttempt(state, "feed", "restart-collector", "inc-1", NOW, { verified: false });
    const d = decideRemediation("feed", "restart-collector", state, NOW + 1000, false, "inc-1");
    expect(d.allowed).toBe(false);
    expect(d.blockedBy).toBe("cooldown");
  });

  test("allowed after cooldown expires", () => {
    let state = emptyState(NOW);
    state = recordAttempt(state, "feed", "restart-collector", "inc-1", NOW, { verified: false });
    const entry = findCatalogueEntry("feed", "restart-collector")!;
    const after = NOW + entry.cooldownMs + 1;
    const d = decideRemediation("feed", "restart-collector", state, after, false, "inc-1");
    // attempt cap not yet hit (1 of 3), so allowed
    expect(d.allowed).toBe(true);
  });
});

describe("decideRemediation — attempt cap", () => {
  test("refused after attempt cap reached in same incident", () => {
    let state = emptyState(NOW);
    const entry = findCatalogueEntry("feed", "restart-collector")!;
    // use all attempts
    for (let i = 0; i < entry.attemptCap; i++) {
      state = recordAttempt(state, "feed", "restart-collector", "inc-1", NOW + i * 1000, { verified: false });
    }
    // cooldown expired
    const after = NOW + entry.attemptCap * 1000 + entry.cooldownMs + 1;
    const d = decideRemediation("feed", "restart-collector", state, after, false, "inc-1");
    expect(d.allowed).toBe(false);
    expect(d.blockedBy).toBe("attempt-cap");
  });

  test("attempt counter resets for a new incident", () => {
    let state = emptyState(NOW);
    const entry = findCatalogueEntry("feed", "restart-collector")!;
    for (let i = 0; i < entry.attemptCap; i++) {
      state = recordAttempt(state, "feed", "restart-collector", "inc-1", NOW + i * 1000, { verified: false });
    }
    const after = NOW + entry.attemptCap * 1000 + entry.cooldownMs + 1;
    // new incident id resets attempts
    const d = decideRemediation("feed", "restart-collector", state, after, false, "inc-2");
    expect(d.allowed).toBe(true);
  });
});

describe("decideRemediation — daily caps", () => {
  test("per-facet daily cap enforced", () => {
    let state = emptyState(NOW);
    // Simulate FACET_DAILY_CAP successful+failed attempts across incidents
    for (let i = 0; i < FACET_DAILY_CAP; i++) {
      state = recordAttempt(state, "feed", "restart-collector", `inc-${i}`, NOW + i * 1000, { verified: false });
    }
    // Need to bypass cooldown: use a fresh action? cooldown blocks. Use a different facet's action? 
    // Actually cooldown is per action. Let's check global-ish: use tunnel (different cooldown) but same daily facet? 
    // feed daily counter aggregates all feed actions. restart-collector cooldown blocks. 
    // To test daily cap in isolation, advance time past all cooldowns.
    const entry = findCatalogueEntry("feed", "restart-collector")!;
    const after = NOW + FACET_DAILY_CAP * 1000 + entry.cooldownMs + 1;
    const d = decideRemediation("feed", "restart-collector", state, after, false, "inc-new");
    expect(d.allowed).toBe(false);
    expect(d.blockedBy).toBe("daily-cap-facet");
  });

  test("global daily cap enforced", () => {
    let state = emptyState(NOW);
    // Spread attempts across facets/actions to hit global cap without per-facet cap
    const actions: Array<[FacetId, ActionId]> = [
      ["feed", "restart-collector"],
      ["aggregator", "restart-aggregator"],
      ["dispatch", "restart-dispatch"],
      ["projector", "restart-projector"],
      ["tunnel", "restart-tunnel"],
      ["quota", "rerun-quota-refresh"],
    ];
    let t = NOW;
    let i = 0;
    while (true) {
      const [f, a] = actions[i % actions.length]!;
      state = recordAttempt(state, f, a, `inc-${i}`, t, { verified: false });
      t += 1000;
      i++;
      if (i >= GLOBAL_DAILY_CAP) break;
    }
    const after = t + 60 * 60_000;
    // any action should now be blocked by global cap
    const d = decideRemediation("feed", "rerun-feed-push", state, after, false, "inc-x");
    expect(d.allowed).toBe(false);
    expect(d.blockedBy).toBe("daily-cap-global");
  });
});

describe("decideRemediation — circuit breaker", () => {
  test("opens after 3 exhausted incidents in 24h", () => {
    let state = emptyState(NOW);
    const entry = findCatalogueEntry("feed", "restart-collector")!;
    // 3 incidents, each exhausting all attempts
    for (let inc = 0; inc < 3; inc++) {
      const base = NOW + inc * 60 * 60_000;
      for (let a = 0; a < entry.attemptCap; a++) {
        state = recordAttempt(state, "feed", "restart-collector", `inc-${inc}`, base + a * 1000, { verified: false });
      }
    }
    // circuit breaker should now be open
    const cb = state.circuitBreakers["feed"];
    expect(cb).toBeDefined();
    expect(cb!.closesAt).toBeGreaterThan(NOW);
    // decision refused
    const after = NOW + 3 * 60 * 60_000 + entry.cooldownMs + 1;
    const d = decideRemediation("feed", "restart-collector", state, after, false, "inc-new");
    expect(d.allowed).toBe(false);
    expect(d.blockedBy).toBe("circuit-breaker");
  });

  test("does not open before threshold", () => {
    let state = emptyState(NOW);
    const entry = findCatalogueEntry("feed", "restart-collector")!;
    for (let inc = 0; inc < CIRCUIT_BREAKER_EXHAUSTED_THRESHOLD - 1; inc++) {
      const base = NOW + inc * 60 * 60_000;
      for (let a = 0; a < entry.attemptCap; a++) {
        state = recordAttempt(state, "feed", "restart-collector", `inc-${inc}`, base + a * 1000, { verified: false });
      }
    }
    expect(state.circuitBreakers["feed"]).toBeUndefined();
  });

  test("verified attempt does not exhaust and does not count toward breaker", () => {
    let state = emptyState(NOW);
    const entry = findCatalogueEntry("feed", "restart-collector")!;
    // 3 incidents but all verified on first attempt
    for (let inc = 0; inc < 3; inc++) {
      const base = NOW + inc * 60 * 60_000;
      state = recordAttempt(state, "feed", "restart-collector", `inc-${inc}`, base, { verified: true });
    }
    expect(state.circuitBreakers["feed"]).toBeUndefined();
    expect((state.exhaustedIncidents["feed"] ?? []).length).toBe(0);
  });
});

describe("decideRemediation — in-flight", () => {
  test("refused when another action in flight", () => {
    const state = setInFlight(emptyState(NOW), "aggregator", "restart-aggregator", NOW);
    const d = decideRemediation("feed", "restart-collector", state, NOW, false, "inc-1");
    expect(d.allowed).toBe(false);
    expect(d.blockedBy).toBe("in-flight");
  });
});

describe("recordAttempt", () => {
  test("verified attempt resets attempt counter and cooldown", () => {
    let state = emptyState(NOW);
    state = recordAttempt(state, "feed", "restart-collector", "inc-1", NOW, { verified: false });
    const before = state.actions["feed:restart-collector"]!;
    expect(before.attemptsUsed).toBe(1);
    // now a verified attempt
    state = recordAttempt(state, "feed", "restart-collector", "inc-1", NOW + 1000, { verified: true });
    const after = state.actions["feed:restart-collector"]!;
    expect(after.attemptsUsed).toBe(0);
    expect(after.currentIncidentId).toBeNull();
    expect(after.cooldownUntil).toBeNull();
  });

  test("failed attempt increments counter and sets cooldown", () => {
    const state = recordAttempt(emptyState(NOW), "feed", "restart-collector", "inc-1", NOW, { verified: false });
    const a = state.actions["feed:restart-collector"]!;
    expect(a.attemptsUsed).toBe(1);
    expect(a.cooldownUntil).toBe(NOW + 15 * 60_000);
    expect(a.exhausted).toBe(false);
  });

  test("exhausted on last failed attempt", () => {
    let state = emptyState(NOW);
    const entry = findCatalogueEntry("feed", "restart-collector")!;
    for (let i = 0; i < entry.attemptCap; i++) {
      state = recordAttempt(state, "feed", "restart-collector", "inc-1", NOW + i * 1000, { verified: false });
    }
    const a = state.actions["feed:restart-collector"]!;
    expect(a.exhausted).toBe(true);
    expect((state.exhaustedIncidents["feed"] ?? []).length).toBe(1);
  });
});

describe("serialization", () => {
  test("round-trips state", () => {
    let state = emptyState(NOW);
    state = recordAttempt(state, "feed", "restart-collector", "inc-1", NOW, { verified: false });
    const raw = serializeState(state);
    const restored = deserializeState(raw, NOW);
    expect(restored.actions["feed:restart-collector"]!.attemptsUsed).toBe(1);
  });

  test("deserializeState handles corrupt input", () => {
    const restored = deserializeState("not json", NOW);
    expect(restored).toEqual(emptyState(NOW));
  });
});
