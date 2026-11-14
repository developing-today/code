import { describe, expect, test } from "bun:test";
import {
  ANTIGRAVITY_CLAUDE_GPT_SERVICE_ID,
  ANTIGRAVITY_GEMINI_SERVICE_ID,
  GO_POPESDREW_SERVICE_ID,
  GO_DREWRYPOPE_SERVICE_ID,
  OPENAI_ALTERNATE_SERVICE_ID,
  OPENAI_MAIN_SERVICE_ID,
  OPENAI_SELF_SERVICE_ID,
  canonicalProviderId,
  entriesWithPoolSplits,
  entryFromFileExport,
  entryFromTurso,
  entryFromResult,
  openaiPoolServiceId,
  goPoolServiceId,
  lastKnownEntries,
  mapWithConcurrency,
  markRetained,
  normalizeResetAt,
  recordsFromEntries,
  serviceIdFor,
  type QuotaProviderResult,
} from "../src/quota-gather-map.ts";

// A cline-pass-shaped result, as the OpenChamber module answers it
// (resetAt in epoch milliseconds, fetchedAt likewise).
const OK_RESULT: QuotaProviderResult = {
  providerId: "cline-pass",
  providerName: "ClinePass",
  ok: true,
  configured: true,
  fetchedAt: 1793075143879,
  credentialId: "cred_1",
  credentialLabel: "ClinePass",
  usage: {
    windows: {
      weekly: {
        usedPercent: 29,
        remainingPercent: 71,
        windowSeconds: 604800,
        resetAt: 1793438427299,
        resetAfterSeconds: 360761,
      },
      credits_balance: {
        usedPercent: null,
        remainingPercent: null,
        windowSeconds: null,
        resetAt: null,
        valueLabel: "12.50",
      },
    },
  },
};

describe("normalizeResetAt", () => {
  test("epoch ms and s both become ISO", () => {
    expect(normalizeResetAt(1793438427299)).toBe("2026-10-31T09:20:27.299Z");
    expect(normalizeResetAt(1793438427)).toBe("2026-10-31T09:20:27.000Z");
  });
  test("ISO strings pass through normalized; junk is null", () => {
    expect(normalizeResetAt("2026-10-31T14:20:27Z")).toBe("2026-10-31T14:20:27.000Z");
    expect(normalizeResetAt("not a date")).toBeNull();
    expect(normalizeResetAt(null)).toBeNull();
    expect(normalizeResetAt(0)).toBeNull();
  });
  test("impossible and non-ISO date strings are null (#4968)", () => {
    expect(normalizeResetAt("2026-02-30")).toBeNull();
    expect(normalizeResetAt("2026-04-31")).toBeNull();
    expect(normalizeResetAt("2026-02-29")).toBeNull(); // 2026 is not a leap year
    expect(normalizeResetAt("March 2, 2026")).toBeNull();
  });
  test("valid date-only and leap-day strings pass (#4968)", () => {
    expect(normalizeResetAt("2026-03-02")).toBe("2026-03-02T00:00:00.000Z");
    expect(normalizeResetAt("2024-02-29")).toBe("2024-02-29T00:00:00.000Z");
  });
});

describe("entryFromResult", () => {
  test("maps windows onto the quota-current shape", () => {
    const entry = entryFromResult(OK_RESULT, "2026-10-27T05:00:00.000Z");
    expect(entry.serviceId).toBe("cline-pass");
    expect(entry.state).toBe("ok");
    expect(entry.measuredAt).toBe(new Date(1793075143879).toISOString());
    const weekly = entry.windows.find((w) => w.name === "weekly");
    expect(weekly?.pctOfLimit).toBe(29);
    expect(weekly?.value).toBe(29);
    expect(weekly?.resetsAt).toBe("2026-10-31T09:20:27.299Z");
    const balance = entry.windows.find((w) => w.name === "credits_balance");
    expect(balance?.pctOfLimit).toBeNull();
    expect(balance?.valueLabel).toBe("12.50");
    expect(entry.credentialWatch).toBe(true);
  });
  test("maps only authoritative expiry metadata and separates unconfigured credentials", () => {
    const entry = entryFromResult(
      { ...OK_RESULT, providerId: "claude", credentialExpiresAt: 1793438427299, credentialSource: "claude-code" },
      "2026-10-27T05:00:00.000Z",
    );
    expect(entry.credentialExpiresAt).toBe("2026-10-31T09:20:27.299Z");
    expect(entry.credentialSource).toBe("claude-code");
    const absent = entryFromResult({ ...OK_RESULT, configured: false, ok: false }, "2026-10-27T05:00:00.000Z");
    expect(absent.state).toBe("unconfigured");
    expect(absent.credentialWatch).toBe(true);
  });
  test.each([
    ["impossible timestamp", "2026-02-30T00:00:00.000Z"],
    ["impossible date-only value", "2026-04-31"],
    ["non-ISO string", "March 2, 2026"],
  ])("rejects %s credential expiry strings", (_kind, credentialExpiresAt) => {
    const tick = "2026-10-27T05:00:00.000Z";
    const entry = entryFromResult({ ...OK_RESULT, credentialExpiresAt }, tick);
    expect(entry.credentialExpiresAt).toBeUndefined();
  });

  test("maps a valid ISO credential expiry string", () => {
    const tick = "2026-10-27T05:00:00.000Z";
    const validIso = entryFromResult({ ...OK_RESULT, credentialExpiresAt: "2027-01-15T00:00:00.000Z" }, tick);
    expect(validIso.credentialExpiresAt).toBe("2027-01-15T00:00:00.000Z");
  });

  test("maps numeric epoch-ms credential expiry", () => {
    const tick = "2026-10-27T05:00:00.000Z";
    const validEpoch = entryFromResult({ ...OK_RESULT, credentialExpiresAt: 1793438427299 }, tick);
    expect(validEpoch.credentialExpiresAt).toBe("2026-10-31T09:20:27.299Z");
  });
  test("falls back for invalid fetchedAt values without dropping a mixed batch", () => {
    const tickIso = "2026-11-02T03:00:00.000Z";
    const fetchedAts = [1e16, -1e16, 0, -1, NaN, Infinity, -Infinity, undefined, 1793075143879];
    const results = fetchedAts.map((fetchedAt) => ({ ...OK_RESULT, fetchedAt }));
    const entries = results.map((result) => entryFromResult(result, tickIso));

    expect(entries).toHaveLength(results.length);
    expect(entries.map((entry) => entry.measuredAt)).toEqual([
      tickIso,
      tickIso,
      tickIso,
      tickIso,
      tickIso,
      tickIso,
      tickIso,
      tickIso,
      new Date(1793075143879).toISOString(),
    ]);
    for (const entry of entries) {
      expect(entry.windows).toHaveLength(2);
      expect(entry.windows.every((window) => window.measuredAt === entry.measuredAt)).toBe(true);
    }
  });
  test("per-model usage flattens to <model>:<window>", () => {
    const entry = entryFromResult(
      {
        providerId: "antigravity",
        providerName: "Antigravity",
        ok: true,
        configured: true,
        fetchedAt: 1793075143879,
        usage: {
          models: {
            "gemini-3.5-flash-lite": {
              windows: { weekly: { usedPercent: 2, windowSeconds: 604800, resetAt: 1793452260000 } },
            },
          },
        },
      },
      "2026-10-27T05:00:00.000Z",
    );
    expect(entry.windows).toHaveLength(1);
    expect(entry.windows[0].name).toBe("gemini-3.5-flash-lite:weekly");
    expect(entry.windows[0].pctOfLimit).toBe(2);
    expect(entry.windows[0].resetsAt).toBe("2026-10-31T13:11:00.000Z");
  });
  test("out-of-range percentages are omitted while null and valid sibling windows survive", () => {
    const entry = entryFromResult(
      {
        providerId: "claude",
        ok: true,
        configured: true,
        usage: {
          windows: {
            below: { usedPercent: -20 },
            above: { usedPercent: 150 },
            huge: { usedPercent: 1e9 },
            zero: { usedPercent: 0 },
            full: { usedPercent: 100 },
            valid: { usedPercent: 42 },
            balance: { usedPercent: null, valueLabel: "$12.50" },
          },
        },
      },
      "2026-10-27T05:00:00.000Z",
    );

    expect(entry.windows.map((w) => [w.name, w.pctOfLimit])).toEqual([
      ["zero", 0],
      ["full", 100],
      ["valid", 42],
      ["balance", null],
    ]);
    const { records } = recordsFromEntries([entry], new Map(), "quota-gatherer");
    expect(records.map((r) => [r.metricId, r.value])).toEqual([
      ["full", 100],
      ["valid", 42],
      ["zero", 0],
    ]);
  });
  test("turso windows keep over-limit percentages; other providers keep the [0,100] gate (#6227)", () => {
    // The live path: the opencode quota subcommand's turso result
    // reaches the gatherer through entryFromResult. Its storage and
    // rows-read windows are usage-vs-limit ratios that legitimately
    // exceed 100 over the Starter limits (see the entryFromTurso
    // fixture above); negative and non-finite values still drop.
    const entry = entryFromResult(
      {
        providerId: "turso",
        providerName: "Turso",
        ok: true,
        configured: true,
        usage: {
          windows: {
            databases: { usedPercent: 44 },
            storage: { usedPercent: 147.16 },
            "rows-read": { usedPercent: 164.287 },
            "rows-written": { usedPercent: 16.721 },
            sync: { usedPercent: 0 },
            negative: { usedPercent: -1 },
          },
        },
      },
      "2026-11-05T13:00:00.000Z",
    );
    expect(entry.serviceId).toBe("turso");
    expect(entry.windows.map((w) => [w.name, w.pctOfLimit])).toEqual([
      ["databases", 44],
      ["storage", 147.16],
      ["rows-read", 164.287],
      ["rows-written", 16.721],
      ["sync", 0],
    ]);
  });
  test("error and unconfigured states stay distinct", () => {
    const err = entryFromResult(
      { providerId: "claude", ok: false, configured: true, error: "session expired", usage: null },
      "2026-10-27T05:00:00.000Z",
    );
    expect(err.state).toBe("error");
    expect(err.serviceId).toBe("anthropic:claude-pro");
    expect(err.windows).toEqual([]);
    const un = entryFromResult(
      { providerId: "cursor", ok: false, configured: false, error: "Not configured" },
      "2026-10-27T05:00:00.000Z",
    );
    expect(un.state).toBe("unconfigured");
  });
});

