import { describe, expect, test } from "bun:test";
import {
  overlayQuotaCurrent,
  overlayTursoUsage,
  parseQuotaCurrent,
  QUOTA_CURRENT_FRESH_MS,
  QUOTA_CURRENT_FUTURE_SKEW_MS,
} from "../src/quota-current.ts";
import type { QuotaCurrentEntry, QuotaCurrentFile, QuotaPool, QuotaView, SaasService } from "../src/model.ts";

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

  test("an ok entry dated past the future-skew tolerance is stale, windows untouched (#1541)", () => {
    for (const futureMs of [60 * 60_000, 30 * 24 * 60 * 60_000]) {
      const futureAt = iso(-futureMs);
      const out = overlayQuotaCurrent(
        baseView(),
        file([
          entry({ serviceId: "cline-pass:cline-pass", measuredAt: futureAt, windows: [win("monthly", 71, futureAt)] }),
        ]),
        NOW,
      );
      const cline = out.pools.find((p) => p.id === "cline-pass:cline-pass")!;
      expect(cline.state).toBe("stale");
      expect(cline.freshness).toEqual({ measuredAt: futureAt, source: "openchamber-quota", state: "stale" });
      expect(cline.windows[0].percent).toBe(69); // dated read, not the future-dated gatherer value
      expect(out.gatherer?.providers.find((p) => p.serviceId === "cline-pass:cline-pass")?.state).toBe("stale");
    }
  });

  test("an ok entry dated within the future-skew tolerance stays fresh (#1541)", () => {
    const at = iso(-(QUOTA_CURRENT_FUTURE_SKEW_MS - 30_000));
    const out = overlayQuotaCurrent(
      baseView(),
      file([entry({ serviceId: "cline-pass:cline-pass", measuredAt: at, windows: [win("monthly", 71, at)] })]),
      NOW,
    );
    const cline = out.pools.find((p) => p.id === "cline-pass:cline-pass")!;
    expect(cline.freshness?.state).toBe("ok");
    expect(cline.windows[0].percent).toBe(71);
  });

  test("a future-dated entry cannot outrank a genuinely fresh entry for the same service (#1541)", () => {
    const futureAt = iso(-60 * 60_000);
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        entry({ serviceId: "cline-pass:cline-pass", measuredAt: futureAt, windows: [win("monthly", 99, futureAt)] }),
        entry({
          serviceId: "cline-pass:cline-pass",
          measuredAt: iso(5 * 60_000),
          windows: [win("monthly", 71, iso(5 * 60_000))],
        }),
      ]),
      NOW,
    );
    const cline = out.pools.find((p) => p.id === "cline-pass:cline-pass")!;
    expect(cline.freshness?.state).toBe("ok");
    expect(cline.freshness?.measuredAt).toBe(iso(5 * 60_000));
    expect(cline.windows[0].percent).toBe(71); // the genuinely fresh read, not the future-dated 99
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

describe("overlayTursoUsage (#1049)", () => {
  const tursoSvc = (): SaasService => ({
    id: "turso",
    name: "Turso",
    group: "database",
    plan: "Starter Free $0",
    status: "active",
    metrics: [{ id: "databases", label: "Databases", used: 1, limit: 100, unit: "count", percent: 1, highlight: true }],
    paygEstimate: null,
    measuredAt: "2026-10-20T00:00:00.000Z",
    sourceClass: "manual",
    sourceNote: "quota-view.md Part (b)",
    nextDate: "2026-10-31 cycle end",
    cadenceDays: 7,
    conflicts: [],
    reads: [],
  });
  const tursoWin = (name: string, value: number, pct: number, measuredAt: string) => ({
    name,
    value,
    pctOfLimit: pct,
    valueLabel: null,
    windowSeconds: 2_678_400,
    resetsAt: "2026-11-01T04:00:00.000Z",
    measuredAt,
    source: "turso-api",
    state: "ok" as const,
  });
  test("a fresh ok entry replaces the card metrics, stamp, and cycle end", () => {
    const at = iso(5 * 60_000);
    const out = overlayTursoUsage(
      [tursoSvc()],
      file([
        entry({
          serviceId: "turso",
          providerId: "turso",
          source: "turso-api",
          measuredAt: at,
          windows: [tursoWin("databases", 3, 3, at), tursoWin("storage", 1.053323, 21.066, at)],
        }),
      ]),
      NOW,
    );
    const svc = out[0];
    expect(svc.measuredAt).toBe(at);
    expect(svc.sourceClass).toBe("api");
    expect(svc.nextDate).toBe("cycle ends 2026-11-01");
    expect(svc.metrics.map((m) => [m.id, m.used, m.unit, m.percent])).toEqual([
      ["databases", 3, "count", 3],
      ["storage", 1.053323, "gb", 21.066],
    ]);
  });
  test("an error entry keeps the dated card and adds a dated conflict flag", () => {
    const at = iso(5 * 60_000);
    const out = overlayTursoUsage(
      [tursoSvc()],
      file([entry({ serviceId: "turso", providerId: "turso", source: "turso-api", state: "error", measuredAt: at, error: "turso /usage HTTP 401" })]),
      NOW,
    );
    expect(out[0].measuredAt).toBe("2026-10-20T00:00:00.000Z");
    expect(out[0].metrics[0].used).toBe(1);
    expect(out[0].conflicts).toHaveLength(1);
    expect(out[0].conflicts[0]).toContain("HTTP 401");
  });
  test("no turso entry or no turso card passes through unchanged", () => {
    const services = [tursoSvc()];
    expect(overlayTursoUsage(services, file([]), NOW)).toBe(services);
    const other: SaasService = { ...tursoSvc(), id: "supabase", name: "Supabase" };
    const arr = [other];
    const at = iso(5 * 60_000);
    expect(
      overlayTursoUsage(arr, file([entry({ serviceId: "turso", providerId: "turso", source: "turso-api", measuredAt: at, windows: [tursoWin("databases", 3, 3, at)] })]), NOW),
    ).toBe(arr);
  });
});

