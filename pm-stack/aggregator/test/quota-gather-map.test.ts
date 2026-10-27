import { describe, expect, test } from "bun:test";
import {
  entryFromFileExport,
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