describe("mapWithConcurrency (#1268)", () => {
  test("preserves order and never exceeds the in-flight bound", async () => {
    let inFlight = 0;
    let maxInFlight = 0;
    const out = await mapWithConcurrency([1, 2, 3, 4, 5, 6, 7], 3, async (n) => {
      inFlight++;
      maxInFlight = Math.max(maxInFlight, inFlight);
      await Bun.sleep(n % 3);
      inFlight--;
      return n * 10;
    });
    expect(out).toEqual([10, 20, 30, 40, 50, 60, 70]);
    expect(maxInFlight).toBe(3);
  });
  test("a limit above the item count still completes; empty input maps to empty", async () => {
    expect(await mapWithConcurrency([1, 2], 6, async (n) => n + 1)).toEqual([2, 3]);
    expect(await mapWithConcurrency([], 6, async (n: number) => n)).toEqual([]);
  });
  test("a non-positive limit degrades to sequential, never unbounded", async () => {
    let inFlight = 0;
    let maxInFlight = 0;
    const out = await mapWithConcurrency([1, 2, 3], 0, async (n) => {
      inFlight++;
      maxInFlight = Math.max(maxInFlight, inFlight);
      await Bun.sleep(1);
      inFlight--;
      return n;
    });
    expect(out).toEqual([1, 2, 3]);
    expect(maxInFlight).toBe(1);
  });
  test("a worker rejection rejects the map", async () => {
    await expect(
      mapWithConcurrency([1, 2], 2, async (n) => {
        if (n === 2) throw new Error("boom");
        return n;
      }),
    ).rejects.toThrow("boom");
  });
});

describe("lastKnownEntries (#1268)", () => {
  const tick = "2026-10-27T05:00:00.000Z";
  const okEntry = entryFromResult(OK_RESULT, tick);
  const errEntry = entryFromResult(
    { providerId: "cline-pass", ok: false, configured: true, error: "timed out", usage: null },
    tick,
  );
  const otherOk = entryFromResult({ ...OK_RESULT, providerId: "claude" }, tick);
  test("returns the provider's ok entries verbatim, original measuredAt kept", () => {
    const kept = lastKnownEntries([okEntry, errEntry, otherOk], "cline-pass");
    expect(kept).toEqual([okEntry]);
    expect(kept[0].measuredAt).toBe(okEntry.measuredAt);
  });
  test("error-only history retains nothing; unknown providers retain nothing", () => {
    expect(lastKnownEntries([errEntry], "cline-pass")).toEqual([]);
    expect(lastKnownEntries([okEntry], "nope")).toEqual([]);
    expect(lastKnownEntries([], "cline-pass")).toEqual([]);
  });
  test("markRetained stamps copies with the failed tick; originals and measuredAt untouched", () => {
    const stamped = markRetained([okEntry], "2026-10-27T05:15:00.000Z");
    expect(stamped).toHaveLength(1);
    expect(stamped[0].retainedFromFailedTick).toBe("2026-10-27T05:15:00.000Z");
    expect(stamped[0].measuredAt).toBe(okEntry.measuredAt);
    expect(stamped[0]).not.toBe(okEntry);
    expect(okEntry.retainedFromFailedTick).toBeUndefined();
  });
});

describe("recordsFromEntries", () => {
  test("a pct record per changed window; label-only windows yield none", () => {
    const entry = entryFromResult(OK_RESULT, "2026-10-27T05:00:00.000Z");
    const first = recordsFromEntries([entry], new Map(), "quota-gatherer");
    expect(first.records).toHaveLength(1);
    expect(first.records[0]).toMatchObject({
      serviceId: "cline-pass",
      metricId: "weekly",
      value: 29,
      unit: "pct",
      limit: 100,
      source: "api",
      observedBy: "quota-gatherer",
    });
    // Same values again -> nothing new appended.
    const state = new Map(first.appended);
    const second = recordsFromEntries([entry], state, "quota-gatherer");
    expect(second.records).toHaveLength(0);
    // A moved value appends again.
    const moved = entryFromResult(
      {
        ...OK_RESULT,
        usage: { windows: { weekly: { usedPercent: 31, windowSeconds: 604800, resetAt: 1793438427299 } } },
      },
      "2026-10-27T05:15:00.000Z",
    );
    const third = recordsFromEntries([moved], state, "quota-gatherer");
    expect(third.records).toHaveLength(1);
    expect(third.records[0].value).toBe(31);
    // A recomputed reset instant with the SAME value appends nothing
    // (providers derive resetsAt from durations; it drifts per fetch).
    const drifted = entryFromResult(
      {
        ...OK_RESULT,
        usage: { windows: { weekly: { usedPercent: 31, windowSeconds: 604800, resetAt: 1793438427999 } } },
      },
      "2026-10-27T05:30:00.000Z",
    );
    const stateAfterMove = new Map([...state, ...third.appended]);
    expect(recordsFromEntries([drifted], stateAfterMove, "quota-gatherer").records).toHaveLength(0);
  });
  test("error entries produce no records", () => {
    const err = entryFromResult(
      { providerId: "claude", ok: false, configured: true, error: "x", usage: null },
      "2026-10-27T05:00:00.000Z",
    );
    expect(recordsFromEntries([err], new Map(), "quota-gatherer").records).toEqual([]);
  });
});