describe("per-credential quota rows (#1113)", () => {
  const twoLogins = (aPct: number, bPct: number, bMeasured = iso(2 * 60_000)) =>
    file([
      entry({
        serviceId: "openai:codex",
        providerId: "openai-main",
        credentialLabel: "OpenAI",
        windows: [win("weekly", aPct, iso(5 * 60_000))],
      }),
      entry({
        serviceId: "openai:codex",
        providerId: "openai-second",
        credentialLabel: "OpenAI 2",
        measuredAt: bMeasured,
        windows: [win("weekly", bPct, bMeasured)],
      }),
    ]);

  test("two logins with genuinely differing quotas split into rows; headline stays the newest read", () => {
    const out = overlayQuotaCurrent(baseView(), twoLogins(30, 80), NOW);
    const pool = out.pools.find((p) => p.id === "openai:codex")!;
    expect(pool.windows.map((w) => w.percent)).toEqual([80]);
    expect(pool.credentials).toHaveLength(2);
    const byLabel = new Map(pool.credentials!.map((c) => [c.label, c]));
    expect(byLabel.get("OpenAI")!.windows.map((w) => w.percent)).toEqual([30]);
    expect(byLabel.get("OpenAI 2")!.windows.map((w) => w.percent)).toEqual([80]);
    expect(byLabel.get("OpenAI 2")!.freshness.state).toBe("ok");
  });

  test("two logins sharing one pool read do not split", () => {
    const out = overlayQuotaCurrent(baseView(), twoLogins(42, 42), NOW);
    const pool = out.pools.find((p) => p.id === "openai:codex")!;
    expect(pool.credentials).toBeUndefined();
  });

  test("a credential whose newest read is an error surfaces as an error row beside the ok one", () => {
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        entry({
          serviceId: "openai:codex",
          providerId: "openai-main",
          credentialLabel: "OpenAI",
          windows: [win("weekly", 30, iso(5 * 60_000))],
        }),
        entry({
          serviceId: "openai:codex",
          providerId: "openai-second",
          credentialLabel: "OpenAI 2",
          state: "error",
          measuredAt: iso(1 * 60_000),
          error: "Sign in with ChatGPT only",
        }),
      ]),
      NOW,
    );
    const pool = out.pools.find((p) => p.id === "openai:codex")!;
    expect(pool.windows.map((w) => w.percent)).toEqual([30]); // headline from the ok login
    const err = pool.credentials!.find((c) => c.label === "OpenAI 2")!;
    expect(err.freshness.state).toBe("error");
    expect(err.freshness.error).toContain("Sign in with ChatGPT");
    expect(err.windows).toEqual([]);
  });

  test("a single credential never produces rows", () => {
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        entry({
          serviceId: "openai:codex",
          providerId: "openai-main",
          credentialLabel: "OpenAI",
          windows: [win("weekly", 30, iso(5 * 60_000))],
        }),
      ]),
      NOW,
    );
    expect(out.pools.find((p) => p.id === "openai:codex")!.credentials).toBeUndefined();
  });

  test("pool card renders one line per credential row", async () => {
    const { renderQuota } = await import("../src/render.ts");
    const out = overlayQuotaCurrent(baseView(), twoLogins(30, 80), NOW);
    const prov = {
      board: "", roadmap: "", items: "", comments: "", quota: "", usage: "",
      attention: "", orchestration: "", dns: "", registry: "", aa: "",
    };
    const html = renderQuota(out, prov);
    expect(html).toContain("credential OpenAI:");
    expect(html).toContain("credential OpenAI 2:");
    expect(html).toContain("weekly 80%");
  });
});

