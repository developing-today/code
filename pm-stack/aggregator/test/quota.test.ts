import { describe, expect, test } from "bun:test";
import { poolState, windowPercent } from "../src/derive.ts";
import { disabledProvidersFromConfig } from "../src/aa.ts";
import { extractQuotaPools, poolProviderId, poolProviderIds } from "../src/quota-extract.ts";
import { computeChoices } from "../src/quota-guidance.ts";

// A synthetic quota view carrying the same dated-read phrasing the
// stewardship view uses, so the extractors are pinned to the format.
const VIEW = `
## Service details (a)

### Claude
- **Usage / dates:** Hitting limits. **Newer dated read:** session limit hit and reset 05:30 CDT today; **weekly 68% used, weekly reset Wed 2026-10-28 14:00 CDT**, projected to run out Mon evening.

### ChatGPT — main account plans
- **Usage / dates:** Hitting limits. **Newer dated read, Codex pool specifically: 7-day 11% used, 1 reset credit banked, resets Oct 30, 18:01 UTC.**

### Gemini / Google AI Pro
- **Usage / dates:** unknown. **Newer dated read, Antigravity pools specifically: Gemini weekly 99.1% remaining; Claude+GPT pools 100% remaining.**

### OpenCode Go ×2 workspaces
- **Usage / dates:** **Personal — monthly 60% used**, hit a funds wall today; monthly reset Nov 3, 03:02 UTC. **Lab — monthly 50% used, weekly 0%**; monthly reset Nov 7, 05:55 UTC.

### ClinePass
- **Usage / dates:** **Measured 2026-10-25 ~04:48 CDT** (Dashboard > Subscription, USAGE LIMITS): 5-hour **0%** used; weekly **26%** used (resets in 5d 23h); monthly **69%** used (resets in 11d 15h).

### Command Code
- **Usage / dates:** **Newer dated read: $6.57 of $10 used ($3.43 left); 5h 1.2%, weekly 8.8%; period ends Nov 2, 23:04 UTC.**

### Meta Model API
- **Usage / dates:** weekly quota **46% used at 03:20 CDT**, then burned to empty by morning sessions and spilled to PAYG. **Weekly reset today 2026-10-25 19:00 CDT.**
`;

const NOW = new Date("2026-10-25T15:00:00Z"); // 10:00 CDT, before the Meta reset

