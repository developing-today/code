// #1163: Claude token adapter — the claude.ai usage-API mapping on
// the #1153 framework, plus the expired-session fixture: an expired
// session is session-expired / re-login with the previous entry
// retained, and can never become a 0% row. Credential handling is
// CredentialRef-only throughout: values move only inside the
// injected fetch's Cookie header.
import { describe, expect, test } from "bun:test";
import type { QuotaCurrentEntry } from "../src/model.ts";
import {
  CLAUDE_ORGANIZATIONS_URL,
  claudeUsageUrl,
  collectClaudeAdapter,
  claudeCredentialRefFromConfig,
  entryFromClaudeReading,
  fetchClaudePayload,
  gatherClaude,
  organizationUuidOf,
  parseClaudeUsagePayload,
  type ClaudeFetch,
} from "../src/claude-adapter.ts";
import { AdapterError, type PreviousValue } from "../src/token-adapter.ts";

const NOW = new Date("2026-11-02T12:00:00Z");

// The claude.ai usage-API shape the /usage surface serves:
// per-window utilization percent + ISO reset instant.
const DOCUMENTED = {
  five_hour: { utilization: 43, resets_at: "2026-11-02T17:00:00Z" },
  seven_day: { utilization: 17, resets_at: "2026-11-08T12:00:00Z" },
  seven_day_sonnet: { utilization: 9, resets_at: "2026-11-08T12:00:00Z" },
};

const PREVIOUS: PreviousValue<QuotaCurrentEntry> = {
  value: {
    providerId: "claude",
    providerName: "Claude",
    serviceId: "anthropic:claude-pro",
    state: "ok",
    measuredAt: "2026-11-02T11:45:00Z",
    source: "token-adapter/claude",
    windows: [
      {
        name: "5h",
        value: 42,
        pctOfLimit: 42,
        valueLabel: null,
        windowSeconds: 18000,
        resetsAt: null,
        measuredAt: "2026-11-02T11:45:00Z",
        source: "token-adapter/claude",
        state: "ok",
      },
    ],
  },
  measuredAt: "2026-11-02T11:45:00Z",
};

describe("parseClaudeUsagePayload", () => {
  test("fresh windows parse: 5h + weekly with resets; unknown windows skipped", () => {
    const reading = parseClaudeUsagePayload(DOCUMENTED);
    expect(reading.windows.map((w) => w.name)).toEqual(["5h", "weekly"]);
    expect(reading.windows[0]).toMatchObject({ usedPct: 43, windowSeconds: 18000, resetsAt: "2026-11-02T17:00:00.000Z" });
    expect(reading.windows[1]).toMatchObject({ usedPct: 17, windowSeconds: 604800 });
  });

  test("genuine zero usage is a reading, not drift", () => {
    const reading = parseClaudeUsagePayload({ five_hour: { utilization: 0, resets_at: null } });
    expect(reading.windows).toHaveLength(1);
    expect(reading.windows[0]!.usedPct).toBe(0);
  });

  test("utilization clamps to 0..100 and plain-decimal strings parse", () => {
    const reading = parseClaudeUsagePayload({ seven_day: { utilization: "121.5" } });
    expect(reading.windows[0]!.usedPct).toBe(100);
  });

  test("shape drift fails closed as unparseable", () => {
    for (const bad of [null, [], {}, { five_hour: { resets_at: "x" } }, { five_hour: { utilization: "0x10" } }]) {
      expect(() => parseClaudeUsagePayload(bad)).toThrow(AdapterError);
    }
  });
});

describe("entryFromClaudeReading", () => {
  test("rows are sourced from the adapter as anthropic:claude-pro", () => {
    const entry = entryFromClaudeReading(parseClaudeUsagePayload(DOCUMENTED), NOW.toISOString());
    expect(entry.serviceId).toBe("anthropic:claude-pro");
    expect(entry.source).toBe("token-adapter/claude");
    expect(entry.state).toBe("ok");
    expect(entry.windows.map((w) => w.name)).toEqual(["5h", "weekly"]);
  });
});

