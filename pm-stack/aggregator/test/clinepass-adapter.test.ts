// #1159: ClinePass token adapter — the OpenChamber provider's
// dashboard mapping ported onto the #1153 framework, plus the
// expired-token fixture: an expired session is session-expired /
// re-login with the previous entry retained, and can never become
// a 0% row. Credential handling is CredentialRef-only throughout:
// values move only inside the injected fetch's Authorization header.
import { describe, expect, test } from "bun:test";
import type { QuotaCurrentEntry } from "../src/model.ts";
import {
  CLINEPASS_USAGE_URL,
  collectClinePassAdapter,
  credentialRefFromConfig,
  entryFromClinePassReading,
  fetchClinePassPayload,
  gatherClinePass,
  parseClinePassPayload,
  type ClinePassFetch,
} from "../src/clinepass-adapter.ts";
import { AdapterError, type PreviousValue } from "../src/token-adapter.ts";

const NOW = new Date("2026-11-02T12:00:00Z");

// Response shape verified against the live ClinePass API by the
// OpenChamber provider's contributor (its cline-pass.test.js).
const DOCUMENTED = {
  data: {
    limits: [
      { type: "five_hour", percentUsed: 43, resetsAt: "2026-09-08T17:00:44.598174595Z" },
      { type: "weekly", percentUsed: 17 },
      { type: "monthly", percentUsed: 8 },
    ],
  },
  success: true,
};

const PREVIOUS: PreviousValue<QuotaCurrentEntry> = {
  value: {
    providerId: "cline-pass",
    providerName: "ClinePass",
    serviceId: "cline-pass",
    state: "ok",
    measuredAt: "2026-11-02T11:45:00Z",
    source: "token-adapter/cline-pass",
    windows: [
      {
        name: "monthly",
        value: 42,
        pctOfLimit: 42,
        valueLabel: null,
        windowSeconds: null,
        resetsAt: null,
        measuredAt: "2026-11-02T11:45:00Z",
        source: "token-adapter/cline-pass",
        state: "ok",
      },
    ],
  },
  measuredAt: "2026-11-02T11:45:00Z",
};

describe("parseClinePassPayload (ported OpenChamber mapping)", () => {
  test("maps the documented windows in order with durations and resets", () => {
    const reading = parseClinePassPayload(DOCUMENTED);
    expect(reading.windows.map((w) => w.name)).toEqual(["5h", "weekly", "monthly"]);
    expect(reading.windows[0]).toEqual({
      name: "5h",
      usedPct: 43,
      windowSeconds: 18000,
      resetsAt: "2026-09-08T17:00:44.598Z",
    });
    expect(reading.windows[1].windowSeconds).toBe(604800);
    expect(reading.windows[2].windowSeconds).toBeNull();
  });

  test("accepts numeric-string percentages and clamps out-of-range values", () => {
    const reading = parseClinePassPayload({
      data: {
        limits: [
          { type: "weekly", percentUsed: "51" },
          { type: "monthly", percentUsed: 150 },
          { type: "five_hour", percentUsed: -20 },
        ],
      },
    });
    expect(reading.windows.map((w) => w.usedPct)).toEqual([51, 100, 0]);
  });

  test("skips malformed windows and unknown types, keeping valid siblings", () => {
    const reading = parseClinePassPayload({
      data: {
        limits: [
          { type: "quarterly", percentUsed: 5 },
          { type: "__proto__", percentUsed: 5 },
          { type: "weekly", percentUsed: "" },
          { type: "weekly", percentUsed: "NaN" },
          null,
          { type: "monthly", percentUsed: 8 },
        ],
      },
    });
    expect(reading.windows).toEqual([{ name: "monthly", usedPct: 8, windowSeconds: null, resetsAt: null }]);
  });

  test("empty or drifted payloads are unparseable, never a zero reading", () => {
    for (const payload of [
      null,
      [],
      {},
      { data: null },
      { data: { limits: [] } },
      { data: { limits: [{ type: "weekly" }] } },
    ]) {
      let caught: unknown = null;
      try {
        parseClinePassPayload(payload);
      } catch (err) {
        caught = err;
      }
      expect(caught).toBeInstanceOf(AdapterError);
      expect((caught as AdapterError).status).toBe("unparseable");
    }
  });

  test("entry mapping lands on the cline-pass service id with pct windows", () => {
    const entry = entryFromClinePassReading(parseClinePassPayload(DOCUMENTED), NOW.toISOString());
    expect(entry.providerId).toBe("cline-pass");
    expect(entry.serviceId).toBe("cline-pass");
    expect(entry.state).toBe("ok");
    expect(entry.windows.map((w) => [w.name, w.pctOfLimit])).toEqual([
      ["5h", 43],
      ["weekly", 17],
      ["monthly", 8],
    ]);
  });
});

