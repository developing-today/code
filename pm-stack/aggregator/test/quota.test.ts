import { describe, expect, test } from "bun:test";
import { poolState, windowPercent } from "../src/derive.ts";
import { disabledProvidersFromConfig } from "../src/aa.ts";
import { extractQuotaPools, POOL_SPECS } from "../src/quota-extract.ts";
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
- **Usage / dates:** **drewrypope — monthly 60% used**, hit a funds wall today; monthly reset Nov 3, 03:02 UTC. **popesdrew — monthly 50% used, weekly 0%**; monthly reset Nov 7, 05:55 UTC.

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

  test("claude and meta invalid reset captures yield resetsAt null and preserve the read (#2470)", () => {
    const invalidResets = ["2026-13-28 14:00", "2026-02-30 14:00", "2026-10-39 14:00", "2026-10-28 99:99"];
    for (const reset of invalidResets) {
      const claudeView = `\n### Claude\n- weekly 68% used, weekly reset Wed ${reset} CDT\n`;
      const claude = extractQuotaPools(claudeView, NOW).find((p) => p.id === "anthropic:claude-pro")!;
      expect(claude.windows).toHaveLength(1);
      expect(claude.windows[0].percent).toBe(68);
      expect(claude.windows[0].resetsAt).toBeNull();

      const metaView = `\n### Meta Model API\n- weekly quota 46% used at 03:20 CDT. Weekly reset today ${reset} CDT.\n`;
      const meta = extractQuotaPools(metaView, NOW).find((p) => p.id === "meta")!;
      expect(meta.windows).toHaveLength(1);
      expect(meta.windows[0].percent).toBe(46);
      expect(meta.windows[0].resetsAt).toBeNull();
      expect(meta.wallLiftsAt).toBeUndefined();
    }
  });

  test("meta feb-30 reset neither discards the read nor invents a wall lift (#2470)", () => {
    const burnedView = `\n### Meta Model API\n- weekly quota 46% used at 03:20 CDT, then burned to empty by morning sessions. Weekly reset today 2026-02-30 19:00 CDT.\n`;
    const meta = extractQuotaPools(burnedView, NOW).find((p) => p.id === "meta")!;
    expect(meta.windows).toHaveLength(1);
    expect(meta.windows[0].percent).toBe(46);
    expect(meta.windows[0].resetsAt).toBeNull();
    expect(meta.wallLiftsAt).toBeUndefined();
    expect(meta.state).toBe("healthy");
  });

  test("codex 7-day read", () => {
    const p = byId.get("_openai.main")!;
    expect(p.windows[0].percent).toBe(11);
    expect(p.windows[0].resetsAt).toBe("2026-10-30T18:01:00.000Z");
  });

  test("month-based resets normalize month case and preserve reads for invalid dates across extractors", () => {
    const consumers = [
      {
        id: "_openai.main",
        view: (reset: string) => `\n### ChatGPT — main account plans\n7-day 42% used; resets ${reset}, 18:01 UTC.\n`,
        window: { name: "7-day", percent: 42 },
        extraWindows: 0,
      },
      {
        id: "opencode-go.drewrypope",
        view: (reset: string) =>
          `\n### OpenCode Go ×2 workspaces\n- **drewrypope — monthly 60% used**; monthly reset ${reset}, 18:01 UTC.\n`,
        window: { name: "monthly", unit: "usd", limit: 60, used: 36, percent: 60 },
        extraWindows: 0,
      },
      {
        id: "_commandcode",
        view: (reset: string) =>
          `\n### Command Code\n$6.57 of $10 used; 5h 1.2%, weekly 8.8%; period ends ${reset}, 18:01 UTC.\n`,
        window: { name: "monthly credits", unit: "usd", limit: 10, used: 6.57, percent: 65.7 },
        extraWindows: 2,
      },
    ];
    const resets = [
      { input: "Oct 30", expected: "2026-10-30T18:01:00.000Z" },
      { input: "oct 30", expected: "2026-10-30T18:01:00.000Z" },
      { input: "OCT 30", expected: "2026-10-30T18:01:00.000Z" },
      { input: "Foo 30", expected: null },
      { input: "Oct 39", expected: null },
    ];

    for (const consumer of consumers) {
      for (const reset of resets) {
        const pool = extractQuotaPools(consumer.view(reset.input), NOW).find((p) => p.id === consumer.id)!;
        expect(pool.windows).toHaveLength(1 + consumer.extraWindows);
        expect(pool.windows[0]).toMatchObject(consumer.window);
        expect(pool.windows[0].resetsAt).toBe(reset.expected);
        if (consumer.id === "_commandcode") {
          expect(pool.windows.slice(1).map((window) => window.percent)).toEqual([1.2, 8.8]);
        }
      }
    }
  });

  test("valid month-based resets still roll forward when more than 30 days past", () => {
    const pastDate = "Sep 24";
    const cases = [
      {
        id: "_openai.main",
        view: `\n### ChatGPT — main account plans\n7-day 42% used; resets ${pastDate}, 18:01 UTC.\n`,
      },
      {
        id: "opencode-go.drewrypope",
        view: `\n### OpenCode Go ×2 workspaces\n- **drewrypope — monthly 60% used**; monthly reset ${pastDate}, 18:01 UTC.\n`,
      },
      {
        id: "_commandcode",
        view: `\n### Command Code\n$6.57 of $10 used; 5h 1.2%, weekly 8.8%; period ends ${pastDate}, 18:01 UTC.\n`,
      },
    ];
    for (const { id, view } of cases) {
      const pool = extractQuotaPools(view, NOW).find((p) => p.id === id)!;
      expect(pool.windows[0].resetsAt).toBe("2027-09-24T18:01:00.000Z");
    }
  });

  test("alternate ChatGPT account is its own pool — never merged into the main pool", () => {
    const p = byId.get("_openai.alt")!;
    expect(p).toBeDefined();
    expect(p.provider).toBe("OpenAI");
    // This view has no "### ChatGPT (Alternate)" dated-read section, so
    // the pool renders as a shell; the gatherer overlay supplies the
    // live windows (see quota-current.test.ts).
    expect(p.windows).toEqual([]);
  });

  test("main and alternate ChatGPT sections never cross-read, in either order (#8005)", () => {
    const mainSection = "### ChatGPT — main account plans\n- 7-day 11% used, resets Oct 30, 18:01 UTC.\n";
    const altSection = "### ChatGPT (Alternate)\n- 7-day 42% used.\n";
    for (const view of ["\n" + altSection + "\n" + mainSection, "\n" + mainSection + "\n" + altSection]) {
      const m = new Map(extractQuotaPools(view, NOW).map((p) => [p.id, p]));
      expect(m.get("_openai.main")!.windows[0].percent).toBe(11);
      expect(m.get("_openai.alt")!.windows[0].percent).toBe(42);
    }
  });

  test("meta is walled until the weekly reset — and it is the only Meta pool", () => {
    const p = byId.get("meta")!;
    expect(p.state).toBe("walled");
    expect(p.wallLiftsAt).toBe("2026-10-25T19:00:00-05:00");
    expect(pools.some((x) => /muse plan|power/i.test(`${x.id} ${x.plan}`) && x.id !== "meta")).toBe(false);
    expect(pools.filter((x) => x.provider.includes("Meta")).length).toBe(1);
  });

  test("meta billing-blocked prose no longer promotes into a wall (Drew, 2026-11-03)", () => {
    // The extractMeta promotion branch is REMOVED by Drew's
    // pass-through ruling: a dated billing_not_configured mention in
    // steward prose once fabricated a walled 100% window + BLOCKED
    // note that outranked the live read for days. This view carries
    // no parseable dated % read, so the pool fail-softs to its shell;
    // a real billing wall now arrives as the quota read's own error
    // state through overlayQuotaCurrent (quota-current.test.ts).
    const blockedView = VIEW.replace(
      "weekly quota **46% used at 03:20 CDT**, then burned to empty by morning sessions and spilled to PAYG. **Weekly reset today 2026-10-25 19:00 CDT.**",
      "**Freshest dated read (2026-10-26, laptop meta-provider probes 14:30–14:42Z): API returns 402 billing_not_configured (billing-verification wall) on all 11 probes — pool BLOCKED, no usable quota.** Weekly reset was today 2026-10-25 19:00 CDT (that reset has since passed).",
    );
    const p = extractQuotaPools(blockedView, new Date("2026-10-26T17:00:00Z")).find((x) => x.id === "meta")!;
    expect(p.state).toBe("healthy");
    expect(p.windows).toEqual([]);
    expect(p.note).not.toContain("billing_not_configured");
    expect(p.wallLiftsAt).toBeUndefined();
  });

  test("opencode go resets are anchored to each workspace's own paragraph (#1626)", () => {
    const goView = (section: string) => "\n### OpenCode Go \u00d72 workspaces\n" + section + "\n";
    const go = (view: string) => {
      const m = new Map(extractQuotaPools(view, NOW).map((p) => [p.id, p]));
      return { personal: m.get("opencode-go.drewrypope")!, lab: m.get("opencode-go.popesdrew")! };
    };

    // Control: both paragraphs carry their own reset.
    const both = go(
      goView(
        "- **drewrypope \u2014 monthly 60% used**; monthly reset Nov 3, 03:02 UTC.\n\n" +
          "- **popesdrew \u2014 monthly 50% used, weekly 0%**; monthly reset Nov 9, 05:55 UTC.",
      ),
    );
    expect(both.personal.windows[0].resetsAt).toBe("2026-11-03T03:02:00.000Z");
    expect(both.lab.windows[0].resetsAt).toBe("2026-11-09T05:55:00.000Z");

    // drewrypope's paragraph has no reset: drewrypope gets null — never popesdrew's —
    // and popesdrew still gets its own.
    const missing = go(
      goView(
        "- **drewrypope \u2014 monthly 60% used**, no reset read this time.\n\n" +
          "- **popesdrew \u2014 monthly 50% used, weekly 0%**; monthly reset Nov 9, 05:55 UTC.",
      ),
    );
    expect(missing.personal.windows[0].resetsAt).toBeNull();
    expect(missing.lab.windows[0].resetsAt).toBe("2026-11-09T05:55:00.000Z");

    // A stray reset mention in an earlier paragraph is never assigned.
    const stray = go(
      goView(
        "- Earlier note: a monthly reset Oct 5, 01:00 UTC was observed last cycle.\n\n" +
          "- **drewrypope \u2014 monthly 60% used**; monthly reset Nov 3, 03:02 UTC.\n\n" +
          "- **popesdrew \u2014 monthly 50% used, weekly 0%**; monthly reset Nov 9, 05:55 UTC.",
      ),
    );
    expect(stray.personal.windows[0].resetsAt).toBe("2026-11-03T03:02:00.000Z");
    expect(stray.lab.windows[0].resetsAt).toBe("2026-11-09T05:55:00.000Z");
  });

  test("opencode go workspaces are separate pools in USD", () => {
    const personal = byId.get("opencode-go.drewrypope")!;
    expect(personal.windows[0]).toMatchObject({ unit: "usd", limit: 60, used: 36 });
    expect(personal.windows[0].resetsAt).toBe("2026-11-03T03:02:00.000Z");
    const lab = byId.get("opencode-go.popesdrew")!;
    expect(lab.windows[0].used).toBe(30);
    expect(lab.windows[1]).toMatchObject({ name: "weekly", used: 0 });
  });

  test("clinepass windows with relative resets anchored to the measured read", () => {
    const p = byId.get("cline-pass")!;
    expect(p.windows.map((w) => w.percent)).toEqual([0, 26, 69]);
    expect(p.windows[1].resetsAt).toBe("2026-10-31T08:48:00.000Z");
    expect(p.windows[2].resetsAt).toBe("2026-11-06T00:48:00.000Z");
  });

  test("command code credits + short windows", () => {
    const p = byId.get("_commandcode")!;
    expect(p.windows[0]).toMatchObject({ unit: "usd", used: 6.57, limit: 10 });
    expect(p.windows[1].percent).toBe(1.2);
    expect(p.windows[2].percent).toBe(8.8);
  });

  test("command code takes the latest dated read, percentage-only format (#3410)", () => {
    const multi = `
### Command Code
- **Usage / dates:** **Newer dated read (model-efficiency plan, freshest reads 2026-10-25 morning): $6.57 of $10 used ($3.43 left); 5h 1.2%, weekly 8.8%; period ends Nov 2, 23:04 UTC.** **Freshest dated read (console, 2026-10-27 10:05 CDT, commandcode.ai Settings): monthly limit 69% used, resets Nov 2; weekly limit 14% (resets in 4d); 5-hour limit 0% ("No usage in this window yet"); extra/top-up credits $0.00 balance.**
`;
    const p = extractQuotaPools(multi, NOW).find((x) => x.id === "_commandcode")!;
    expect(p.windows.map((w) => [w.name, w.percent])).toEqual([
      ["monthly", 69],
      ["weekly", 14],
      ["5-hour", 0],
    ]);
    // No window is a dollar/balance figure: the top-up balance is not quota.
    expect(p.windows.some((w) => w.unit === "usd")).toBe(false);
  });

  test("command code renders no windows when the freshest dated read is unparseable (#3410)", () => {
    const stale = `
### Command Code
- **Usage / dates:** **Dated read (2026-10-25): $6.57 of $10 used; 5h 1.2%, weekly 8.8%; period ends Nov 2, 23:04 UTC.** **Freshest dated read (2026-10-30): monthly limit 55 percent used (the console printed the figure in words this cycle).**
`;
    const p = extractQuotaPools(stale, NOW).find((x) => x.id === "_commandcode")!;
    expect(p.windows).toEqual([]);
  });

  test("command code: a later same-day unparseable read also invalidates the earlier one (#3410)", () => {
    const sameDay = `
### Command Code
- **Usage / dates:** **Dated read (2026-10-30 09:00 CDT): monthly limit 55% used; weekly limit 20%; 5-hour limit 3%.** **Dated update (2026-10-30 12:00 CDT): monthly limit 100 percent used (the console printed the figure in words).**
`;
    const p = extractQuotaPools(sameDay, NOW).find((x) => x.id === "_commandcode")!;
    expect(p.windows).toEqual([]);
  });

  test("command code never emits the top-up balance as quota (#3410)", () => {
    const balanceOnly = `
### Command Code
- **Usage / dates:** **Dated read (2026-10-30): extra/top-up credits $0.10 balance; hub credits 0.1 flat.**
`;
    const p = extractQuotaPools(balanceOnly, NOW).find((x) => x.id === "_commandcode")!;
    expect(p.windows).toEqual([]);
  });

  test("antigravity pools are independent, remaining -> used", () => {
    expect(byId.get("_antigravity.gemini")!.windows[0].percent).toBeCloseTo(0.9);
    expect(byId.get("_antigravity.claude-gpt")!.windows[0].percent).toBe(0);
  });

  test("unreadable doc fails soft: identity + note, no invented numbers", () => {
    const empty = extractQuotaPools("no reads here", NOW);
    expect(empty.length).toBeGreaterThan(0);
    for (const p of empty) {
      expect(p.windows).toEqual([]);
      expect(p.note).toBeTruthy();
    }
  });

  test("extractor health distinguishes misses from intentional shell pools", () => {
    const empty = new Map(extractQuotaPools("no reads here", NOW).map((p) => [p.id, p]));
    for (const spec of POOL_SPECS) {
      expect(empty.get(spec.id)?.extractionHealth).toEqual({
        status: "gap",
        matched: false,
        reason: "section-missing",
        count: 0,
      });
    }
    for (const id of ["opendesign", "deepseek"]) {
      expect(empty.get(id)?.extractionHealth).toEqual({
        status: "intentional",
        matched: null,
        reason: "uninstrumented",
        count: 0,
      });
    }
  });

  test("extractor health records parsed windows and distinguishes null/throw misses", () => {
    expect(byId.get("anthropic:claude-pro")?.extractionHealth).toEqual({
      status: "matched",
      matched: true,
      count: 1,
    });
    const nullResult = extractQuotaPools("### ChatGPT — main account plans\nno quota figures", NOW).find(
      (p) => p.id === "_openai.main",
    );
    expect(nullResult?.extractionHealth).toEqual({
      status: "gap",
      matched: false,
      reason: "extractor-null",
      count: 0,
    });
    // No extractor-threw case: on current main every extractor is
    // fail-soft (the codex extractor no longer throws on an
    // unparseable reset date — it parses the window and ignores the
    // date), so the extractor-threw branch in extractQuotaPools is
    // defensive only and has no live input.
  });
});