describe("collectClaudeAdapter", () => {
  test("expired session: session-expired + needsRelogin, previous retained, never zeros", async () => {
    const outcome = await collectClaudeAdapter({
      now: NOW,
      previous: PREVIOUS,
      fetchPayload: async () => ({ httpStatus: 401, payload: null }),
    });
    expect(outcome.status).toBe("session-expired");
    expect(outcome.needsRelogin).toBe(true);
    expect(outcome.stale).toBe(true);
    expect(outcome.value).toEqual(PREVIOUS.value);
  });

  test("unparseable payload retains previous", async () => {
    const outcome = await collectClaudeAdapter({
      now: NOW,
      previous: PREVIOUS,
      fetchPayload: async () => ({ httpStatus: 200, payload: { drifted: true } }),
    });
    expect(outcome.status).toBe("unparseable");
    expect(outcome.value).toEqual(PREVIOUS.value);
  });
});

describe("fetchClaudePayload", () => {
  const fakeFetch =
    (routes: Record<string, { status: number; body: unknown }>): ClaudeFetch =>
    async (url) => {
      const route = routes[url];
      if (!route) throw new Error(`unexpected url ${url}`);
      return { status: route.status, text: async () => JSON.stringify(route.body) };
    };

  test("resolves the organization then reads usage, credential only in the Cookie header", async () => {
    const seen: { url: string; cookie?: string }[] = [];
    const base = fakeFetch({
      [CLAUDE_ORGANIZATIONS_URL]: { status: 200, body: [{ uuid: "org-1" }] },
      [claudeUsageUrl("org-1")]: { status: 200, body: DOCUMENTED },
    });
    const fetchImpl: ClaudeFetch = async (url, init) => {
      seen.push({ url, cookie: init.headers.Cookie });
      return base(url, init);
    };
    const res = await fetchClaudePayload({ sessionKey: "sk-test-value", fetchImpl });
    expect(res.httpStatus).toBe(200);
    expect(parseClaudeUsagePayload(res.payload).windows).toHaveLength(2);
    expect(seen.map((s) => s.url)).toEqual([CLAUDE_ORGANIZATIONS_URL, claudeUsageUrl("org-1")]);
    expect(seen.every((s) => s.cookie === "sessionKey=sk-test-value")).toBe(true);
  });

  test("an expired session at the organizations step surfaces its status", async () => {
    const res = await fetchClaudePayload({
      sessionKey: "sk-test-value",
      fetchImpl: fakeFetch({ [CLAUDE_ORGANIZATIONS_URL]: { status: 403, body: {} } }),
    });
    expect(res.httpStatus).toBe(403);
  });
});

describe("organizationUuidOf / credential config / gatherClaude", () => {
  test("first uuid-bearing organization wins; drift is unparseable", () => {
    expect(organizationUuidOf([{ name: "x" }, { uuid: "org-9" }])).toBe("org-9");
    expect(() => organizationUuidOf([{ name: "x" }])).toThrow(AdapterError);
    expect(() => organizationUuidOf({})).toThrow(AdapterError);
  });

  test("credential config is references only", () => {
    expect(claudeCredentialRefFromConfig(undefined)).toBeNull();
    expect(claudeCredentialRefFromConfig("bitwarden:claude-session")).toEqual({ kind: "bitwarden", name: "claude-session" });
    expect(() => claudeCredentialRefFromConfig("sk-raw-value")).toThrow(AdapterError);
  });

  test("unresolvable credential is a fetch failure naming the ref, never a zero", async () => {
    const outcome = await gatherClaude({
      now: NOW,
      previous: PREVIOUS,
      credentialRef: { kind: "bitwarden", name: "claude-session" },
      resolveCredentialValue: async () => null,
    });
    expect(outcome.status).toBe("fetch-failed");
    expect(outcome.detail).toContain("bitwarden:claude-session");
    expect(outcome.value).toEqual(PREVIOUS.value);
  });
});