describe("collectClinePassAdapter (framework semantics)", () => {
  test("a valid payload yields a fresh quota-current entry", async () => {
    const out = await collectClinePassAdapter({
      now: NOW,
      previous: PREVIOUS,
      fetchPayload: async () => ({ httpStatus: 200, payload: DOCUMENTED }),
    });
    expect(out.status).toBe("ok");
    expect(out.value?.windows[0].pctOfLimit).toBe(43);
    expect(out.stale).toBe(false);
    expect(out.needsRelogin).toBe(false);
  });

  test("EXPIRED-TOKEN FIXTURE: 401 is session-expired + re-login, previous retained, never 0%", async () => {
    const out = await collectClinePassAdapter({
      now: NOW,
      previous: PREVIOUS,
      fetchPayload: async () => ({ httpStatus: 401, payload: null }),
    });
    expect(out.status).toBe("session-expired");
    expect(out.needsRelogin).toBe(true);
    expect(out.session).toBe("expired");
    expect(out.stale).toBe(true);
    expect(out.ageSeconds).toBe(900);
    // The retained value is the previous entry verbatim — its 42%
    // monthly window survives; no window reads 0 and none is invented.
    expect(out.value).toEqual(PREVIOUS.value);
    expect(out.value?.windows.every((w) => w.pctOfLimit !== 0)).toBe(true);
  });

  test("an expired token with no previous value yields null, not a zero entry", async () => {
    const out = await collectClinePassAdapter({
      now: NOW,
      previous: null,
      fetchPayload: async () => ({ httpStatus: 403, payload: null }),
    });
    expect(out.status).toBe("session-expired");
    expect(out.needsRelogin).toBe(true);
    expect(out.value).toBeNull();
  });

  test("a genuine zero-usage reading is ok and is not an expiry", async () => {
    const out = await collectClinePassAdapter({
      now: NOW,
      previous: PREVIOUS,
      fetchPayload: async () => ({
        httpStatus: 200,
        payload: { data: { limits: [{ type: "monthly", percentUsed: 0 }] } },
      }),
    });
    expect(out.status).toBe("ok");
    expect(out.needsRelogin).toBe(false);
    expect(out.value?.windows[0].pctOfLimit).toBe(0);
  });

  test("shape drift is unparseable with the previous entry retained", async () => {
    const out = await collectClinePassAdapter({
      now: NOW,
      previous: PREVIOUS,
      fetchPayload: async () => ({ httpStatus: 200, payload: { renamed: true } }),
    });
    expect(out.status).toBe("unparseable");
    expect(out.value).toEqual(PREVIOUS.value);
  });
});