describe("quota extractors", () => {
  const pools = extractQuotaPools(VIEW, NOW);
  const byId = new Map(pools.map((p) => [p.id, p]));

  test("claude weekly read", () => {
    const p = byId.get("anthropic:claude-pro")!;
    expect(p.windows[0].percent).toBe(68);
    expect(p.windows[0].resetsAt).toBe("2026-10-28T14:00:00-05:00");
    expect(p.state).toBe("healthy");
  });

  test("codex 7-day read", () => {
    const p = byId.get("openai:codex")!;
    expect(p.windows[0].percent).toBe(11);
    expect(p.windows[0].resetsAt).toBe("2026-10-30T18:01:00.000Z");
  });

  test("meta is walled until the weekly reset — and it is the only Meta pool", () => {
    const p = byId.get("meta:high-usage")!;
    expect(p.state).toBe("walled");
    expect(p.wallLiftsAt).toBe("2026-10-25T19:00:00-05:00");
    expect(pools.some((x) => /muse plan|power/i.test(`${x.id} ${x.plan}`) && x.id !== "meta:high-usage")).toBe(false);
    expect(pools.filter((x) => x.provider.includes("Meta")).length).toBe(1);
  });

  test("meta billing-blocked read walls the pool with a truthful note, even after the reset", () => {
    const blockedView = VIEW.replace(
      "weekly quota **46% used at 03:20 CDT**, then burned to empty by morning sessions and spilled to PAYG. **Weekly reset today 2026-10-25 19:00 CDT.**",
      "**Freshest dated read (2026-10-26, laptop meta-provider probes 14:30–14:42Z): API returns 402 billing_not_configured (billing-verification wall) on all 11 probes — pool BLOCKED, no usable quota.** Weekly reset was today 2026-10-25 19:00 CDT (that reset has since passed).",
    );
    const p = extractQuotaPools(blockedView, new Date("2026-10-26T17:00:00Z")).find((x) => x.id === "meta:high-usage")!;
    expect(p.state).toBe("walled");
    expect(p.note).toContain("billing_not_configured");
    expect(p.note).toContain("2026-10-26");
    expect(p.wallLiftsAt).toBeUndefined();
  });

  test("opencode go resets are anchored to each workspace's own paragraph (#1626)", () => {
    const goView = (section: string) => "\n### OpenCode Go \u00d72 workspaces\n" + section + "\n";
    const go = (view: string) => {
      const m = new Map(extractQuotaPools(view, NOW).map((p) => [p.id, p]));
      return { personal: m.get("opencode-go:personal")!, lab: m.get("opencode-go:lab")! };
    };

    // Control: both paragraphs carry their own reset.
    const both = go(
      goView(
        "- **Personal \u2014 monthly 60% used**; monthly reset Nov 3, 03:02 UTC.\n\n" +
          "- **Lab \u2014 monthly 50% used, weekly 0%**; monthly reset Nov 9, 05:55 UTC.",
      ),
    );
    expect(both.personal.windows[0].resetsAt).toBe("2026-11-03T03:02:00.000Z");
    expect(both.lab.windows[0].resetsAt).toBe("2026-11-09T05:55:00.000Z");

    // Personal's paragraph has no reset: Personal gets null — never Lab's —
    // and Lab still gets its own.
    const missing = go(
      goView(
        "- **Personal \u2014 monthly 60% used**, no reset read this time.\n\n" +
          "- **Lab \u2014 monthly 50% used, weekly 0%**; monthly reset Nov 9, 05:55 UTC.",
      ),
    );
    expect(missing.personal.windows[0].resetsAt).toBeNull();
    expect(missing.lab.windows[0].resetsAt).toBe("2026-11-09T05:55:00.000Z");

    // A stray reset mention in an earlier paragraph is never assigned.
    const stray = go(
      goView(
        "- Earlier note: a monthly reset Oct 5, 01:00 UTC was observed last cycle.\n\n" +
          "- **Personal \u2014 monthly 60% used**; monthly reset Nov 3, 03:02 UTC.\n\n" +
          "- **Lab \u2014 monthly 50% used, weekly 0%**; monthly reset Nov 9, 05:55 UTC.",
      ),
    );
    expect(stray.personal.windows[0].resetsAt).toBe("2026-11-03T03:02:00.000Z");
    expect(stray.lab.windows[0].resetsAt).toBe("2026-11-09T05:55:00.000Z");
  });

  test("opencode go workspaces are separate pools in USD", () => {
    const personal = byId.get("opencode-go:personal")!;
    expect(personal.windows[0]).toMatchObject({ unit: "usd", limit: 60, used: 36 });
    expect(personal.windows[0].resetsAt).toBe("2026-11-03T03:02:00.000Z");
    const lab = byId.get("opencode-go:lab")!;
    expect(lab.windows[0].used).toBe(30);
    expect(lab.windows[1]).toMatchObject({ name: "weekly", used: 0 });
  });

  test("clinepass windows with relative resets anchored to the measured read", () => {
    const p = byId.get("cline-pass:cline-pass")!;
    expect(p.windows.map((w) => w.percent)).toEqual([0, 26, 69]);
    expect(p.windows[1].resetsAt).toBe("2026-10-31T08:48:00.000Z");
    expect(p.windows[2].resetsAt).toBe("2026-11-06T00:48:00.000Z");
  });

  test("command code credits + short windows", () => {
    const p = byId.get("commandcode:go")!;
    expect(p.windows[0]).toMatchObject({ unit: "usd", used: 6.57, limit: 10 });
    expect(p.windows[1].percent).toBe(1.2);
    expect(p.windows[2].percent).toBe(8.8);
  });

  test("antigravity pools are independent, remaining -> used", () => {
    expect(byId.get("antigravity:gemini")!.windows[0].percent).toBeCloseTo(0.9);
    expect(byId.get("antigravity:claude-gpt")!.windows[0].percent).toBe(0);
  });

  test("unreadable doc fails soft: identity + note, no invented numbers", () => {
    const empty = extractQuotaPools("no reads here", NOW);
    expect(empty.length).toBeGreaterThan(0);
    for (const p of empty) {
      expect(p.windows).toEqual([]);
      expect(p.note).toBeTruthy();
    }
  });
});