describe("parseClaudeUsagePayload / collectClaudeAdapter properties (#1163, Codex P1 on PR #8480)", () => {
  // Deterministic seeded sweep (fixed seed: failures reproduce exactly),
  // following the seeded property precedent in token-adapter.test.ts.
  // Oracles are the parser invariants:
  //  (1) every accepted window's usedPct is finite, within 0..100, and
  //      equal to the raw utilization clamped to 0..100;
  //  (2) malformed utilization never becomes a successful zero reading:
  //      a payload with no usable window throws AdapterError;
  //  (3) a failed collection retains the previous entry verbatim -
  //      never a fabricated zero row.
  let state = 0x1163;
  const rand = () => {
    state |= 0;
    state = (state + 0x6d2b79f5) | 0;
    let x = Math.imul(state ^ (state >>> 15), 1 | state);
    x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x;
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
  };
  const PLAIN_DECIMAL = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)$/;
  const clamp = (n: number) => Math.min(100, Math.max(0, n));
  // Returns the raw utilization when the parser must accept it, else null.
  const acceptedRaw = (value: unknown): number | null => {
    if (typeof value === "number") return Number.isFinite(value) ? value : null;
    if (typeof value === "string" && PLAIN_DECIMAL.test(value)) {
      const n = Number(value);
      return Number.isFinite(n) ? n : null;
    }
    return null;
  };
  const genUtilization = (): unknown => {
    const pool = rand();
    if (pool < 0.3) return rand() * 200 - 50; // finite number, in/out of range
    if (pool < 0.45) return String(Math.round((rand() * 200 - 50) * 100) / 100); // plain-decimal string
    if (pool < 0.55) return [NaN, Infinity, -Infinity][Math.floor(rand() * 3)];
    if (pool < 0.7) return ["0x10", "1e3", "", "abc", "12%"][Math.floor(rand() * 5)]; // non-plain-decimal strings
    if (pool < 0.85) return [null, true, {}, []][Math.floor(rand() * 4)]; // wrong types
    return undefined; // missing
  };
  const FIELDS: [string, string][] = [
    ["five_hour", "5h"],
    ["seven_day", "weekly"],
  ];

  test("property: accepted percentages are finite, in 0..100, and equal the clamped raw value", () => {
    for (let iter = 0; iter < 300; iter++) {
      const payload: Record<string, unknown> = {};
      const expected = new Map<string, number>();
      for (const [field, name] of FIELDS) {
        if (rand() < 0.15) continue; // field absent
        const utilization = genUtilization();
        payload[field] = { utilization, resets_at: "2026-11-02T17:00:00Z" };
        const raw = acceptedRaw(utilization);
        if (raw !== null) expected.set(name, clamp(raw));
      }
      if (expected.size === 0) {
        // Invariant (2): nothing usable -> fail closed, never a zero reading.
        expect(() => parseClaudeUsagePayload(payload)).toThrow(AdapterError);
        continue;
      }
      const reading = parseClaudeUsagePayload(payload);
      expect(reading.windows.map((w) => w.name).sort()).toEqual([...expected.keys()].sort());
      for (const window of reading.windows) {
        expect(Number.isFinite(window.usedPct)).toBe(true);
        expect(window.usedPct).toBeGreaterThanOrEqual(0);
        expect(window.usedPct).toBeLessThanOrEqual(100);
        expect(window.usedPct).toBe(expected.get(window.name)!);
      }
    }
  });

  test("property: failed collection retains the previous entry, never zeros", async () => {
    const failureModes = [
      () => ({ httpStatus: 401, payload: null }),
      () => ({ httpStatus: 403, payload: null }),
      () => ({ httpStatus: 500, payload: null }),
      () => ({ httpStatus: 200, payload: { drifted: true } }),
      () => ({ httpStatus: 200, payload: { five_hour: { utilization: "0x10" } } }),
      () => {
        throw new Error("network down");
      },
    ];
    for (let iter = 0; iter < 120; iter++) {
      const fetchPayload = failureModes[Math.floor(rand() * failureModes.length)]!;
      const outcome = await collectClaudeAdapter({ now: NOW, previous: PREVIOUS, fetchPayload });
      expect(outcome.status).not.toBe("ok");
      expect(outcome.value).toEqual(PREVIOUS.value);
      expect(outcome.value?.measuredAt).toBe(PREVIOUS.measuredAt);
      expect(outcome.value?.windows.every((w) => w.pctOfLimit !== 0)).toBe(true);
    }
  });
});
