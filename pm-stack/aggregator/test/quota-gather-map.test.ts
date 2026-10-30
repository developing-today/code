import { describe, expect, test } from "bun:test";
import {
  entryFromFileExport,
  entryFromTurso,
  entryFromResult,
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
});

describe("entryFromResult", () => {
  test("maps windows onto the quota-current shape", () => {
    const entry = entryFromResult(OK_RESULT, "2026-10-27T05:00:00.000Z");
    expect(entry.serviceId).toBe("cline-pass:cline-pass");
    expect(entry.state).toBe("ok");
    expect(entry.measuredAt).toBe(new Date(1793075143879).toISOString());
    const weekly = entry.windows.find((w) => w.name === "weekly");
    expect(weekly?.pctOfLimit).toBe(29);
    expect(weekly?.value).toBe(29);
    expect(weekly?.resetsAt).toBe("2026-10-31T09:20:27.299Z");
    const balance = entry.windows.find((w) => w.name === "credits_balance");
    expect(balance?.pctOfLimit).toBeNull();
    expect(balance?.valueLabel).toBe("12.50");
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

describe("recordsFromEntries", () => {
  test("a pct record per changed window; label-only windows yield none", () => {
    const entry = entryFromResult(OK_RESULT, "2026-10-27T05:00:00.000Z");
    const first = recordsFromEntries([entry], new Map(), "quota-gatherer");
    expect(first.records).toHaveLength(1);
    expect(first.records[0]).toMatchObject({
      serviceId: "cline-pass:cline-pass",
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
      serviceId: "deepseek:api",
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
      const { records, appended } = recordsFromEntries([cred("cred_a", 10, tick), cred("cred_b", 20, tick)], state, "quota-gatherer");
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
    expect(recordsFromEntries([cred("cred_a", 10, T3), cred("cred_b", 25, T3)], settled, "quota-gatherer").records).toHaveLength(0);
  });

  test("non-max credential moving 10->15 with the max unchanged appends zero", () => {
    const first = recordsFromEntries([cred("cred_a", 10, T1), cred("cred_b", 20, T1)], new Map(), "quota-gatherer");
    const state = new Map(first.appended);
    const { records } = recordsFromEntries([cred("cred_a", 15, T2), cred("cred_b", 20, T2)], state, "quota-gatherer");
    expect(records).toHaveLength(0);
  });

  test("one recordsFromEntries call never emits two records for the same serviceId|window key", () => {
    const { records } = recordsFromEntries([cred("cred_a", 10, T1), cred("cred_b", 20, T1)], new Map(), "quota-gatherer");
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
    expect(entry.serviceId).toBe("meta:high-usage");
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
    expect(serviceIdFor("codex")).toBe("openai:codex");
    expect(serviceIdFor("deepseek")).toBe("deepseek:api");
    expect(serviceIdFor("openai.alternate")).toBe("openai.alternate");
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
    subscription: { plan: "starter", name: "starter", current_billing_period_start: "2026-10-01T04:00:00+00:00", current_billing_period_end: "2026-11-01T04:00:00+00:00" },
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
});
