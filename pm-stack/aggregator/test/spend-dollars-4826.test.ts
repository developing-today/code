// #4826 (#1259 slice 5): PAYG dollar attribution via the charges
// pipeline — the issue's acceptance tests, run against the real
// pipeline (candidates -> matchCharges -> matchedChargesFrom ->
// attributePaygDollars), plus the reconciliation invariant and the
// guard that no rollup writer input reads a per-session cost value.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { matchCharges, type ChargeCandidate } from "../src/charges-match.ts";
import {
  attributePaygDollars,
  matchedChargesFrom,
  monthOf,
  type AttributedActivity,
} from "../src/spend-dollars.ts";

const SERVICES = [
  { name: "Anthropic", aliases: ["Claude"] },
  { name: "OpenAI", aliases: ["ChatGPT"] },
  { name: "Cloudflare" },
];
const SERVICE_NAMES = new Set(SERVICES.map((s) => s.name));

function rollupsFor(candidates: ChargeCandidate[], activity: AttributedActivity[]) {
  const result = matchCharges(candidates, SERVICES);
  return { result, rollups: attributePaygDollars(matchedChargesFrom(result, SERVICE_NAMES), activity) };
}

const ANTHROPIC_OCT: ChargeCandidate = {
  source: "plaid",
  at: "2026-10-14",
  merchant: "ANTHROPIC *CLAUDE",
  amountUsd: 20,
  method: "Visa •7535",
};

describe("attributePaygDollars (#4826 acceptance)", () => {
  test("matched charge with in-period attributed activity -> project/epic dollars", () => {
    const { rollups } = rollupsFor([ANTHROPIC_OCT], [
      { service: "Anthropic", month: "2026-10", projectRef: "proj/pm", epicRef: "epic/1184" },
    ]);
    expect(rollups).toHaveLength(1);
    expect(rollups[0]!.projectDollars).toEqual([
      { projectRef: "proj/pm", epicRef: "epic/1184", dollarsUsd: 20 },
    ]);
    expect(rollups[0]!.unattributedDollarsUsd).toBe(0);
    expect(rollups[0]!.unattributedCharges).toEqual([]);
  });

  test("matched charge without attributed activity -> stack-level unattributed dollars", () => {
    const { rollups } = rollupsFor([ANTHROPIC_OCT], []);
    expect(rollups[0]!.projectDollars).toEqual([]);
    expect(rollups[0]!.unattributedDollarsUsd).toBe(20);
    expect(rollups[0]!.unattributedCharges).toEqual([
      { service: "Anthropic", at: "2026-10-14", amountUsd: 20, reason: "no-attributed-activity" },
    ]);
  });

  test("activity in a different month does not attribute the charge", () => {
    const { rollups } = rollupsFor([ANTHROPIC_OCT], [
      { service: "Anthropic", month: "2026-09", projectRef: "proj/pm", epicRef: "epic/1184" },
    ]);
    expect(rollups[0]!.projectDollars).toEqual([]);
    expect(rollups[0]!.unattributedDollarsUsd).toBe(20);
  });

  test("activity for a different service does not attribute the charge", () => {
    const { rollups } = rollupsFor([ANTHROPIC_OCT], [
      { service: "OpenAI", month: "2026-10", projectRef: "proj/pm", epicRef: "epic/1184" },
    ]);
    expect(rollups[0]!.projectDollars).toEqual([]);
    expect(rollups[0]!.unattributedDollarsUsd).toBe(20);
  });

  test("unmatched charge -> absent from rollups, still surfaced as unmatched", () => {
    const mystery: ChargeCandidate = {
      source: "plaid",
      at: "2026-10-20",
      merchant: "CURSOR.COM",
      amountUsd: 42.5,
      method: "Visa •7535",
    };
    const { result, rollups } = rollupsFor([mystery], [
      { service: "Anthropic", month: "2026-10", projectRef: "proj/pm", epicRef: "epic/1184" },
    ]);
    // Still surfaced by the charges pipeline's own unmatched path.
    expect(result.unmatchedSaas).toHaveLength(1);
    expect(result.unmatchedSaas[0]!.merchant).toBe("CURSOR.COM");
    // And absent from every rollup figure.
    expect(rollups).toEqual([]);
  });

  test("activity resolving to two scopes -> unattributed, never split or guessed", () => {
    const { rollups } = rollupsFor([ANTHROPIC_OCT], [
      { service: "Anthropic", month: "2026-10", projectRef: "proj/a", epicRef: "epic/1" },
      { service: "Anthropic", month: "2026-10", projectRef: "proj/b", epicRef: "epic/2" },
    ]);
    expect(rollups[0]!.projectDollars).toEqual([]);
    expect(rollups[0]!.unattributedDollarsUsd).toBe(20);
    expect(rollups[0]!.unattributedCharges[0]!.reason).toBe("ambiguous-activity");
  });

  test("repeated facts for one scope still attribute once, charges sum in cents", () => {
    const second: ChargeCandidate = { ...ANTHROPIC_OCT, at: "2026-10-21", amountUsd: 15.99 };
    const { rollups } = rollupsFor([ANTHROPIC_OCT, second], [
      { service: "Anthropic", month: "2026-10", projectRef: "proj/pm", epicRef: "epic/1184" },
      { service: "Anthropic", month: "2026-10", projectRef: "proj/pm", epicRef: "epic/1184" },
    ]);
    expect(rollups[0]!.projectDollars).toEqual([
      { projectRef: "proj/pm", epicRef: "epic/1184", dollarsUsd: 35.99 },
    ]);
  });

  test("reconciliation: rollup dollars equal the pipeline's matched totals, per month", () => {
    const candidates: ChargeCandidate[] = [
      ANTHROPIC_OCT,
      { source: "gmail", at: "2026-10-03", merchant: "OpenAI *ChatGPT", amountUsd: 20 },
      { source: "plaid", at: "2026-11-02", merchant: "CLOUDFLARE", amountUsd: 5.25 },
      { source: "plaid", at: "2026-11-09", merchant: "ANTHROPIC *CLAUDE", amountUsd: 20 },
      { source: "plaid", at: "2026-11-11", merchant: "CURSOR.COM", amountUsd: 42.5 },
    ];
    const activity: AttributedActivity[] = [
      { service: "Anthropic", month: "2026-10", projectRef: "proj/pm", epicRef: "epic/1184" },
      { service: "Anthropic", month: "2026-11", projectRef: "proj/pm", epicRef: "epic/1184" },
    ];
    const { result, rollups } = rollupsFor(candidates, activity);
    const matchedTotal = matchedChargesFrom(result, SERVICE_NAMES).reduce((s, c) => s + c.amountUsd, 0);
    expect(rollups.map((r) => r.month)).toEqual(["2026-10", "2026-11"]);
    for (const rollup of rollups) {
      expect(rollup.rollupTotalUsd).toBe(rollup.matchedTotalUsd);
      const parts =
        rollup.projectDollars.reduce((s, p) => s + p.dollarsUsd, 0) + rollup.unattributedDollarsUsd;
      expect(Math.round(parts * 100)).toBe(Math.round(rollup.matchedTotalUsd * 100));
    }
    expect(Math.round(rollups.reduce((s, r) => s + r.rollupTotalUsd, 0) * 100)).toBe(
      Math.round(matchedTotal * 100),
    );
    // Spot-check the split: Oct Anthropic attributed, OpenAI not;
    // Nov Cloudflare not, Anthropic attributed.
    expect(rollups[0]!.projectDollars).toEqual([
      { projectRef: "proj/pm", epicRef: "epic/1184", dollarsUsd: 20 },
    ]);
    expect(rollups[0]!.unattributedDollarsUsd).toBe(20);
    expect(rollups[1]!.projectDollars).toEqual([
      { projectRef: "proj/pm", epicRef: "epic/1184", dollarsUsd: 20 },
    ]);
    expect(rollups[1]!.unattributedDollarsUsd).toBe(5.25);
  });
});

