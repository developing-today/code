// "-direct" provider display-name rule + the manual quota refresh
// control (#6125, Drew 2026-11-05). The rule is markup-only: display
// strings change, ids/keys never do.
import { describe, expect, test } from "bun:test";
import type { Provenance, QuotaView } from "../src/model.ts";
import { providerDisplayName, providerDisplayNames } from "../src/provider-display.ts";
import { renderQuota } from "../src/render.ts";

describe("providerDisplayNames (#6125)", () => {
  test("strips -direct when no non-direct provider with the base name exists", () => {
    const names = providerDisplayNames(["vibe-direct", "antigravity-direct", "mistral", "deepseek"]);
    expect(names.get("vibe-direct")).toBe("vibe");
    expect(names.get("antigravity-direct")).toBe("antigravity");
  });

  test("keeps -direct when a non-direct provider with the base name exists in the same set", () => {
    const names = providerDisplayNames(["openai", "openai-direct"]);
    expect(names.get("openai-direct")).toBe("openai-direct");
    expect(names.get("openai")).toBe("openai");
  });

  test("the decision is per-set, not a hardcoded list", () => {
    // Same name, different set: stripped in one, kept in the other.
    expect(providerDisplayName("openai-direct", ["openai-direct", "mistral"])).toBe("openai");
    expect(providerDisplayName("openai-direct", ["openai-direct", "openai"])).toBe("openai-direct");
  });

  test("strips the leading-underscore ownership marker (#1823, Drew 2026-11-07)", () => {
    const names = providerDisplayNames(["_claude", "_antigravity.gemini", "_antigravity.claude-gpt", "meta"]);
    expect(names.get("_claude")).toBe("claude");
    expect(names.get("_antigravity.gemini")).toBe("antigravity.gemini");
    expect(names.get("_antigravity.claude-gpt")).toBe("antigravity.claude-gpt");
    expect(names.get("meta")).toBe("meta");
    // Keys stay the exact canonical input strings — ids are never rewritten.
    expect([...names.keys()].sort()).toEqual(["_antigravity.claude-gpt", "_antigravity.gemini", "_claude", "meta"]);
  });

  test("non-direct names are untouched", () => {
    const names = providerDisplayNames(["mistral", "openai.main", "cline-pass", "direct"]);
    expect(names.get("mistral")).toBe("mistral");
    expect(names.get("openai.main")).toBe("openai.main");
    expect(names.get("cline-pass")).toBe("cline-pass");
    expect(names.get("direct")).toBe("direct");
  });

  test("a base provider present only as a dotted sibling still blocks stripping", () => {
    // "openai.main" is a non-direct openai provider in the set.
    const names = providerDisplayNames(["openai.main", "openai-direct"]);
    expect(names.get("openai-direct")).toBe("openai-direct");
  });

  test("dotted pool ids keep their suffix; only the provider head is judged", () => {
    const names = providerDisplayNames(["antigravity-direct.gemini", "antigravity-direct.claude-gpt", "meta"]);
    expect(names.get("antigravity-direct.gemini")).toBe("antigravity.gemini");
    expect(names.get("antigravity-direct.claude-gpt")).toBe("antigravity.claude-gpt");
  });

  test("input strings are the map keys, unchanged (ids used for data/keys are never rewritten)", () => {
    const input = ["vibe-direct", "antigravity-direct.gemini"];
    const names = providerDisplayNames(input);
    expect([...names.keys()]).toEqual(input);
  });
});

const PROVENANCE: Provenance = {
  board: "test",
  roadmap: "test",
  items: "test",
  comments: "test",
  quota: "test",
  usage: "test",
  attention: "test",
  orchestration: "test",
  dns: "test",
  registry: "test",
  aa: "test",
};

function quotaView(pools: QuotaView["pools"], extra: Partial<QuotaView> = {}): QuotaView {
  return {
    pools,
    burn: [],
    choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] },
    ...extra,
  };
}

function pool(id: string, provider: string): QuotaView["pools"][number] {
  return { id, provider, plan: "plan", state: "healthy", windows: [] };
}

describe("renderQuota display rule + refresh control (#6125)", () => {
  test("choices pool ids render with -direct stripped when the base provider is absent", () => {
    const view = quotaView([pool("antigravity-direct.gemini", "Antigravity"), pool("meta", "Meta")], {
      choices: { hasHeadroom: ["antigravity-direct.gemini"], burnBeforeReset: [], doNotStart: ["meta"] },
    });
    const html = renderQuota(view, PROVENANCE);
    expect(html).toContain("antigravity.gemini");
    expect(html).not.toContain("antigravity-direct.gemini");
  });

  test("choices keep -direct when the base provider is in the rendered set", () => {
    const view = quotaView([pool("openai", "OpenAI"), pool("openai-direct", "openai-direct")], {
      choices: { hasHeadroom: ["openai-direct"], burnBeforeReset: [], doNotStart: [] },
    });
    const html = renderQuota(view, PROVENANCE);
    expect(html).toContain("openai-direct");
  });

  test("the Refresh quota button is on the quota surface and wires the manual route + generatedAt polling", () => {
    const view = quotaView([pool("meta", "Meta")], {
      gatherer: {
        generatedAt: "2026-11-05T07:32:27.650Z",
        source: "quota-current.json",
        tickAt: "2026-11-05T07:32:27.650Z",
        ok: true,
        providers: [],
      },
    });
    const html = renderQuota(view, PROVENANCE, undefined, undefined, {
      stale: true,
      refreshing: false,
      triggered: false,
      lastTriggerAt: null,
    });
    expect(html).toContain("Refresh quota");
    expect(html).toContain('data-quota-refresh-btn');
    expect(html).toContain('data-quota-generated-at="2026-11-05T07:32:27.650Z"');
    expect(html).toContain('action="/quota/refresh"');
    expect(html).toContain("/api/quota/refresh");
    expect(html).toContain("/api/quota.json");
    // Failure is surfaced, never faked.
    expect(html).toContain("refresh did not complete");
    expect(html).toContain("refresh failed");
  });
});