describe("recordsFromEntries — CNY balances (#1052)", () => {
  // The DeepSeek provider answers { credits_balance: { usedPercent: null,
  // valueLabel: "¥44.29" } } — a yuan figure with no percent.
  const deepseek = (label: string): QuotaProviderResult => ({
    providerId: "deepseek",
    providerName: "DeepSeek",
    ok: true,
    configured: true,
    fetchedAt: Date.parse("2026-10-25T09:00:00Z"),
    usage: { windows: { credits_balance: { usedPercent: null, valueLabel: label } } },
  });
  const at = "2026-10-25T09:00:00.000Z";

  test("a ¥ label becomes one cny record, stored in yuan", () => {
    const entry = entryFromResult(deepseek("¥44.63"), at);
    const first = recordsFromEntries([entry], new Map(), "quota-gatherer");
    expect(first.records).toHaveLength(1);
    expect(first.records[0]).toMatchObject({
      serviceId: "deepseek",
      metricId: "credits_balance",
      value: 44.63,
      unit: "cny",
      source: "api",
      observedBy: "quota-gatherer",
    });
  });

  test("value-moved dedupe: unchanged appends nothing, a moved balance appends", () => {
    const state = new Map(recordsFromEntries([entryFromResult(deepseek("¥44.63"), at)], new Map(), "q").appended);
    expect(recordsFromEntries([entryFromResult(deepseek("¥44.63"), at)], state, "q").records).toHaveLength(0);
    const moved = recordsFromEntries([entryFromResult(deepseek("¥44.29"), at)], state, "q");
    expect(moved.records).toHaveLength(1);
    expect(moved.records[0]).toMatchObject({ value: 44.29, unit: "cny" });
  });

  test("a USD-labelled balance still yields no record (no USD balance series)", () => {
    expect(recordsFromEntries([entryFromResult(deepseek("$12.50"), at)], new Map(), "q").records).toEqual([]);
  });
});

describe("recordsFromEntries multi-credential pool pinning (#1624)", () => {
  const T1 = "2026-10-28T05:00:00.000Z";
  const T2 = "2026-10-28T05:15:00.000Z";
  const T3 = "2026-10-28T05:30:00.000Z";
  const T4 = "2026-10-28T05:45:00.000Z";
  const cred = (credentialId: string, pct: number, tickIso: string) =>
    entryFromResult(
      {
        providerId: "claude",
        providerName: "Claude",
        ok: true,
        configured: true,
        fetchedAt: Date.parse(tickIso),
        credentialId,
        usage: { windows: { "5h": { usedPercent: pct, windowSeconds: 18_000, resetAt: 1793452260000 } } },
      },
      tickIso,
    );

  test("two ok credentials, constant 10/20: tick 1 appends one max record, ticks 2+ append none", () => {
    let state = new Map<string, string>();
    const ticks = [T1, T2, T3, T4];
    const counts: number[] = [];
    for (const tick of ticks) {
      const { records, appended } = recordsFromEntries(
        [cred("cred_a", 10, tick), cred("cred_b", 20, tick)],
        state,
        "quota-gatherer",
      );
      counts.push(records.length);
      if (tick === T1) {
        expect(records).toHaveLength(1);
        expect(records[0]).toMatchObject({ serviceId: "anthropic:claude-pro", metricId: "5h", value: 20 });
      }
      // Replicate the caller merge (gather-quota.ts): stored stamp
      // updates only when the tick appended records.
      if (records.length) for (const [k, v] of appended) state.set(k, v);
    }
    expect(counts).toEqual([1, 0, 0, 0]);
  });

  test("entry order reversal produces the identical tick-1 record", () => {
    const fwd = recordsFromEntries([cred("cred_a", 10, T1), cred("cred_b", 20, T1)], new Map(), "quota-gatherer");
    const rev = recordsFromEntries([cred("cred_b", 20, T1), cred("cred_a", 10, T1)], new Map(), "quota-gatherer");
    expect(fwd.records).toHaveLength(1);
    expect(rev.records).toEqual(fwd.records);
  });

  test("equal values tie-break to the lowest credentialId", () => {
    const fwd = recordsFromEntries([cred("cred_a", 20, T1), cred("cred_b", 20, T1)], new Map(), "quota-gatherer");
    const rev = recordsFromEntries([cred("cred_b", 20, T1), cred("cred_a", 20, T1)], new Map(), "quota-gatherer");
    expect(fwd.records).toHaveLength(1);
    expect(fwd.records[0].value).toBe(20);
    // The max is the same either way; the tie-break only decides
    // which credential's row backs the record, deterministically.
    expect(rev.records).toEqual(fwd.records);
  });

  test("single-credential constant case appends once then zero", () => {
    const first = recordsFromEntries([cred("cred_a", 10, T1)], new Map(), "quota-gatherer");
    expect(first.records).toHaveLength(1);
    const state = new Map(first.appended);
    for (const tick of [T2, T3]) {
      expect(recordsFromEntries([cred("cred_a", 10, tick)], state, "quota-gatherer").records).toHaveLength(0);
    }
  });

  test("max credential moving 20->25 appends exactly once", () => {
    const first = recordsFromEntries([cred("cred_a", 10, T1), cred("cred_b", 20, T1)], new Map(), "quota-gatherer");
    expect(first.records).toHaveLength(1);
    const state = new Map(first.appended);
    const moved = recordsFromEntries([cred("cred_a", 10, T2), cred("cred_b", 25, T2)], state, "quota-gatherer");
    expect(moved.records).toHaveLength(1);
    expect(moved.records[0].value).toBe(25);
    const settled = new Map([...state, ...moved.appended]);
    expect(
      recordsFromEntries([cred("cred_a", 10, T3), cred("cred_b", 25, T3)], settled, "quota-gatherer").records,
    ).toHaveLength(0);
  });

  test("non-max credential moving 10->15 with the max unchanged appends zero", () => {
    const first = recordsFromEntries([cred("cred_a", 10, T1), cred("cred_b", 20, T1)], new Map(), "quota-gatherer");
    const state = new Map(first.appended);
    const { records } = recordsFromEntries([cred("cred_a", 15, T2), cred("cred_b", 20, T2)], state, "quota-gatherer");
    expect(records).toHaveLength(0);
  });

  test("one recordsFromEntries call never emits two records for the same serviceId|window key", () => {
    const { records } = recordsFromEntries(
      [cred("cred_a", 10, T1), cred("cred_b", 20, T1)],
      new Map(),
      "quota-gatherer",
    );
    const keys = records.map((r) => `${r.serviceId}|${r.metricId}`);
    expect(new Set(keys).size).toBe(keys.length);
  });
});

