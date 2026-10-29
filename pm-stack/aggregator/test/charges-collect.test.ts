// #1057: collector-level fixture replay for the charges auto-matcher —
// an export document in, feed rows + once-only surfacing out. Replaying
// the same period twice must add nothing and re-surface nothing (the
// packet's acceptance shape, exercised without live Plaid/Gmail data).
import { describe, expect, test } from "bun:test";
import { matchCharges } from "../src/charges-match.ts";
import {
  parseChargeCandidatesExport,
  parseSurfacedState,
  planUnmatchedSurfacing,
  serviceRefsFromSaas,
} from "../src/charges-collect.ts";
import type { SaasService } from "../src/model.ts";

const SAAS = [
  { id: "anthropic", name: "Anthropic" },
  { id: "openai", name: "OpenAI" },
] as SaasService[];

// One period as a producer would export it: the Anthropic charge seen
// by both sources, a Gmail-only Cursor charge (SaaS, unmatched), a
// Plaid-only grocery charge (not SaaS, not unmatched-worthy).
const EXPORT = JSON.stringify({
  candidates: [
    { source: "gmail", at: "2026-10-27", merchant: "Anthropic", amountUsd: 20 },
    { source: "plaid", at: "2026-10-28", merchant: "ANTHROPIC", amountUsd: 20, method: "Visa •7535" },
    { source: "gmail", at: "2026-10-26", merchant: "CURSOR.COM", amountUsd: 20 },
    { source: "plaid", at: "2026-10-25", merchant: "WALMART STORE 1234", amountUsd: 84.12 },
    { source: "plaid", at: "2026-10-25", merchant: "", amountUsd: 5 },
    { source: "other", at: "2026-10-25", merchant: "NOPE", amountUsd: 5 },
  ],
});

describe("parseChargeCandidatesExport", () => {
  test("keeps valid entries, drops malformed ones", () => {
    const candidates = parseChargeCandidatesExport(EXPORT);
    expect(candidates).toHaveLength(4);
    expect(candidates?.[0].source).toBe("gmail");
  });

  test("non-JSON and wrong-shape documents return null", () => {
    expect(parseChargeCandidatesExport("not json")).toBeNull();
    expect(parseChargeCandidatesExport('{"nope": true}')).toBeNull();
  });

  test("a bare array is also an export", () => {
    expect(parseChargeCandidatesExport("[]")).toEqual([]);
  });
});

describe("collector replay", () => {
  test("one row per charge, source labelled, cross-source deduped", () => {
    const candidates = parseChargeCandidatesExport(EXPORT)!;
    const { rows, unmatchedSaas } = matchCharges(candidates, serviceRefsFromSaas(SAAS));
    expect(rows).toHaveLength(3);
    const anthropic = rows.find((r) => r.label === "Anthropic");
    expect(anthropic?.source).toBe("Plaid");
    expect(anthropic?.method).toBe("Visa •7535");
    expect(unmatchedSaas.map((u) => u.normalized)).toEqual(["CURSOR"]);
  });

  test("unmatched surfacing happens exactly once across replays", () => {
    const candidates = parseChargeCandidatesExport(EXPORT)!;
    const { unmatchedSaas } = matchCharges(candidates, serviceRefsFromSaas(SAAS));

    const first = planUnmatchedSurfacing(unmatchedSaas, parseSurfacedState("not json"));
    expect(first.fresh).toHaveLength(1);

    const stateText = JSON.stringify({ surfacedKeys: first.nextKeys });
    const second = planUnmatchedSurfacing(unmatchedSaas, parseSurfacedState(stateText));
    expect(second.fresh).toEqual([]);
    expect(second.nextKeys).toEqual(first.nextKeys);
  });

  test("a new posting of the same merchant next period surfaces again", () => {
    const candidates = parseChargeCandidatesExport(EXPORT)!;
    const { unmatchedSaas } = matchCharges(candidates, serviceRefsFromSaas(SAAS));
    const first = planUnmatchedSurfacing(unmatchedSaas, []);
    const nextPeriod = [{ ...unmatchedSaas[0], at: "2026-11-26" }];
    const plan = planUnmatchedSurfacing(nextPeriod, first.nextKeys);
    expect(plan.fresh).toHaveLength(1);
  });
});
