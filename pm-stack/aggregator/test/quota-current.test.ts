import { describe, expect, test } from "bun:test";
import { overlayQuotaCurrent, parseQuotaCurrent, QUOTA_CURRENT_FRESH_MS } from "../src/quota-current.ts";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaPool, QuotaView } from "../src/model.ts";

// The final visibility hop (#1151): the gatherer's quota-current.json
// overlays the dated-read quota view. Fresh reads win; error/stale
// reads surface as the pool's state; unmeasured pools are untouched.

const NOW = new Date("2026-10-27T07:00:00.000Z");
const iso = (msAgo: number) => new Date(NOW.getTime() - msAgo).toISOString();

function pool(id: string, windows: QuotaPool["windows"], state: QuotaPool["state"] = "healthy"): QuotaPool {
  const [provider, plan] = id.split(":");
  return { id, provider, plan: plan ?? id, state, windows };
}

function baseView(): QuotaView {
  return {
    pools: [
      pool("anthropic:claude-pro", [
        { name: "weekly", unit: "other", limit: 100, used: 68, percent: 68, resetsAt: null },
      ]),
      pool("openai:codex", [{ name: "7-day", unit: "other", limit: 100, used: 11, percent: 11, resetsAt: null }]),
      pool(
        "meta:high-usage",
        [{ name: "weekly", unit: "other", limit: 100, used: 100, percent: 100, resetsAt: null }],
        "walled",
      ),
      pool("deepseek:api", []),
      pool("cline-pass:cline-pass", [
        { name: "monthly", unit: "other", limit: 100, used: 69, percent: 69, resetsAt: null },
      ]),
      pool("commandcode:go", [{ name: "monthly credits", unit: "usd", limit: 10, used: 6.57, resetsAt: null }]),
    ],
    burn: [],
    choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: ["meta:high-usage"] },
  };
}

function entry(partial: Partial<QuotaCurrentEntry> & { serviceId: string }): QuotaCurrentEntry {
  return {
    providerId: partial.serviceId,
    providerName: partial.serviceId,
    state: "ok",
    measuredAt: iso(5 * 60_000),
    source: "openchamber-quota",
    windows: [],
    ...partial,
  };
}

function win(name: string, pctOfLimit: number | null, measuredAt: string) {
  return {
    name,
    value: pctOfLimit,
    pctOfLimit,
    valueLabel: null,
    windowSeconds: null,
    resetsAt: null,
    measuredAt,
    source: "openchamber-quota",
    state: "ok" as const,
  };
}

function file(providers: QuotaCurrentEntry[]): QuotaCurrentFile {
  return {
    version: 1,
    generatedAt: iso(4 * 60_000),
    source: "openchamber-quota",
    heartbeat: { tickAt: iso(4 * 60_000), ok: true, durationMs: 900, perProvider: {} },
    providers,
  };
}