describe("per-credential rows: Codex findings on PR #2978", () => {
  test("distinct credentialIds with identical labels still split when quotas differ", () => {
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        entry({
          serviceId: "openai:codex",
          providerId: "openai-main",
          credentialId: "cred-a",
          credentialLabel: "OpenAI",
          windows: [win("weekly", 30, iso(5 * 60_000))],
        }),
        entry({
          serviceId: "openai:codex",
          providerId: "openai-main",
          credentialId: "cred-b",
          credentialLabel: "OpenAI",
          measuredAt: iso(2 * 60_000),
          windows: [win("weekly", 80, iso(2 * 60_000))],
        }),
      ]),
      NOW,
    );
    const pool = out.pools.find((p) => p.id === "openai:codex")!;
    expect(pool.credentials).toHaveLength(2);
    expect(pool.credentials!.map((c) => c.credentialId).sort()).toEqual(["cred-a", "cred-b"]);
    expect(pool.credentials!.map((c) => c.windows[0].percent).sort((a, b) => a - b)).toEqual([30, 80]);
  });

  test("reset drift under the tolerance does not split a shared read", () => {
    const mk = (cred: string, label: string, resetsAt: string, measuredAt: string) =>
      entry({
        serviceId: "openai:codex",
        providerId: "openai-main",
        credentialId: cred,
        credentialLabel: label,
        measuredAt,
        windows: [{ ...win("weekly", 42, measuredAt), resetsAt }],
      });
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        mk("cred-a", "OpenAI", "2026-10-27T12:00:00.000Z", iso(5 * 60_000)),
        mk("cred-b", "OpenAI 2", "2026-10-27T12:00:00.700Z", iso(2 * 60_000)),
      ]),
      NOW,
    );
    expect(out.pools.find((p) => p.id === "openai:codex")!.credentials).toBeUndefined();
  });

  test("resets genuinely apart still split even at equal percent", () => {
    const mk = (cred: string, label: string, resetsAt: string, measuredAt: string) =>
      entry({
        serviceId: "openai:codex",
        providerId: "openai-main",
        credentialId: cred,
        credentialLabel: label,
        measuredAt,
        windows: [{ ...win("weekly", 42, measuredAt), resetsAt }],
      });
    const out = overlayQuotaCurrent(
      baseView(),
      file([
        mk("cred-a", "OpenAI", "2026-10-27T12:00:00.000Z", iso(5 * 60_000)),
        mk("cred-b", "OpenAI 2", "2026-10-27T18:00:00.000Z", iso(2 * 60_000)),
      ]),
      NOW,
    );
    expect(out.pools.find((p) => p.id === "openai:codex")!.credentials).toHaveLength(2);
  });
});

describe("stale-path newest pick is finite-gated (#2253)", () => {
  const staleRead = () =>
    entry({ serviceId: "meta:high-usage", measuredAt: iso(2 * 3_600_000), source: "real-src", windows: [] });
  const garbageRead = () =>
    entry({ serviceId: "meta:high-usage", measuredAt: "not-a-date", source: "garbage-src", windows: [] });

  test("a garbage measuredAt listed first does not shadow the real stale read", () => {
    const out = overlayQuotaCurrent(baseView(), file([garbageRead(), staleRead()]), NOW);
    const p = out.pools.find((x) => x.id === "meta:high-usage")!;
    expect(p.freshness).toEqual({ measuredAt: iso(2 * 3_600_000), source: "real-src", state: "stale" });
  });

  test("a garbage measuredAt listed last gives the same winner (order independence)", () => {
    const out = overlayQuotaCurrent(baseView(), file([staleRead(), garbageRead()]), NOW);
    const p = out.pools.find((x) => x.id === "meta:high-usage")!;
    expect(p.freshness).toEqual({ measuredAt: iso(2 * 3_600_000), source: "real-src", state: "stale" });
  });

  test("a +30d future entry does not outrank a real 1h-old stale read", () => {
    const future = entry({
      serviceId: "meta:high-usage",
      measuredAt: new Date(NOW.getTime() + 30 * 86_400_000).toISOString(),
      source: "future-src",
      windows: [],
    });
    const real = entry({ serviceId: "meta:high-usage", measuredAt: iso(3_600_000), source: "real-src", windows: [] });
    const out = overlayQuotaCurrent(baseView(), file([future, real]), NOW);
    const p = out.pools.find((x) => x.id === "meta:high-usage")!;
    expect(p.freshness).toEqual({ measuredAt: iso(3_600_000), source: "real-src", state: "stale" });
  });

  test("parseQuotaCurrent still accepts a document carrying a garbage entry (lenient parse)", () => {
    const doc = file([garbageRead(), staleRead()]);
    expect(parseQuotaCurrent(JSON.parse(JSON.stringify(doc)))).not.toBeNull();
  });
});