describe("entryFromFileExport (Meta #1301 convention)", () => {
  const okDoc = {
    service: "meta",
    source: "file-export/meta-laptop",
    status: "ok",
    measuredAt: "2026-10-27T05:30:00.000Z",
    tier: "PAID",
    windows: [
      { name: "5h", usedPct: 12, resetAt: "2026-10-27T10:30:00.000Z" },
      { name: "weekly", usedPct: 46, resetAt: "2026-11-02T01:00:00.000Z" },
    ],
    freeCreditsUsd: 3.5,
    paygBalanceUsd: 0,
  };
  test("success document maps windows + dollar labels", () => {
    const entry = entryFromFileExport(okDoc, { fileMtimeIso: "2026-10-27T05:30:05.000Z", stale: false });
    expect(entry.serviceId).toBe("meta");
    expect(entry.state).toBe("ok");
    expect(entry.source).toBe("file-export/meta-laptop");
    expect(entry.planLabel).toBe("PAID");
    const weekly = entry.windows.find((w) => w.name === "weekly");
    expect(weekly?.pctOfLimit).toBe(46);
    expect(weekly?.windowSeconds).toBe(604800);
    expect(entry.windows.find((w) => w.name === "free_credits")?.valueLabel).toBe("$3.50");
    expect(entry.windows.find((w) => w.name === "payg_balance")?.valueLabel).toBe("$0.00");
    // ...and its pct windows flow into snapshot records like any provider's.
    const { records } = recordsFromEntries([entry], new Map(), "quota-gatherer");
    expect(records.map((r) => r.metricId).sort()).toEqual(["5h", "weekly"]);
    expect(records.every((r) => r.source === "console")).toBe(true);
  });
  test("out-of-range percentages and negative dollar labels are omitted", () => {
    const doc = {
      source: "file-export/test",
      status: "ok",
      measuredAt: "2026-10-27T05:30:00.000Z",
      windows: [
        { name: "below", usedPct: -20 },
        { name: "above", usedPct: 150 },
        { name: "huge", usedPct: 1e9 },
        { name: "null", usedPct: null },
        { name: "zero", usedPct: 0 },
        { name: "full", usedPct: 100 },
        { name: "valid", usedPct: 42 },
      ],
      freeCreditsUsd: -42.5,
      paygBalanceUsd: -1,
    };
    const entry = entryFromFileExport(doc, { fileMtimeIso: doc.measuredAt, stale: false });
    expect(entry.windows.map((w) => [w.name, w.pctOfLimit, w.valueLabel])).toEqual([
      ["zero", 0, null],
      ["full", 100, null],
      ["valid", 42, null],
    ]);
    const { records } = recordsFromEntries([entry], new Map(), "quota-gatherer");
    expect(records.map((r) => [r.metricId, r.value])).toEqual([
      ["full", 100],
      ["valid", 42],
      ["zero", 0],
    ]);

    const nonnegativeLabels = entryFromFileExport(
      { ...doc, windows: [], freeCreditsUsd: 0, paygBalanceUsd: 12.5 },
      { fileMtimeIso: doc.measuredAt, stale: false },
    );
    expect(nonnegativeLabels.windows.map((w) => [w.name, w.valueLabel])).toEqual([
      ["free_credits", "$0.00"],
      ["payg_balance", "$12.50"],
    ]);
  });
  test("failure status documents are error entries, never zeros", () => {
    const entry = entryFromFileExport(
      { service: "meta", source: "file-export/meta-laptop", status: "session-expired", measuredAt: null },
      { fileMtimeIso: "2026-10-27T05:30:05.000Z", stale: false },
    );
    expect(entry.state).toBe("error");
    expect(entry.error).toContain("session-expired");
    expect(entry.windows).toEqual([]);
  });
  test("stale and missing files are error entries", () => {
    const stale = entryFromFileExport(okDoc, { fileMtimeIso: "2026-10-27T04:00:00.000Z", stale: true });
    expect(stale.state).toBe("error");
    expect(stale.error).toContain("stale");
    const missing = entryFromFileExport(null, { fileMtimeIso: null, stale: false });
    expect(missing.state).toBe("error");
    expect(missing.error).toContain("unreadable");
  });
});

describe("serviceIdFor", () => {
  test("known pools map to hub ids; others pass through", () => {
    expect(serviceIdFor("_openai")).toBe("_openai.main");
    expect(serviceIdFor("deepseek")).toBe("deepseek");
    expect(serviceIdFor("meta")).toBe("meta");
    expect(serviceIdFor("_muse")).toBe("meta");
    expect(serviceIdFor("opendesign")).toBe("opendesign");
    // The module's provider id is "command-code"; the pool (and the
    // OpenCode config provider) is "commandcode". Without the mapping
    // the read filed under a key no pool carries and never landed
    // (the silent-mismatch family Drew ruled out, 2026-11-03).
    expect(serviceIdFor("command-code")).toBe("commandcode");
    expect(serviceIdFor("_openai.alt")).toBe("_openai.alt");
  });
});

