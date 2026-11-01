import { describe, expect, test } from "bun:test";
import { chargeServiceCatalog } from "../src/charges-catalog.ts";
import { matchCharges } from "../src/charges-match.ts";
import { extractQuotaPools } from "../src/quota-extract.ts";
import { extractSaasServices } from "../src/saas-extract.ts";

describe("charge service catalog", () => {
  test("matches quota providers and merchant aliases alongside SaaS services", () => {
    const catalog = chargeServiceCatalog(extractSaasServices(""), extractQuotaPools(""));
    const merchants = ["ANTHROPIC", "OPENAI", "CLAUDE", "CHATGPT", "VERCEL", "PADDLE MYSTERY"];
    const result = matchCharges(
      merchants.map((merchant, i) => ({
        source: "plaid" as const,
        at: "2026-10-01",
        merchant,
        amountUsd: i + 1,
      })),
      catalog,
    );
    expect(result.rows.map((row) => row.label)).toEqual(expect.arrayContaining(["Anthropic", "OpenAI", "Vercel"]));
    expect(result.rows.filter((row) => row.label === "Anthropic")).toHaveLength(2);
    expect(result.rows.filter((row) => row.label === "OpenAI")).toHaveLength(2);
    expect(result.unmatchedSaas.map((row) => row.merchant)).not.toContain("ANTHROPIC");
    expect(result.unmatchedSaas.map((row) => row.merchant)).not.toContain("OPENAI");
    expect(result.unmatchedSaas.map((row) => row.merchant)).toContain("PADDLE MYSTERY");
  });

  test("deduplicates providers and respects filtered pools", () => {
    const catalog = chargeServiceCatalog([], extractQuotaPools("", new Date(), new Set(["claude-code"])));
    expect(catalog.filter((entry) => entry.name === "OpenCode Go")).toHaveLength(1);
    expect(catalog.some((entry) => entry.name === "Anthropic")).toBe(false);
  });
});
