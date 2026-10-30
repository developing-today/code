// #1057: charges auto-matcher — cross-source dedupe, service match,
// source labels, unmatched SaaS surfacing, rerun determinism.
import { describe, expect, test } from "bun:test";
import { isSaasLooking, matchCharges, normalizeMerchantName, type ChargeCandidate } from "../src/charges-match.ts";

const SERVICES = [
  { name: "Anthropic", aliases: ["Claude"] },
  { name: "OpenAI", aliases: ["ChatGPT"] },
  { name: "Cloudflare" },
];

describe("normalizeMerchantName", () => {
  test("strips descriptors, id tokens, and filler", () => {
    expect(normalizeMerchantName("METAPAY*UC5DPA22Y4 US")).toBe("METAPAY");
    expect(normalizeMerchantName("OPENAI *CHATGPT SUB")).toBe("OPENAI CHATGPT SUB");
    expect(normalizeMerchantName("CURSOR.COM")).toBe("CURSOR");
    expect(normalizeMerchantName("WALMART STORE 1234")).toBe("WALMART STORE");
  });
});

describe("matchCharges", () => {
  test("a charge seen by both Plaid and Gmail is one row, Plaid of record", () => {
    const candidates: ChargeCandidate[] = [
      { source: "gmail", at: "2026-10-27", merchant: "MetaPay", amountUsd: 20 },
      { source: "plaid", at: "2026-10-28", merchant: "METAPAY*UC5DPA22Y4 US", amountUsd: 20, method: "Visa •7535" },
    ];
    const { rows } = matchCharges(candidates, SERVICES);
    expect(rows).toEqual([
      { at: "2026-10-28", label: "METAPAY*UC5DPA22Y4 US", amountUsd: 20, method: "Visa •7535", source: "Plaid" },
    ]);
  });

  test("same merchant and amount outside the window is two charges", () => {
    const candidates: ChargeCandidate[] = [
      { source: "plaid", at: "2026-10-01", merchant: "NETFLIX", amountUsd: 15.99 },
      { source: "gmail", at: "2026-10-20", merchant: "Netflix", amountUsd: 15.99 },
    ];
    expect(matchCharges(candidates, SERVICES).rows).toHaveLength(2);
  });

  test("different amounts are different charges even on the same day", () => {
    const candidates: ChargeCandidate[] = [
      { source: "plaid", at: "2026-10-05", merchant: "STRIPE", amountUsd: 5 },
      { source: "plaid", at: "2026-10-05", merchant: "STRIPE", amountUsd: 9 },
    ];
    expect(matchCharges(candidates, SERVICES).rows).toHaveLength(2);
  });

  test("matched charges take the service name as their label", () => {
    const candidates: ChargeCandidate[] = [
      { source: "plaid", at: "2026-10-11", merchant: "ANTHROPIC PBC", amountUsd: 100 },
    ];
    const { rows, unmatchedSaas } = matchCharges(candidates, SERVICES);
    expect(rows[0]?.label).toBe("Anthropic");
    expect(unmatchedSaas).toEqual([]);
  });

  test("alias match: a ChatGPT charge matches the OpenAI service", () => {
    const candidates: ChargeCandidate[] = [
      { source: "gmail", at: "2026-10-09", merchant: "ChatGPT Subscription", amountUsd: 20 },
    ];
    const { rows } = matchCharges(candidates, SERVICES);
    expect(rows[0]?.label).toBe("OpenAI");
    expect(rows[0]?.source).toBe("Gmail alert");
  });

  test("unmatched SaaS-looking charges surface once, deduped", () => {
    const candidates: ChargeCandidate[] = [
      { source: "plaid", at: "2026-10-12", merchant: "CURSOR.COM", amountUsd: 20 },
      { source: "gmail", at: "2026-10-12", merchant: "Cursor", amountUsd: 20 },
    ];
    const { rows, unmatchedSaas } = matchCharges(candidates, SERVICES);
    expect(rows).toHaveLength(1);
    expect(unmatchedSaas).toHaveLength(1);
    expect(unmatchedSaas[0]?.normalized).toBe("CURSOR");
  });

  test("unmatched non-SaaS charges are rows but never surface", () => {
    const candidates: ChargeCandidate[] = [
      { source: "plaid", at: "2026-10-13", merchant: "WALMART STORE 1234", amountUsd: 54.12 },
    ];
    const { rows, unmatchedSaas } = matchCharges(candidates, SERVICES);
    expect(rows).toHaveLength(1);
    expect(unmatchedSaas).toEqual([]);
    expect(isSaasLooking("WALMART STORE")).toBe(false);
  });

  test("non-positive candidates (refunds/inflows) are ignored", () => {
    const candidates: ChargeCandidate[] = [
      { source: "plaid", at: "2026-10-14", merchant: "OPENAI REFUND", amountUsd: -20 },
      { source: "plaid", at: "2026-10-14", merchant: "PAYROLL", amountUsd: 0 },
    ];
    expect(matchCharges(candidates, SERVICES)).toEqual({ rows: [], unmatchedSaas: [] });
  });

  test("re-running the same period is deterministic and adds nothing", () => {
    const candidates: ChargeCandidate[] = [
      { source: "plaid", at: "2026-10-28", merchant: "METAPAY*UC5DPA22Y4 US", amountUsd: 20, method: "Visa •7535" },
      { source: "gmail", at: "2026-10-27", merchant: "MetaPay", amountUsd: 20 },
      { source: "gmail", at: "2026-10-09", merchant: "ChatGPT Subscription", amountUsd: 20 },
      { source: "plaid", at: "2026-10-12", merchant: "CURSOR.COM", amountUsd: 20 },
    ];
    const first = matchCharges(candidates, SERVICES);
    const second = matchCharges(candidates, SERVICES);
    expect(second).toEqual(first);
    expect(first.rows).toHaveLength(3);
  });
});