describe("windowPercent + guidance", () => {
  test("explicit percent wins when the limit is unknown", () => {
    expect(windowPercent({ name: "w", unit: "other", limit: null, used: 0, resetsAt: null, percent: 42 })).toBe(42);
    expect(windowPercent({ name: "w", unit: "other", limit: null, used: 0, resetsAt: null })).toBe(100);
  });

  test("choices bucket pools by state and reset proximity", () => {
    const pools = extractQuotaPools(VIEW, NOW);
    const choices = computeChoices(pools, NOW.getTime());
    expect(choices.doNotStart).toContain("meta:high-usage");
    expect(choices.hasHeadroom).toContain("openai:codex");
    expect(choices.hasHeadroom).not.toContain("meta:high-usage");
    // Claude resets in >24h; nothing in this fixture resets within a day
    // with headroom, so burn-before-reset is empty.
    expect(choices.burnBeforeReset).toEqual([]);
    for (const p of pools) expect(poolState(p)).toBe(p.state === "walled" ? "walled" : poolState(p));
  });
});

describe("disabled_providers enforcement (#1114)", () => {
  const KEEPERS = ["cloudflare-gateway", "cloudflare-workers-ai", "code-design", "design-code", "open-design"];

  test("config parsing reuses the AA JSONC parse: comments tolerated, junk fails soft", () => {
    const cfg = `{
      // deployed config shape
      "disabled_providers": ["openrouter", "github-copilot", 42, null],
      "provider": {}
    }`;
    expect([...disabledProvidersFromConfig(cfg)].sort()).toEqual(["github-copilot", "openrouter"]);
    expect(disabledProvidersFromConfig("not json").size).toBe(0);
    expect(disabledProvidersFromConfig("{}").size).toBe(0);
  });

  test("a disabled provider's pool is filtered at assembly and visibly noted; guidance never names it", () => {
    const filtered: Array<[string, string]> = [];
    const disabled = new Set(["openrouter", "github-copilot", "openai", "meta"]);
    const pools = extractQuotaPools(VIEW, NOW, disabled, (poolId, providerId) => filtered.push([poolId, providerId]));
    const ids = pools.map((p) => p.id);
    expect(ids).not.toContain("openai:codex");
    expect(ids).not.toContain("meta:high-usage");
    expect(filtered).toEqual([
      ["meta:high-usage", "meta"],
      ["openai:codex", "openai"],
    ]);
    const choices = computeChoices(pools, NOW.getTime());
    const named = [...choices.hasHeadroom, ...choices.burnBeforeReset, ...choices.doNotStart];
    expect(named).not.toContain("openai:codex");
    expect(named).not.toContain("meta:high-usage");
    for (const id of named) expect(disabled.has(poolProviderId(id))).toBe(false);
  });

  test("the Claude pool's door is claude-code: disabling it filters anthropic:claude-pro", () => {
    expect(poolProviderId("anthropic:claude-pro")).toBe("claude-code");
    const filtered: Array<[string, string]> = [];
    const disabled = disabledProvidersFromConfig(
      JSON.stringify({ disabled_providers: ["claude-code"], provider: {} }),
    );
    const pools = extractQuotaPools(VIEW, NOW, disabled, (poolId, providerId) => filtered.push([poolId, providerId]));
    expect(pools.map((p) => p.id)).not.toContain("anthropic:claude-pro");
    expect(filtered).toContainEqual(["anthropic:claude-pro", "claude-code"]);
    const choices = computeChoices(pools, NOW.getTime());
    const named = [...choices.hasHeadroom, ...choices.burnBeforeReset, ...choices.doNotStart];
    expect(named).not.toContain("anthropic:claude-pro");
  });

  test("the issue's fixture disabled set (openrouter/copilot) emits no pool for them", () => {
    const disabled = disabledProvidersFromConfig(
      JSON.stringify({ disabled_providers: ["openrouter", "github-copilot"], provider: {} }),
    );
    const pools = extractQuotaPools(VIEW, NOW, disabled);
    for (const p of pools) {
      expect(poolProviderId(p.id)).not.toBe("openrouter");
      expect(poolProviderId(p.id)).not.toBe("github-copilot");
    }
  });

  test("keeper set can never appear in pools or guidance choices", () => {
    const pools = extractQuotaPools(VIEW, NOW, new Set(KEEPERS));
    const choices = computeChoices(pools, NOW.getTime());
    const named = [...choices.hasHeadroom, ...choices.burnBeforeReset, ...choices.doNotStart];
    for (const keeper of KEEPERS) {
      expect(pools.map((p) => poolProviderId(p.id))).not.toContain(keeper);
      expect(named.map((id) => poolProviderId(id))).not.toContain(keeper);
      expect(named).not.toContain(keeper);
    }
  });

  test("antigravity pools answer to antigravity-ls/antigravity-agy/google jointly: filtered only when every door is disabled", () => {
    // There is no "antigravity" provider: poolFor attributes the two
    // antigravity pools to these three doors (Codex P2 on PR #2377).
    for (const id of ["antigravity:gemini", "antigravity:claude-gpt"]) {
      expect(poolProviderIds(id)).toEqual(["antigravity-ls", "antigravity-agy", "google"]);
    }
    // One usable door remaining (the deployed set disables google alone)
    // keeps both pools.
    const partial = extractQuotaPools(VIEW, NOW, new Set(["antigravity-ls", "antigravity-agy"])).map((p) => p.id);
    expect(partial).toContain("antigravity:gemini");
    expect(partial).toContain("antigravity:claude-gpt");
    const deployedLike = extractQuotaPools(VIEW, NOW, new Set(["google"])).map((p) => p.id);
    expect(deployedLike).toContain("antigravity:gemini");
    expect(deployedLike).toContain("antigravity:claude-gpt");
    // Every door disabled: both pools filtered at assembly, visibly
    // noted, and guidance never names them.
    const filtered: Array<[string, string]> = [];
    const disabled = new Set(["antigravity-ls", "antigravity-agy", "google"]);
    const pools = extractQuotaPools(VIEW, NOW, disabled, (poolId, providerId) => filtered.push([poolId, providerId]));
    const ids = pools.map((p) => p.id);
    expect(ids).not.toContain("antigravity:gemini");
    expect(ids).not.toContain("antigravity:claude-gpt");
    expect(filtered).toContainEqual(["antigravity:gemini", "antigravity-ls,antigravity-agy,google"]);
    expect(filtered).toContainEqual(["antigravity:claude-gpt", "antigravity-ls,antigravity-agy,google"]);
    const choices = computeChoices(pools, NOW.getTime());
    const named = [...choices.hasHeadroom, ...choices.burnBeforeReset, ...choices.doNotStart];
    expect(named).not.toContain("antigravity:gemini");
    expect(named).not.toContain("antigravity:claude-gpt");
  });

  test("exact-match only: near-miss disabled ids do not filter lookalike pools", () => {
    // The deployed disabled set contains openai-codex/chatgpt (not openai),
    // open-design (not opendesign) and opencode (not opencode-go).
    const disabled = new Set(["openai-codex", "chatgpt", "open-design", "opencode"]);
    const ids = extractQuotaPools(VIEW, NOW, disabled).map((p) => p.id);
    expect(ids).toContain("openai:codex");
    expect(ids).toContain("opendesign:go");
    expect(ids).toContain("opencode-go:personal");
    expect(ids).toContain("opencode-go:lab");
  });
});