describe("entryFromTurso (#1049)", () => {
  // Fixture values are the triage-verified live read of 2026-10-29
  // (task 2311): org dezren39, Starter plan, period 2026-10-01..11-01.
  const usageDoc = {
    organization: {
      slug: "dezren39",
      usage: {
        rows_read: 1_905_385,
        rows_written: 30_026,
        storage_bytes: 1_053_323_264,
        bytes_synced: 30_408_704,
        databases: 3,
        locations: 1,
        groups: 1,
      },
    },
  };
  const subDoc = {
    subscription: {
      plan: "starter",
      name: "starter",
      current_billing_period_start: "2026-10-01T04:00:00+00:00",
      current_billing_period_end: "2026-11-01T04:00:00+00:00",
    },
  };
  const at = "2026-10-29T02:30:00.000Z";
  test("maps org usage onto the turso entry vs Starter limits, period end as cycle end", () => {
    const entry = entryFromTurso(usageDoc, subDoc, { measuredAt: at });
    expect(entry.providerId).toBe("turso");
    expect(entry.serviceId).toBe("turso");
    expect(entry.state).toBe("ok");
    expect(entry.planLabel).toBe("starter");
    const byName = new Map(entry.windows.map((w) => [w.name, w]));
    expect(byName.get("databases")?.value).toBe(3);
    expect(byName.get("databases")?.pctOfLimit).toBe(3);
    expect(byName.get("storage")?.value).toBe(1.053323);
    expect(byName.get("storage")?.pctOfLimit).toBe(21.066);
    expect(byName.get("rows-read")?.value).toBe(1_905_385);
    expect(byName.get("rows-read")?.pctOfLimit).toBe(0.381);
    expect(byName.get("rows-written")?.pctOfLimit).toBe(0.3);
    expect(byName.get("sync")?.value).toBe(0.030409);
    expect(byName.get("sync")?.pctOfLimit).toBe(1.014);
    for (const w of entry.windows) {
      expect(w.resetsAt).toBe("2026-11-01T04:00:00.000Z");
      expect(w.windowSeconds).toBe(2_678_400);
      expect(w.source).toBe("turso-api");
    }
    // ...and the windows flow into §4 snapshot records (source api):
    // databases/storage/sync in their NATIVE units (the series
    // recordsFromSaasServices already writes), rows in pct (the
    // store has no "rows" unit and no native rows series exists).
    const { records } = recordsFromEntries([entry], new Map(), "quota-gatherer");
    const byMetric = new Map(records.map((r) => [r.metricId, r]));
    expect([...byMetric.keys()].sort()).toEqual(["databases", "rows-read", "rows-written", "storage", "sync"]);
    expect(byMetric.get("databases")).toMatchObject({ value: 3, unit: "count", limit: 100, source: "api" });
    expect(byMetric.get("storage")).toMatchObject({ value: 1.053323, unit: "gb", limit: 5, source: "api" });
    expect(byMetric.get("sync")).toMatchObject({ value: 0.030409, unit: "gb", limit: 3, source: "api" });
    expect(byMetric.get("rows-read")).toMatchObject({ value: 0.381, unit: "pct", limit: 100, source: "api" });
    expect(byMetric.get("rows-written")).toMatchObject({ value: 0.3, unit: "pct", limit: 100, source: "api" });
  });
  test("missing metrics are omitted, never zeroed; a partial read is still ok", () => {
    const entry = entryFromTurso({ organization: { usage: { databases: 3 } } }, null, { measuredAt: at });
    expect(entry.state).toBe("ok");
    expect(entry.windows.map((w) => w.name)).toEqual(["databases"]);
    expect(entry.windows[0].resetsAt).toBeNull();
  });
  test("failed or empty reads are error entries with no windows and no records", () => {
    const failed = entryFromTurso(null, null, { measuredAt: at, error: "turso /usage HTTP 401" });
    expect(failed.state).toBe("error");
    expect(failed.error).toContain("401");
    expect(failed.windows).toEqual([]);
    expect(recordsFromEntries([failed], new Map(), "quota-gatherer").records).toEqual([]);
    const empty = entryFromTurso({ organization: { usage: {} } }, subDoc, { measuredAt: at });
    expect(empty.state).toBe("error");
    expect(empty.windows).toEqual([]);
  });
  test("over-limit usage is kept with its native value and >100 pct (#6227)", () => {
    // The 2026-11-05 live shape from issue #6227: storage and
    // rows-read over their Starter limits — the exact windows the
    // [0,100] percent gate used to drop, leaving a fresh-looking
    // partial row that read as recovery.
    const entry = entryFromTurso(
      {
        organization: {
          usage: {
            databases: 44,
            storage_bytes: 7_358_009_000,
            rows_read: 821_437_321,
            rows_written: 1_672_071,
            bytes_synced: 0,
          },
        },
      },
      subDoc,
      { measuredAt: at },
    );
    expect(entry.state).toBe("ok");
    const byName = new Map(entry.windows.map((w) => [w.name, w]));
    expect(byName.get("storage")?.value).toBe(7.358009);
    expect(byName.get("storage")?.pctOfLimit).toBe(147.16);
    expect(byName.get("rows-read")?.value).toBe(821_437_321);
    expect(byName.get("rows-read")?.pctOfLimit).toBe(164.287);
    expect(byName.get("databases")?.pctOfLimit).toBe(44);
    expect(byName.get("sync")?.pctOfLimit).toBe(0);
    const { records } = recordsFromEntries([entry], new Map(), "quota-gatherer");
    const byMetric = new Map(records.map((r) => [r.metricId, r]));
    expect(byMetric.get("storage")).toMatchObject({ value: 7.358009, unit: "gb", limit: 5 });
    expect(byMetric.get("rows-read")).toMatchObject({ value: 164.287, unit: "pct" });
  });
  test("dynamic limits: a /plans doc matched by plan name supplies the denominators (#5468)", () => {
    // The 2026-11-08 live shape: org on Scaler since 2026-11-04, but
    // Starter denominators made rows-read read ~235% and storage
    // ~213%. /plans is the source of truth.
    const plansDoc = {
      plans: [
        {
          name: "starter",
          quotas: {
            rowsRead: 500_000_000,
            rowsWritten: 10_000_000,
            databases: 100,
            storage: 5_000_000_000,
            bytesSynced: 3_000_000_000,
          },
        },
        {
          name: "Scaler",
          quotas: {
            rowsRead: 100_000_000_000,
            rowsWritten: 100_000_000,
            databases: 1_000_000_000,
            storage: 24_000_000_000,
            bytesSynced: 24_000_000_000,
          },
        },
      ],
    };
    const entry = entryFromTurso(
      {
        organization: {
          usage: {
            databases: 40,
            storage_bytes: 10_800_000_000,
            rows_read: 1_180_000_000,
            rows_written: 4_000_000,
            bytes_synced: 1_200_000_000,
          },
        },
      },
      { subscription: { plan: "scaler", name: "scaler" } },
      { measuredAt: at, plansDoc },
    );
    expect(entry.state).toBe("ok");
    expect(entry.planLabel).toBe("scaler");
    const byName = new Map(entry.windows.map((w) => [w.name, w]));
    expect(byName.get("rows-read")?.pctOfLimit).toBe(1.18);
    expect(byName.get("rows-read")?.valueLabel).toBe("1,180,000,000 / 100,000,000,000");
    expect(byName.get("storage")?.pctOfLimit).toBe(45);
    expect(byName.get("storage")?.valueLabel).toBe("10.8 GB / 24 GB");
    expect(byName.get("rows-written")?.pctOfLimit).toBe(4);
    expect(byName.get("sync")?.pctOfLimit).toBe(5);
    expect(byName.get("sync")?.valueLabel).toBe("1.2 GB / 24 GB");
    // Native-unit snapshot records carry the plan's limits too.
    const { records } = recordsFromEntries([entry], new Map(), "quota-gatherer");
    const byMetric = new Map(records.map((r) => [r.metricId, r]));
    expect(byMetric.get("storage")).toMatchObject({ value: 10.8, unit: "gb", limit: 24 });
    expect(byMetric.get("sync")).toMatchObject({ value: 1.2, unit: "gb", limit: 24 });
    expect(byMetric.get("databases")).toMatchObject({ value: 40, unit: "count", limit: 1_000_000_000 });
    expect(byMetric.get("rows-read")).toMatchObject({ value: 1.18, unit: "pct", limit: 100 });
  });
  test("plans doc absent: the per-plan fallback applies, so Scaler never regresses to Starter (#5468)", () => {
    const entry = entryFromTurso(
      { organization: { usage: { rows_read: 1_180_000_000, storage_bytes: 10_800_000_000 } } },
      { subscription: { plan: "scaler", name: "scaler" } },
      { measuredAt: at },
    );
    const byName = new Map(entry.windows.map((w) => [w.name, w]));
    expect(byName.get("rows-read")?.pctOfLimit).toBe(1.18);
    expect(byName.get("storage")?.pctOfLimit).toBe(45);
    expect(byName.get("storage")?.valueLabel).toBe("10.8 GB / 24 GB");
    const { records } = recordsFromEntries([entry], new Map(), "quota-gatherer");
    const byMetric = new Map(records.map((r) => [r.metricId, r]));
    expect(byMetric.get("storage")).toMatchObject({ unit: "gb", limit: 24 });
  });
});

describe("entryFromTurso period stamps (#5091)", () => {
  const usageDoc = { organization: { usage: { databases: 3 } } };
  const at = "2026-10-29T02:30:00.000Z";
  const subWith = (start: unknown, end: unknown) => ({
    subscription: {
      plan: "starter",
      current_billing_period_start: start,
      current_billing_period_end: end,
    },
  });
  const windowSecondsOf = (sub: unknown): number | null => {
    const entry = entryFromTurso(usageDoc, sub as never, { measuredAt: at });
    expect(entry.state).toBe("ok");
    expect(entry.windows.length).toBeGreaterThan(0);
    return entry.windows[0].windowSeconds;
  };
  test("impossible period start yields a null window, not a rolled-over length", () => {
    // 2026-02-30 does not exist; raw Date.parse rolls it to Mar 2 and would
    // publish a 28-day window against the 2026-03-30 end.
    expect(windowSecondsOf(subWith("2026-02-30T00:00:00Z", "2026-03-30T00:00:00Z"))).toBeNull();
  });
  test("impossible period end yields a null window", () => {
    expect(windowSecondsOf(subWith("2026-02-01T00:00:00Z", "2026-02-30T00:00:00Z"))).toBeNull();
  });
  test("non-ISO period stamps yield a null window", () => {
    // Raw Date.parse accepts these (timezone-dependently); the gate refuses.
    expect(windowSecondsOf(subWith("February 1, 2026", "March 3, 2026"))).toBeNull();
  });
  test("valid ISO period stamps still compute the window", () => {
    // 2026-02-01 -> 2026-03-03 is exactly 30 days.
    expect(windowSecondsOf(subWith("2026-02-01T00:00:00Z", "2026-03-03T00:00:00Z"))).toBe(2_592_000);
  });
});

