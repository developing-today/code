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