describe("quota-current overlay", () => {
  test("fresh ok read replaces the pool's windows and recomputes state + choices", () => {
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        entry({
          serviceId: "meta:high-usage",
          source: "file-export/meta-laptop",
          windows: [win("5h", 1.33, iso(5 * 60_000)), win("weekly", 8.93, iso(5 * 60_000))],
        }),
      ]),
      NOW,
    );
    const meta = out.pools.find((p) => p.id === "meta:high-usage")!;
    expect(meta.windows.map((w) => [w.name, w.percent])).toEqual([
      ["5-hour", 1.33],
      ["weekly", 8.93],
    ]);
    expect(meta.state).toBe("healthy"); // was "walled" on the dated read
    expect(meta.freshness).toEqual({
      measuredAt: iso(5 * 60_000),
      source: "file-export/meta-laptop",
      state: "ok",
    });
    expect(out.choices.doNotStart).not.toContain("meta:high-usage");
    expect(out.gatherer?.providers).toHaveLength(1);
  });

  test("a fresh ok credential row wins over a newer error row for the same service", () => {
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        entry({
          serviceId: "openai:codex",
          state: "error",
          measuredAt: iso(1 * 60_000),
          error: "Sign in with ChatGPT only",
          credentialLabel: "OpenAI 2",
        }),
        entry({
          serviceId: "openai:codex",
          measuredAt: iso(3 * 60_000),
          windows: [win("weekly", 17, iso(3 * 60_000))],
          credentialLabel: "OpenAI",
        }),
      ]),
      NOW,
    );
    const codex = out.pools.find((p) => p.id === "openai:codex")!;
    expect(codex.freshness?.state).toBe("ok");
    expect(codex.windows).toHaveLength(1);
    expect(codex.windows[0].percent).toBe(17);
  });

  test("an error entry surfaces as the pool's state with its reason, windows not zeroed", () => {
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        entry({
          serviceId: "anthropic:claude-pro",
          state: "error",
          measuredAt: iso(2 * 60_000),
          error: "Rate limited. Retrying soon.",
        }),
      ]),
      NOW,
    );
    const claude = out.pools.find((p) => p.id === "anthropic:claude-pro")!;
    expect(claude.state).toBe("error");
    expect(claude.freshness?.state).toBe("error");
    expect(claude.freshness?.error).toBe("Rate limited. Retrying soon.");
    // The dated-read windows stay visible under the error badge — not
    // replaced by invented zeros.
    expect(claude.windows[0].percent).toBe(68);
    expect(out.choices.doNotStart).toContain("meta:high-usage"); // choices untouched: no fresh windows anywhere
  });

  test("an ok entry past the freshness bound surfaces as stale, windows untouched", () => {
    const staleAt = iso(QUOTA_CURRENT_FRESH_MS + 5 * 60_000);
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        entry({ serviceId: "cline-pass:cline-pass", measuredAt: staleAt, windows: [win("monthly", 71, staleAt)] }),
      ]),
      NOW,
    );
    const cline = out.pools.find((p) => p.id === "cline-pass:cline-pass")!;
    expect(cline.state).toBe("stale");
    expect(cline.freshness).toEqual({ measuredAt: staleAt, source: "openchamber-quota", state: "stale" });
    expect(cline.windows[0].percent).toBe(69); // dated read, not the aged gatherer value
  });

  test("a balance-only ok entry stamps freshness without inventing windows", () => {
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        entry({
          serviceId: "deepseek:api",
          windows: [{ ...win("credits_balance", null, iso(5 * 60_000)), valueLabel: "¥44.29" }],
        }),
      ]),
      NOW,
    );
    const ds = out.pools.find((p) => p.id === "deepseek:api")!;
    expect(ds.freshness?.state).toBe("ok");
    expect(ds.windows).toEqual([]);
    expect(ds.state).toBe("healthy");
  });

  test("pools with no gatherer entry are untouched and carry no freshness", () => {
    const out = overlayQuotaCurrent(baseView(), file([entry({ serviceId: "meta:high-usage", windows: [] })]), NOW);
    const cc = out.pools.find((p) => p.id === "commandcode:go")!;
    expect(cc.freshness).toBeUndefined();
    expect(cc.windows[0].used).toBe(6.57);
    expect(cc.state).toBe("healthy");
  });

  test("the gatherer summary lists every provider, including pool-less ones", () => {
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        entry({ serviceId: "opencode-go", providerId: "opencode-go", windows: [win("monthly", 62.5, iso(60_000))] }),
        entry({ serviceId: "anthropic:claude-pro", state: "error", error: "boom" }),
      ]),
      NOW,
    );
    expect(out.gatherer?.tickAt).toBe(iso(4 * 60_000));
    const byService = new Map(out.gatherer!.providers.map((p) => [p.serviceId, p.state]));
    expect(byService.get("opencode-go")).toBe("ok");
    expect(byService.get("anthropic:claude-pro")).toBe("error");
  });

  test("the input view is not mutated", () => {
    const view = baseView();
    overlayQuotaCurrent(
      view,
      file([entry({ serviceId: "meta:high-usage", windows: [win("weekly", 9, iso(60_000))] })]),
      NOW,
    );
    expect(view.pools.find((p) => p.id === "meta:high-usage")!.windows[0].percent).toBe(100);
    expect(view.gatherer).toBeUndefined();
  });
});

describe("parseQuotaCurrent", () => {
  test("accepts a well-formed document", () => {
    const doc = JSON.parse(JSON.stringify(file([entry({ serviceId: "meta:high-usage" })])));
    expect(parseQuotaCurrent(doc)?.providers).toHaveLength(1);
  });

  test("rejects garbage, wrong versions, and shapeless providers", () => {
    expect(parseQuotaCurrent(null)).toBeNull();
    expect(parseQuotaCurrent("nope")).toBeNull();
    expect(parseQuotaCurrent({ version: 2, generatedAt: "x", providers: [] })).toBeNull();
    expect(parseQuotaCurrent({ version: 1, generatedAt: "x", providers: [{ providerId: "meta" }] })).toBeNull();
    expect(parseQuotaCurrent({ version: 1, generatedAt: "x", providers: "no" })).toBeNull();
  });
});