describe("entryFromTurso negative usage (#2553)", () => {
  const subDoc = {
    subscription: {
      plan: "starter",
      name: "starter",
      current_billing_period_start: "2026-10-01T04:00:00+00:00",
      current_billing_period_end: "2026-11-01T04:00:00+00:00",
    },
  };
  const at = "2026-10-29T02:30:00.000Z";
  test("all-negative usage read is an error entry with no windows and no records", () => {
    const entry = entryFromTurso(
      {
        organization: {
          usage: { databases: -3, storage_bytes: -2, rows_read: -5, rows_written: -1, bytes_synced: -1 },
        },
      },
      subDoc,
      { measuredAt: at },
    );
    expect(entry.state).toBe("error");
    expect(entry.error).toContain("no known metrics");
    expect(entry.windows).toEqual([]);
    expect(recordsFromEntries([entry], new Map(), "quota-gatherer").records).toEqual([]);
  });
  test("one negative metric is omitted; the valid metrics publish unchanged and no record is negative", () => {
    const entry = entryFromTurso(
      {
        organization: {
          usage: {
            databases: -3,
            storage_bytes: 1_053_323_264,
            rows_read: 1_905_385,
            rows_written: 30_026,
            bytes_synced: -1,
          },
        },
      },
      subDoc,
      { measuredAt: at },
    );
    expect(entry.state).toBe("ok");
    expect(entry.windows.map((w) => w.name).sort()).toEqual(["rows-read", "rows-written", "storage"]);
    for (const w of entry.windows) {
      expect(w.value).toBeGreaterThanOrEqual(0);
      expect(w.pctOfLimit).toBeGreaterThanOrEqual(0);
    }
    const { records } = recordsFromEntries([entry], new Map(), "quota-gatherer");
    expect(records.map((r) => r.metricId).sort()).toEqual(["rows-read", "rows-written", "storage"]);
    for (const r of records) {
      expect(r.value).toBeGreaterThanOrEqual(0);
    }
  });
});

describe("pool splits (#1331)", () => {
  const TICK = "2026-11-03T05:00:00.000Z";
  const goResult = (label: string | undefined, id: string): QuotaProviderResult => ({
    providerId: "opencode-go",
    providerName: "OpenCode Go",
    ok: true,
    configured: true,
    fetchedAt: Date.parse(TICK),
    credentialId: id,
    ...(label === undefined ? {} : { credentialLabel: label }),
    usage: {
      windows: {
        "5h": { usedPercent: 12, windowSeconds: 18_000, resetAt: Date.parse(TICK) + 18_000_000 },
        monthly: { usedPercent: 60, windowSeconds: null, resetAt: Date.parse("2026-12-03T03:02:00.000Z") },
      },
    },
  });

  test("Go per-workspace entries yield drewrypope/popesdrew split rows alongside the combined rows", () => {
    const entries = [
      entryFromResult(goResult("drewrypope", "cred_drewrypope"), TICK),
      entryFromResult(goResult("popesdrew", "cred_popesdrew"), TICK),
    ];
    expect(entries.map((e) => e.serviceId)).toEqual(["opencode-go", "opencode-go"]);
    const split = entriesWithPoolSplits(entries);
    expect(split.map((e) => e.serviceId).sort()).toEqual([
      "opencode-go",
      "opencode-go",
      GO_DREWRYPOPE_SERVICE_ID,
      GO_POPESDREW_SERVICE_ID,
    ]);
    const personal = split.find((e) => e.serviceId === GO_DREWRYPOPE_SERVICE_ID)!;
    expect(personal.providerId).toBe("opencode-go");
    expect(personal.credentialId).toBe("cred_drewrypope");
    expect(personal.credentialLabel).toBe("drewrypope");
    expect(personal.state).toBe("ok");
    expect(personal.measuredAt).toBe(entries[0].measuredAt);
    expect(personal.windows).toEqual(entries[0].windows);
    const lab = split.find((e) => e.serviceId === GO_POPESDREW_SERVICE_ID)!;
    expect(lab.credentialId).toBe("cred_popesdrew");
    // ...and the pool-level series continue in the snapshot records.
    const { records } = recordsFromEntries(split, new Map(), "quota-gatherer");
    expect(records.map((r) => `${r.serviceId}|${r.metricId}`).sort()).toEqual([
      "opencode-go.drewrypope|5h",
      "opencode-go.drewrypope|monthly",
      "opencode-go.popesdrew|5h",
      "opencode-go.popesdrew|monthly",
      "opencode-go|5h",
      "opencode-go|monthly",
    ]);
  });

  test("Go lapse: a lapsed drewrypope row splits as a pool lapsed; unconfigured rows never split", () => {
    const lapsed: QuotaProviderResult = {
      providerId: "opencode-go",
      providerName: "OpenCode Go",
      ok: false,
      configured: true,
      fetchedAt: Date.parse(TICK),
      credentialId: "cred_personal",
      credentialLabel: "drewrypope",
      error: "No active OpenCode Go subscription on the selected Console account",
      // Even if a provider includes old usage alongside the lapse error,
      // it must not be published as a current quota window or snapshot.
      usage: { windows: { monthly: { usedPercent: 100 } } },
    };
    const unconfigured: QuotaProviderResult = {
      providerId: "opencode-go",
      providerName: "OpenCode Go",
      ok: false,
      configured: false,
      credentialId: "cred_lab",
      credentialLabel: "popesdrew",
      error: "Not configured",
      usage: null,
    };
    const split = entriesWithPoolSplits([entryFromResult(lapsed, TICK), entryFromResult(unconfigured, TICK)]);
    expect(split.map((e) => e.serviceId).sort()).toEqual(["opencode-go", "opencode-go", GO_DREWRYPOPE_SERVICE_ID]);
    const personal = split.find((e) => e.serviceId === GO_DREWRYPOPE_SERVICE_ID)!;
    expect(personal.state).toBe("lapsed");
    expect(personal.error).toContain("No active OpenCode Go subscription");
    expect(personal.windows).toEqual([]);
    expect(recordsFromEntries(split, new Map(), "quota-gatherer").records).toEqual([]);
    // Control: a configured Go failure without the lapse signal stays an error.
    const failed = entryFromResult(
      {
        providerId: "opencode-go",
        providerName: "OpenCode Go",
        ok: false,
        configured: true,
        fetchedAt: Date.parse(TICK),
        credentialId: "cred_personal",
        credentialLabel: "drewrypope",
        error: "timeout",
        usage: null,
      },
      TICK,
    );
    expect(failed.state).toBe("error");
    expect(failed.error).toContain("timeout");
    // The signal only maps for the opencode-go provider itself: a
    // different provider with the same words still throws an error.
    const sameReason = entryFromResult(
      {
        providerId: "claude",
        ok: false,
        configured: true,
        error: "No active OpenCode Go subscription on the selected Console account",
        usage: null,
      },
      TICK,
    );
    expect(sameReason.state).toBe("error");
  });

  test("Meta session lapse (#8294): session-class failures map to lapsed with the re-auth remedy named", () => {
    const base = {
      providerId: "meta",
      providerName: "Meta",
      configured: true,
      fetchedAt: Date.parse(TICK),
    } as const;
    // failureClass-carried unauthorized (the gatherer's session signal).
    const byClass = entryFromResult(
      { ...base, ok: false, failureClass: "unauthorized", error: "unauthorized", usage: null },
      TICK,
    );
    expect(byClass.state).toBe("lapsed");
    expect(byClass.serviceId).toBe("meta");
    expect(byClass.windows).toEqual([]);
    expect(byClass.error).toContain("session lapsed");
    expect(byClass.error).toContain("re-auth");
    // The exporter's own shape: a session-expired error string.
    const byError = entryFromResult(
      { ...base, ok: false, error: "session-expired: dev.meta.ai session is logged out", usage: null },
      TICK,
    );
    expect(byError.state).toBe("lapsed");
    expect(byError.error).toContain("re-auth");
    // Control: a configured Meta failure without the session signal
    // stays a provider error.
    const failed = entryFromResult({ ...base, ok: false, error: "timeout", usage: null }, TICK);
    expect(failed.state).toBe("error");
    // Control: the session words on a different provider stay an error.
    const other = entryFromResult(
      { providerId: "claude", ok: false, configured: true, error: "unauthorized", usage: null },
      TICK,
    );
    expect(other.state).toBe("error");
    // Recovery shape: the next good read is a plain ok entry again.
    const recovered = entryFromResult({ ...base, ok: true, usage: { windows: { weekly: { usedPercent: 42 } } } }, TICK);
    expect(recovered.state).toBe("ok");
    expect(recovered.error).toBeUndefined();
  });

  test("Go workspaces without a hub pool stay combined-only; labels match case-insensitively", () => {
    expect(goPoolServiceId(entryFromResult(goResult("Home", "cred_home"), TICK))).toBeNull();
    expect(goPoolServiceId(entryFromResult(goResult(undefined, "cred_x"), TICK))).toBeNull();
    expect(goPoolServiceId(entryFromResult(goResult("  DREWRYPOPE ", "cred_p"), TICK))).toBe(GO_DREWRYPOPE_SERVICE_ID);
    expect(goPoolServiceId(entryFromResult(goResult("popesdrew", "cred_l"), TICK))).toBe(GO_POPESDREW_SERVICE_ID);
    const split = entriesWithPoolSplits([entryFromResult(goResult("Home", "cred_home"), TICK)]);
    expect(split.map((e) => e.serviceId)).toEqual(["opencode-go"]);
  });

  const antigravityResult = (models: Record<string, Record<string, number>>): QuotaProviderResult => ({
    providerId: "_antigravity",
    providerName: "Antigravity",
    ok: true,
    configured: true,
    fetchedAt: Date.parse(TICK),
    usage: {
      models: Object.fromEntries(
        Object.entries(models).map(([model, wins]) => [
          model,
          {
            windows: Object.fromEntries(
              Object.entries(wins).map(([name, pct]) => [
                name,
                { usedPercent: pct, windowSeconds: 604_800, resetAt: Date.parse("2026-11-10T05:00:00.000Z") },
              ]),
            ),
          },
        ]),
      ),
    },
  });

  test("Antigravity models group into gemini + claude-gpt pool rows; hottest window wins", () => {
    const entry = entryFromResult(
      antigravityResult({
        "gemini-2.5-pro": { "5h": 70, weekly: 40 },
        "gemini-flash": { weekly: 55 },
        "claude-sonnet": { "5h": 20, weekly: 90 },
        "gpt-5": { weekly: 10 },
      }),
      TICK,
    );
    expect(entry.serviceId).toBe("_antigravity");
    expect(entry.windows).toHaveLength(6);
    const split = entriesWithPoolSplits([entry]);
    expect(split).toHaveLength(3);
    expect(split[0].serviceId).toBe("_antigravity");
    const gemini = split.find((e) => e.serviceId === ANTIGRAVITY_GEMINI_SERVICE_ID)!;
    expect(gemini.windows.map((w) => [w.name, w.pctOfLimit])).toEqual([
      ["5h", 70],
      ["weekly", 55],
    ]);
    const claudeGpt = split.find((e) => e.serviceId === ANTIGRAVITY_CLAUDE_GPT_SERVICE_ID)!;
    expect(claudeGpt.windows.map((w) => [w.name, w.pctOfLimit])).toEqual([
      ["5h", 20],
      ["weekly", 90],
    ]);
    expect(gemini.measuredAt).toBe(entry.measuredAt);
    expect(gemini.source).toBe(entry.source);
  });

  test("Antigravity grouping is order-independent and omits empty pools", () => {
    const models = {
      "gemini-2.5-pro": { weekly: 40 },
      "claude-sonnet": { weekly: 90 },
    };
    const fwd = entriesWithPoolSplits([entryFromResult(antigravityResult(models), TICK)]);
    const rev = entriesWithPoolSplits([
      entryFromResult(antigravityResult({ "claude-sonnet": { weekly: 90 }, "gemini-2.5-pro": { weekly: 40 } }), TICK),
    ]);
    // The combined entry preserves module order by design; the derived
    // pool rows must not depend on it.
    const rowsOf = (split: ReturnType<typeof entriesWithPoolSplits>) =>
      split.slice(1).map((e) => [e.serviceId, e.windows.map((w) => [w.name, w.pctOfLimit])]);
    expect(rowsOf(fwd)).toEqual(rowsOf(rev));
    const geminiOnly = entriesWithPoolSplits([
      entryFromResult(antigravityResult({ "gemini-flash": { weekly: 55 } }), TICK),
    ]);
    expect(geminiOnly.map((e) => e.serviceId).sort()).toEqual(["_antigravity", ANTIGRAVITY_GEMINI_SERVICE_ID]);
  });

  test("Antigravity error lapses both pools; unconfigured splits nothing", () => {
    const err = entryFromResult(
      {
        providerId: "_antigravity",
        ok: false,
        configured: true,
        error: "Failed to refresh Antigravity token",
        usage: null,
      },
      TICK,
    );
    const split = entriesWithPoolSplits([err]);
    expect(split.map((e) => e.serviceId).sort()).toEqual([
      "_antigravity",
      ANTIGRAVITY_CLAUDE_GPT_SERVICE_ID,
      ANTIGRAVITY_GEMINI_SERVICE_ID,
    ]);
    for (const row of split.slice(1)) {
      expect(row.state).toBe("error");
      expect(row.error).toContain("Failed to refresh");
    }
    const unconfigured = entriesWithPoolSplits([
      entryFromResult(
        { providerId: "_antigravity", ok: false, configured: false, error: "Not configured", usage: null },
        TICK,
      ),
    ]);
    expect(unconfigured.map((e) => e.serviceId)).toEqual(["_antigravity"]);
  });
});

