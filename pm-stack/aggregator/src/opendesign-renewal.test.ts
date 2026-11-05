import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  horizonHours,
  pickOpenDesignSessionCookie,
  renewOpenDesignSession,
  type OpenDesignCredentialLike,
} from "./opendesign-renewal.ts";

const NOW = new Date("2026-11-05T02:00:00.000Z");
const statePath = () => join(mkdtempSync(join(tmpdir(), "od-renewal-")), "state.json");
const stored: OpenDesignCredentialLike = { cookie: "better-auth.session_token=stored-token", workspaceId: "ws1" };

const baseDeps = (overrides: Record<string, unknown> = {}) => ({
  readCredential: () => stored,
  writeCredential: () => undefined,
  validateCredential: async () => undefined,
  statePath: statePath(),
  now: () => NOW,
  ...overrides,
});

describe("pickOpenDesignSessionCookie", () => {
  test("prefers the __Secure- cookie on the app domain", () => {
    const found = pickOpenDesignSessionCookie([
      { name: "better-auth.session_token", value: "a", domain: ".open-design.ai" },
      { name: "__Secure-better-auth.session_token", value: "b", domain: "app.open-design.ai" },
      { name: "other", value: "c", domain: ".open-design.ai" },
    ]);
    expect(found?.value).toBe("b");
  });
  test("ignores other domains and empty values", () => {
    expect(pickOpenDesignSessionCookie([{ name: "better-auth.session_token", value: "a", domain: ".example.com" }])).toBeNull();
    expect(pickOpenDesignSessionCookie([{ name: "better-auth.session_token", value: "", domain: ".open-design.ai" }])).toBeNull();
  });
});

describe("horizonHours", () => {
  test("computes whole/fractional hours and rejects garbage", () => {
    expect(horizonHours("2026-11-12T02:00:00.000Z", NOW)).toBe(168);
    expect(horizonHours("2026-11-05T03:30:00.000Z", NOW)).toBe(1.5);
    expect(horizonHours(null, NOW)).toBeNull();
    expect(horizonHours("not-a-date", NOW)).toBeNull();
  });
});

describe("renewOpenDesignSession", () => {
  test("a healthy sliding session is valid and untouched", async () => {
    let writes = 0;
    const state = await renewOpenDesignSession(
      baseDeps({
        writeCredential: () => writes++,
        fetchSession: async () => ({ expiresAt: "2026-11-12T02:00:00.000Z" }),
      }) as never,
    );
    expect(state.status).toBe("valid");
    expect(state.horizonHours).toBe(168);
    expect(state.capture.attempted).toBe(false);
    expect(writes).toBe(0);
  });

  test("a dead stored session is renewed from the browser cookie, validated before storing", async () => {
    // Holder object, not bare lets: TS narrows closure-assigned lets to
    // their initializer at the expect() sites below (TS2339 on never).
    const captured: {
      written: OpenDesignCredentialLike | null;
      validated: OpenDesignCredentialLike | null;
    } = { written: null, validated: null };
    const state = await renewOpenDesignSession(
      baseDeps({
        readCredential: () => captured.written ?? stored,
        writeCredential: (value: OpenDesignCredentialLike) => {
          captured.written = value;
        },
        validateCredential: async (candidate: OpenDesignCredentialLike) => {
          captured.validated = candidate;
        },
        fetchSession: async (cred: OpenDesignCredentialLike) =>
          cred.cookie.includes("fresh-token") ? { expiresAt: "2026-11-12T02:00:00.000Z" } : null,
        getBrowserCookies: async () => [
          { name: "__Secure-better-auth.session_token", value: "fresh-token", domain: ".open-design.ai" },
        ],
      }) as never,
    );
    expect(state.status).toBe("renewed");
    expect(captured.validated?.cookie).toContain("fresh-token");
    expect(captured.written?.cookie).toContain("fresh-token");
    expect(captured.written?.workspaceId).toBe("ws1");
    expect(state.horizonHours).toBe(168);
  });

  test("a candidate that fails validation never overwrites the stored credential, and the alert fires", async () => {
    let writes = 0;
    const state = await renewOpenDesignSession(
      baseDeps({
        writeCredential: () => writes++,
        validateCredential: async () => {
          throw new Error("usage API said no");
        },
        fetchSession: async () => null,
        getBrowserCookies: async () => [
          { name: "__Secure-better-auth.session_token", value: "stale-token", domain: ".open-design.ai" },
        ],
      }) as never,
    );
    expect(state.status).toBe("attention");
    expect(state.capture.result).toBe("validation-failed");
    expect(state.alert).toContain("no longer accepted");
    expect(writes).toBe(0);
  });

  test("no browser session + dead stored session = attention with a sign-in alert, days ahead of zero", async () => {
    const state = await renewOpenDesignSession(
      baseDeps({
        fetchSession: async () => null,
        getBrowserCookies: async () => [],
        refreshBrowserTab: async () => undefined,
      }) as never,
    );
    expect(state.status).toBe("attention");
    expect(state.capture.result).toBe("no-browser-session");
    expect(state.alert).toContain("sign-in");
    expect(state.alertSince).toBe(NOW.toISOString());
  });

  test("no stored credential is a configuration state, not an alert", async () => {
    const state = await renewOpenDesignSession(baseDeps({ readCredential: () => null }) as never);
    expect(state.status).toBe("not-configured");
    expect(state.alert).toBeNull();
  });
});