describe("monthOf", () => {
  test("extracts YYYY-MM from an ISO date and rejects non-dates", () => {
    expect(monthOf("2026-10-14")).toBe("2026-10");
    expect(monthOf("")).toBeNull();
    expect(monthOf("Oct 2026")).toBeNull();
  });
});

describe("guard: no rollup writer input reads a per-session cost value (#4826)", () => {
  test("the attribution module never reads a cost field and imports no session source", () => {
    const source = readFileSync(join(import.meta.dir, "../src/spend-dollars.ts"), "utf8");
    // No property read of a cost field anywhere in the writer.
    expect(source).not.toMatch(/\.cost\b/);
    expect(source).not.toMatch(/sessionCost|perSessionCost/);
    // The writer's inputs come from the charges pipeline only — no
    // session/usage source is imported.
    expect(source).not.toMatch(/from\s+["'][^"']*opencode/);
    expect(source).not.toMatch(/from\s+["'][^"']*sources\//);
  });

  test("a cost-shaped extra field on an input cannot leak into the dollars", () => {
    const poisoned = {
      service: "Anthropic",
      at: "2026-10-14",
      amountUsd: 20,
      cost: 9999,
    } as unknown as Parameters<typeof attributePaygDollars>[0][number];
    const rollups = attributePaygDollars([poisoned], [
      { service: "Anthropic", month: "2026-10", projectRef: "proj/pm", epicRef: "epic/1184" },
    ]);
    expect(rollups[0]!.projectDollars).toEqual([
      { projectRef: "proj/pm", epicRef: "epic/1184", dollarsUsd: 20 },
    ]);
    expect(rollups[0]!.rollupTotalUsd).toBe(20);
  });
});