describe("own-key guards on quota-gather-map lookups (#2781)", () => {
  const at = "2026-10-27T05:30:00.000Z";
  test("entryFromFileExport: prototype spellings get windowSeconds null; known names unchanged", () => {
    const doc = {
      service: "meta",
      source: "file-export/meta-laptop",
      status: "ok",
      measuredAt: at,
      windows: [
        { name: "5h", usedPct: 12 },
        { name: "weekly", usedPct: 46 },
        { name: "storage", usedPct: 3 },
        { name: "constructor", usedPct: 10 },
        { name: "__proto__", usedPct: 20 },
        { name: "toString", usedPct: 30 },
        { name: "hasOwnProperty", usedPct: 40 },
      ],
    };
    const entry = entryFromFileExport(doc, { fileMtimeIso: at, stale: false });
    const byName = new Map(entry.windows.map((w) => [w.name, w.windowSeconds]));
    expect(byName.get("5h")).toBe(18000);
    expect(byName.get("weekly")).toBe(604800);
    // "storage" is a genuinely unknown file-export window: null, like the
    // prototype spellings must be after the guard.
    expect(byName.get("storage")).toBeNull();
    for (const name of ["constructor", "__proto__", "toString", "hasOwnProperty"]) {
      expect(byName.get(name)).toBeNull();
    }
    // The published shape: every windowSeconds is a number or null — no
    // function/object prototype member survives serialization.
    const published = JSON.parse(JSON.stringify(entry)) as typeof entry;
    for (const w of published.windows) {
      expect(w.windowSeconds === null || typeof w.windowSeconds === "number").toBe(true);
    }
  });

  test("recordsFromEntries: prototype-spelled Turso windows take the pct path; storage control stays native", () => {
    // Windows arrive via JSON.parse in production, which is what makes
    // "__proto__" an own key of the windows record — build the fixture
    // the same way so the probe is truthful.
    const usage = JSON.parse(
      '{"windows":{"storage":{"usedPercent":40},"constructor":{"usedPercent":42},"__proto__":{"usedPercent":7},"toString":{"usedPercent":9}}}',
    ) as QuotaProviderResult["usage"];
    const entry = entryFromResult(
      { providerId: "turso", providerName: "Turso", ok: true, configured: true, fetchedAt: 1793076600000, usage },
      at,
    );
    expect(entry.serviceId).toBe("turso");
    const { records } = recordsFromEntries([entry], new Map(), "quota-gatherer");
    const byMetric = new Map(records.map((r) => [r.metricId, r]));
    // Control: storage keeps its native-unit record.
    expect(byMetric.get("storage")).toMatchObject({ unit: "gb", limit: 5, value: 40 });
    // Prototype spellings: ordinary pct records, never fabricated
    // native records with undefined unit/limit.
    for (const [name, value] of [
      ["constructor", 42],
      ["__proto__", 7],
      ["toString", 9],
    ] as const) {
      expect(byMetric.get(name)).toMatchObject({ unit: "pct", limit: 100, value });
    }
    for (const r of records) {
      expect(typeof r.unit).toBe("string");
      expect(Number.isFinite(r.limit)).toBe(true);
    }
  });
});