describe("fetchClinePassPayload (credential hygiene)", () => {
  test("sends the token only as the Authorization header, to the provider URL", async () => {
    let seenUrl = "";
    let seenAuth: string | undefined;
    const fetchImpl: ClinePassFetch = async (url, init) => {
      seenUrl = url;
      seenAuth = init.headers.Authorization;
      return { status: 200, text: async () => JSON.stringify(DOCUMENTED) };
    };
    const res = await fetchClinePassPayload({ token: "test-token", fetchImpl });
    expect(seenUrl).toBe(CLINEPASS_USAGE_URL);
    expect(seenAuth).toBe("Bearer test-token");
    expect(res.httpStatus).toBe(200);
    expect(JSON.stringify(res.payload)).not.toContain("test-token");
  });

  test("an unreadable body on a 200 leaves payload null so parse fails closed", async () => {
    const fetchImpl: ClinePassFetch = async () => ({ status: 200, text: async () => "{" });
    const res = await fetchClinePassPayload({ token: "test-token", fetchImpl });
    expect(res.payload).toBeNull();
  });
});

describe("gatherClinePass (gatherer registration path)", () => {
  test("config parses references only; a bare value is rejected", () => {
    expect(credentialRefFromConfig(undefined)).toBeNull();
    expect(credentialRefFromConfig("bitwarden:clinepass-session")).toEqual({
      kind: "bitwarden",
      name: "clinepass-session",
    });
    expect(credentialRefFromConfig("file:/home/user/.local/state/clinepass/token")).toEqual({
      kind: "file",
      path: "/home/user/.local/state/clinepass/token",
      mode: 0o600,
    });
    expect(() => credentialRefFromConfig("sk-live-value")).toThrow(/reference, never a credential value/);
  });

  test("a tick with a fixture credential produces the ClinePass quota-current row", async () => {
    const out = await gatherClinePass({
      now: NOW,
      previous: null,
      credentialRef: { kind: "bitwarden", name: "clinepass-fixture" },
      resolveCredentialValue: async (ref) =>
        ref.kind === "bitwarden" && ref.name === "clinepass-fixture" ? "fixture-token" : null,
      fetchImpl: async () => ({ status: 200, text: async () => JSON.stringify(DOCUMENTED) }),
    });
    expect(out.status).toBe("ok");
    expect(out.value?.serviceId).toBe("cline-pass");
    expect(out.value?.windows.map((w) => w.name)).toEqual(["5h", "weekly", "monthly"]);
    expect(JSON.stringify(out)).not.toContain("fixture-token");
  });

  test("an expired fixture token on the gatherer path: session-expired, previous retained, never 0%", async () => {
    const out = await gatherClinePass({
      now: NOW,
      previous: PREVIOUS,
      credentialRef: { kind: "bitwarden", name: "clinepass-fixture" },
      resolveCredentialValue: async () => "expired-fixture-token",
      fetchImpl: async () => ({ status: 401, text: async () => "" }),
    });
    expect(out.status).toBe("session-expired");
    expect(out.needsRelogin).toBe(true);
    expect(out.value).toEqual(PREVIOUS.value);
    expect(JSON.stringify(out)).not.toContain("expired-fixture-token");
  });

  test("an unresolvable reference fails the collection without fetching and without zeros", async () => {
    let fetched = false;
    const out = await gatherClinePass({
      now: NOW,
      previous: null,
      credentialRef: { kind: "bitwarden", name: "clinepass-missing" },
      resolveCredentialValue: async () => null,
      fetchImpl: async () => {
        fetched = true;
        return { status: 200, text: async () => "{}" };
      },
    });
    expect(out.status).toBe("fetch-failed");
    expect(out.detail).toContain("bitwarden:clinepass-missing");
    expect(out.value).toBeNull();
    expect(fetched).toBe(false);
  });

  test("a value-bearing credential ref is rejected before any fetch", async () => {
    const out = await gatherClinePass({
      now: NOW,
      previous: null,
      credentialRef: { kind: "bitwarden", name: "x", token: "should-never-appear" } as never,
      resolveCredentialValue: async () => "unused",
      fetchImpl: async () => ({ status: 200, text: async () => "{}" }),
    });
    expect(out.status).toBe("fetch-failed");
    expect(out.detail).toContain("references only");
    expect(JSON.stringify(out)).not.toContain("should-never-appear");
  });
});