describe("America/Chicago dated reads", () => {
  const extractClaudeReset = (date: string, time: string) =>
    extractQuotaPools(`### Claude\nweekly 68% used; weekly reset Wed ${date} ${time} CDT`).find(
      (pool) => pool.id === "anthropic:claude-pro",
    )!.windows[0].resetsAt;

  test("CST-month reads use UTC-6", () => {
    expect(extractClaudeReset("2026-12-16", "14:00")).toBe("2026-12-16T14:00:00-06:00");
    expect(Date.parse(extractClaudeReset("2026-12-16", "14:00")!)).toBe(Date.parse("2026-12-16T20:00:00.000Z"));
  });

  test("CDT-month reads keep UTC-5", () => {
    expect(extractClaudeReset("2026-07-16", "14:00")).toBe("2026-07-16T14:00:00-05:00");
    expect(Date.parse(extractClaudeReset("2026-07-16", "14:00")!)).toBe(Date.parse("2026-07-16T19:00:00.000Z"));
  });

  test("a fall-back transition date uses the post-transition offset", () => {
    expect(extractClaudeReset("2026-11-01", "03:30")).toBe("2026-11-01T03:30:00-06:00");
    expect(Date.parse(extractClaudeReset("2026-11-01", "03:30")!)).toBe(Date.parse("2026-11-01T09:30:00.000Z"));
  });

  test("Meta does not treat a winter reset as passed an hour early", () => {
    const view = `### Meta Model API\nweekly quota 46% used at 03:20 CDT, then burned to empty. Weekly reset today 2026-12-16 14:00 CDT.`;
    const pool = extractQuotaPools(view, new Date("2026-12-16T19:30:00.000Z")).find(
      (candidate) => candidate.id === "meta",
    )!;
    expect(pool.state).toBe("walled");
    expect(pool.wallLiftsAt).toBe("2026-12-16T14:00:00-06:00");
  });

  test("ClinePass relative windows anchor to a winter read in CST", () => {
    const view = `### ClinePass\nMeasured 2026-12-16 ~14:00 CDT: 5-hour **0%** used; weekly **26%** used (resets in 0d 1h); monthly **69%** used (resets in 0d 2h).`;
    const pool = extractQuotaPools(view).find((candidate) => candidate.id === "cline-pass")!;
    expect(pool.windows[1].resetsAt).toBe("2026-12-16T21:00:00.000Z");
    expect(pool.windows[2].resetsAt).toBe("2026-12-16T22:00:00.000Z");
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
    expect(choices.doNotStart).toContain("meta");
    expect(choices.hasHeadroom).toContain("_openai.main");
    expect(choices.hasHeadroom).not.toContain("meta");
    // Claude resets in >24h; nothing in this fixture resets within a day
    // with headroom, so burn-before-reset is empty.
    expect(choices.burnBeforeReset).toEqual([]);
    for (const p of pools) expect(poolState(p)).toBe(p.state === "walled" ? "walled" : poolState(p));
  });
});