describe("canonicalProviderId (config-id alignment, #1823)", () => {
  const TICK = "2026-10-27T05:00:00.000Z";

  test("module spellings map to the config provider ids; others pass through", () => {
    expect(canonicalProviderId("antigravity")).toBe("_antigravity");
    expect(canonicalProviderId("claude")).toBe("_claude");
    expect(canonicalProviderId("command-code")).toBe("_commandcode");
    expect(canonicalProviderId("meta")).toBe("_muse");
    expect(canonicalProviderId("_antigravity")).toBe("_antigravity");
    expect(canonicalProviderId("opendesign")).toBe("opendesign");
    expect(canonicalProviderId("codex")).toBe("_openai");
  });

  test("entryFromResult emits the canonical id and keeps the pool service id", () => {
    const ag = entryFromResult({ ...OK_RESULT, providerId: "antigravity" }, TICK);
    expect(ag.providerId).toBe("_antigravity");
    expect(ag.serviceId).toBe("_antigravity");
    const claude = entryFromResult({ ...OK_RESULT, providerId: "claude" }, TICK);
    expect(claude.providerId).toBe("_claude");
    expect(claude.serviceId).toBe("anthropic:claude-pro");
    const cc = entryFromResult({ ...OK_RESULT, providerId: "command-code" }, TICK);
    expect(cc.providerId).toBe("_commandcode");
    expect(cc.serviceId).toBe("_commandcode");
    const meta = entryFromResult({ ...OK_RESULT, providerId: "meta" }, TICK);
    expect(meta.providerId).toBe("_muse");
    expect(meta.serviceId).toBe("meta");
  });

  test("a module-spelled Antigravity entry splits into the renamed pool rows", () => {
    const entry = entryFromResult(
      {
        providerId: "antigravity",
        providerName: "Antigravity",
        ok: true,
        configured: true,
        fetchedAt: Date.parse(TICK),
        usage: {
          models: {
            "gemini-2.5-pro": {
              windows: { weekly: { usedPercent: 40, windowSeconds: 604_800, resetAt: 1793438427299 } },
            },
            "claude-sonnet": {
              windows: { weekly: { usedPercent: 90, windowSeconds: 604_800, resetAt: 1793438427299 } },
            },
          },
        },
      },
      TICK,
    );
    const splits = entriesWithPoolSplits([entry]);
    expect(splits.map((e) => e.serviceId)).toEqual([
      "_antigravity",
      ANTIGRAVITY_CLAUDE_GPT_SERVICE_ID,
      ANTIGRAVITY_GEMINI_SERVICE_ID,
    ]);
  });

  test("retention matches across the rename boundary and re-stamps the canonical id", () => {
    const prevEntry = { ...entryFromResult(OK_RESULT, TICK), providerId: "claude" };
    expect(lastKnownEntries([prevEntry], "claude")).toHaveLength(1);
    expect(lastKnownEntries([prevEntry], "_claude")).toHaveLength(1);
    const retained = markRetained([prevEntry], "2026-10-27T06:00:00.000Z");
    expect(retained[0].providerId).toBe("_claude");
    expect(retained[0].retainedFromFailedTick).toBe("2026-10-27T06:00:00.000Z");
  });
});

describe("openaiPoolServiceId — folded _openai rows file under their own OpenAI pool", () => {
  const TICK = "2026-11-10T21:30:00.000Z";
  const win = (usedPercent: number, windowSeconds: number) => ({
    usedPercent,
    remainingPercent: 100 - usedPercent,
    windowSeconds,
    resetAt: 1794364348000,
  });
  const foldedResult = (
    // the fork's folded OpenAI quota answer (providerId "_openai" since fork PR #2261; "codex" before)
    credentialLabel: string | undefined,
    providerName: string,
    windows: Record<string, ReturnType<typeof win>>,
  ): QuotaProviderResult => ({
    providerId: "_openai",
    providerName,
    ok: true,
    configured: true,
    fetchedAt: Date.parse(TICK),
    credentialId: "cred_x",
    credentialLabel,
    usage: { windows },
  });

  test("each instance credential lands on its own serviceId", () => {
    const alt = entryFromResult(
      foldedResult("OpenAI Alternate (Student - popesdrew)", "OpenAI Alternate (Student - popesdrew)", {
        "5h": win(42, 18_000),
        weekly: win(65, 604_800),
      }),
      TICK,
    );
    expect(alt.serviceId).toBe(OPENAI_ALTERNATE_SERVICE_ID);
    const main = entryFromResult(
      foldedResult("OpenAI Main (drewrypope)", "OpenAI Main (drewrypope)", { weekly: win(100, 604_800) }),
      TICK,
    );
    expect(main.serviceId).toBe(OPENAI_MAIN_SERVICE_ID);
    const self = entryFromResult(
      foldedResult("OpenAI (_openai, drewrypope) — self-contained", "OpenAI Self-Contained (drewrypope, no Codex)", {
        weekly: win(100, 604_800),
      }),
      TICK,
    );
    expect(self.serviceId).toBe(OPENAI_SELF_SERVICE_ID);
  });

  test("unmatched folded rows keep the _openai.main fallback; openaiPoolServiceId is null", () => {
    expect(openaiPoolServiceId({ credentialLabel: undefined, providerName: "OpenAI" })).toBeNull();
    expect(openaiPoolServiceId({ credentialLabel: "some future account", providerName: "OpenAI" })).toBeNull();
    const bare = entryFromResult(foldedResult(undefined, "OpenAI", { weekly: win(10, 604_800) }), TICK);
    expect(bare.serviceId).toBe("_openai.main");
  });

  test("a pre-rename fork answer (providerId codex) still files under the new ids", () => {
    const legacy = entryFromResult(
      {
        ...foldedResult("OpenAI Alternate (Student - popesdrew)", "OpenAI Alternate (Student - popesdrew)", {
          weekly: win(65, 604_800),
        }),
      },
      TICK,
    );
    expect(legacy.serviceId).toBe("_openai.alt");
    const legacyRaw = entryFromResult(
      {
        ...foldedResult("OpenAI Alternate (Student - popesdrew)", "OpenAI Alternate (Student - popesdrew)", {
          weekly: win(65, 604_800),
        }),
        providerId: "codex",
      },
      TICK,
    );
    expect(legacyRaw.providerId).toBe("_openai");
    expect(legacyRaw.serviceId).toBe("_openai.alt");
  });

  test("recordsFromEntries: the alternate account's 5h never enters _openai.main's series", () => {
    const alt = entryFromResult(
      foldedResult("OpenAI Alternate (Student - popesdrew)", "OpenAI Alternate (Student - popesdrew)", {
        "5h": win(42, 18_000),
        weekly: win(65, 604_800),
      }),
      TICK,
    );
    const mainDedicated = entryFromResult(
      {
        providerId: "_openai.main",
        providerName: "OpenAI Main (drewrypope)",
        ok: true,
        configured: true,
        fetchedAt: Date.parse(TICK),
        credentialId: "cred_main",
        credentialLabel: "OpenAI Main (drewrypope)",
        usage: { windows: { weekly: win(100, 604_800) } },
      },
      TICK,
    );
    const { records } = recordsFromEntries([alt, mainDedicated], new Map(), "quota-gatherer");
    const byKey = new Map(records.map((r) => [`${r.serviceId}|${r.metricId}`, r.value]));
    expect(byKey.get("_openai.alt|5h")).toBe(42);
    expect(byKey.has("_openai.main|5h")).toBe(false);
    expect(byKey.get("_openai.main|weekly")).toBe(100);
    expect(byKey.get("_openai.alt|weekly")).toBe(65);
  });
});