describe("pass-through pool assembly (Drew, 2026-11-03)", () => {
  // Drew's ruling — "the dashboard should not be doing any of that":
  // the board is a straight display of the OpenCode quota read. The
  // #1114 disabled_providers filter and its FILTER_EXEMPT hack are
  // gone; extractQuotaPools takes no disabled set and returns every
  // pool. disabledProvidersFromConfig survives in aa.ts for the AA
  // section's own config parsing — only its quota use was removed.
  test("config parsing still reuses the AA JSONC parse: comments tolerated, junk fails soft", () => {
    const cfg = `{
      // deployed config shape
      "disabled_providers": ["openrouter", "github-copilot", 42, null],
      "provider": {}
    }`;
    expect([...disabledProvidersFromConfig(cfg)].sort()).toEqual(["github-copilot", "openrouter"]);
    expect(disabledProvidersFromConfig("not json").size).toBe(0);
    expect(disabledProvidersFromConfig("{}").size).toBe(0);
  });

  test("every pool renders, in spec order — disabled doors drop nothing", () => {
    // The deployed config disables claude-code, commandcode, deepseek,
    // meta, antigravity-ls and antigravity-agy (among others). Under
    // #1114 the board lost anthropic:claude-pro, commandcode, deepseek
    // and BOTH Antigravity pools, and meta survived only through an
    // exemption hack — while the OpenCode quota read reported all of
    // them. All thirteen pools now render unconditionally; the dated
    // reads are the live read's fail-soft fallback, nothing more.
    expect(extractQuotaPools(VIEW, NOW).map((p) => p.id)).toEqual([
      "meta",
      "anthropic:claude-pro",
      "_openai.main",
      "_openai.alt",
      "opencode-go.drewrypope",
      "opencode-go.popesdrew",
      "opencode-go.developing-today-0",
      "cline-pass",
      "_commandcode",
      "_antigravity.gemini",
      "_antigravity.claude-gpt",
      "opendesign",
      "deepseek",
    ]);
  });

  test("keeper design doors never had pools and still do not", () => {
    const KEEPERS = ["cloudflare-gateway", "cloudflare-workers-ai", "code-design", "design-code", "open-design"];
    const pools = extractQuotaPools(VIEW, NOW);
    const choices = computeChoices(pools, NOW.getTime());
    const named = [...choices.hasHeadroom, ...choices.burnBeforeReset, ...choices.doNotStart];
    for (const keeper of KEEPERS) {
      expect(pools.map((p) => p.id)).not.toContain(keeper);
      expect(named).not.toContain(keeper);
    }
  });

  test("the issue's fixture disabled set (openrouter/copilot) still emits no pool for them", () => {
    // No pool ever answered to those providers; with the filter gone
    // the guarantee is structural (POOL_SPECS has no such pool), not
    // a runtime check.
    const pools = extractQuotaPools(VIEW, NOW);
    for (const p of pools) {
      expect(p.id).not.toBe("openrouter");
      expect(p.id).not.toBe("github-copilot");
    }
  });
});
