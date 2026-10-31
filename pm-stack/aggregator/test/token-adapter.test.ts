// #1153: browser-token adapter framework core — credential contract,
// fail-closed parsing (drift retains the previous value, never a
// zero), session expiry distinct from genuine zero usage, and the
// file-export source expressed as the first adapter.
import { describe, expect, test } from "bun:test";
import type { QuotaCurrentEntry } from "../src/model.ts";
import {
  AdapterError,
  classifyHttpProbe,
  collectAdapter,
  collectFileExportAdapter,
  credentialRefLabel,
  validateCredentialRef,
  type PreviousValue,
} from "../src/token-adapter.ts";

const NOW = new Date("2026-10-29T05:00:00Z");

describe("credential contract", () => {
  test("bitwarden and 0600-file references validate and label without values", () => {
    expect(credentialRefLabel(validateCredentialRef({ kind: "bitwarden", name: "meta-session" }))).toBe(
      "bitwarden:meta-session",
    );
    expect(
      credentialRefLabel(validateCredentialRef({ kind: "file", path: "/home/user/.local/state/x/token", mode: 0o600 })),
    ).toBe("file:/home/user/.local/state/x/token (0600)");
  });

  test("a reference carrying a credential value is rejected", () => {
    expect(() => validateCredentialRef({ kind: "bitwarden", name: "x", token: "abc" })).toThrow(/references only/);
    expect(() => validateCredentialRef({ kind: "file", path: "/tmp/t", mode: 0o600, cookie: "a=b" })).toThrow(
      /references only/,
    );
  });

  test("credential value keys are checked as own properties for both reference kinds", () => {
    const keys = ["token", "cookie", "value", "secret", "password", "apiKey", "api_key", "authorization"];
    const refs = [{ kind: "bitwarden", name: "x" }, { kind: "file", path: "/tmp/t", mode: 0o600 }];
    for (const ref of refs) {
      for (const key of keys) {
        const inherited = Object.assign(Object.create({ [key]: "sensitive" }), ref);
        expect(validateCredentialRef(inherited)).toEqual(ref);
        expect(() => validateCredentialRef({ ...ref, [key]: undefined })).toThrow(/references only/);
      }
    }
  });

  test("a file reference without the 0600 contract is rejected", () => {
    expect(() => validateCredentialRef({ kind: "file", path: "/tmp/t", mode: 0o644 })).toThrow(/0600/);
    expect(() => validateCredentialRef({ kind: "vault", name: "x" })).toThrow(/unknown kind/);
  });
});

describe("session health classification", () => {
  test("401/403 and auth bounces are expired; 200 is ok; the rest is unknown", () => {
    expect(classifyHttpProbe({ httpStatus: 401 })).toBe("expired");
    expect(classifyHttpProbe({ httpStatus: 403 })).toBe("expired");
    expect(classifyHttpProbe({ httpStatus: 302, location: "https://example.com/login?next=/dash" })).toBe("expired");
    expect(classifyHttpProbe({ httpStatus: 200 })).toBe("ok");
    expect(classifyHttpProbe({ httpStatus: 500 })).toBe("unknown");
    expect(classifyHttpProbe({ httpStatus: 302, location: "https://example.com/dashboard" })).toBe("unknown");
  });
});

interface Reading {
  usedPct: number;
}
const PREVIOUS: PreviousValue<Reading> = { value: { usedPct: 42 }, measuredAt: "2026-10-29T04:30:00Z" };
const parseReading = (payload: unknown): Reading => {
  const used = (payload as { usedPct?: unknown })?.usedPct;
  if (typeof used !== "number" || !Number.isFinite(used))
    throw new AdapterError("unparseable", "drift: usedPct missing");
  return { usedPct: used };
};

describe("collectAdapter (fail-closed)", () => {
  test("a valid payload yields the fresh value", async () => {
    const out = await collectAdapter({
      now: NOW,
      previous: PREVIOUS,
      fetchPayload: async () => ({ httpStatus: 200, payload: { usedPct: 7 } }),
      parse: parseReading,
    });
    expect(out).toMatchObject({ status: "ok", value: { usedPct: 7 }, stale: false, needsRelogin: false });
  });

  test("a genuine zero-usage reading is ok and is not an expiry", async () => {
    const out = await collectAdapter({
      now: NOW,
      previous: PREVIOUS,
      fetchPayload: async () => ({ httpStatus: 200, payload: { usedPct: 0 } }),
      parse: parseReading,
    });
    expect(out.status).toBe("ok");
    expect(out.value).toEqual({ usedPct: 0 });
    expect(out.needsRelogin).toBe(false);
  });

  test("shape drift is unparseable: the previous value is retained and ages, never zeroed", async () => {
    const out = await collectAdapter({
      now: NOW,
      previous: PREVIOUS,
      fetchPayload: async () => ({ httpStatus: 200, payload: { renamed: 1 } }),
      parse: parseReading,
    });
    expect(out.status).toBe("unparseable");
    expect(out.value).toEqual({ usedPct: 42 });
    expect(out.stale).toBe(true);
    expect(out.ageSeconds).toBe(1800);
  });

  test("an expired session retains the previous value and raises re-login, distinct from zero usage", async () => {
    const out = await collectAdapter({
      now: NOW,
      previous: PREVIOUS,
      fetchPayload: async () => ({ httpStatus: 401, payload: null }),
      parse: parseReading,
    });
    expect(out.status).toBe("session-expired");
    expect(out.needsRelogin).toBe(true);
    expect(out.value).toEqual({ usedPct: 42 });
    expect(out.session).toBe("expired");
  });

  test("a transport failure with no previous value yields null, never a fabricated reading", async () => {
    const out = await collectAdapter({
      now: NOW,
      previous: null,
      fetchPayload: async () => {
        throw new Error("connect ECONNREFUSED");
      },
      parse: parseReading,
    });
    expect(out.status).toBe("fetch-failed");
    expect(out.value).toBeNull();
    expect(out.stale).toBe(false);
  });
});

const PREV_ENTRY: PreviousValue<QuotaCurrentEntry> = {
  value: {
    providerId: "meta",
    providerName: "Meta",
    serviceId: "meta:high-usage",
    state: "ok",
    measuredAt: "2026-10-29T04:30:00Z",
    source: "file-export/meta-laptop",
    windows: [
      {
        name: "5h",
        value: 10,
        pctOfLimit: 10,
        valueLabel: null,
        windowSeconds: 18000,
        resetsAt: null,
        measuredAt: "2026-10-29T04:30:00Z",
        source: "file-export/meta-laptop",
        state: "ok",
      },
    ],
  },
  measuredAt: "2026-10-29T04:30:00Z",
};

describe("file-export adapter (the framework's first adapter)", () => {
  test("a healthy document maps to a quota-current entry labelled file-export", () => {
    const out = collectFileExportAdapter({
      now: NOW,
      previous: null,
      fileMtimeIso: "2026-10-29T04:59:00Z",
      stale: false,
      doc: {
        service: "meta",
        source: "file-export/meta-laptop",
        status: "ok",
        measuredAt: "2026-10-29T04:59:00Z",
        tier: "Muse Code High Usage",
        windows: [{ name: "5h", usedPct: 12.5, resetAt: "2026-10-29T09:00:00Z" }],
        freeCreditsUsd: 100,
        paygBalanceUsd: 0,
      },
    });
    expect(out.status).toBe("ok");
    expect(out.value?.providerId).toBe("meta");
    expect(out.value?.serviceId).toBe("meta:high-usage");
    expect(out.value?.source).toBe("file-export/meta-laptop");
    expect(out.value?.windows.some((w) => w.name === "5h" && w.pctOfLimit === 12.5)).toBe(true);
  });

  test("a session-expired document raises re-login and retains the previous entry", () => {
    const out = collectFileExportAdapter({
      now: NOW,
      previous: PREV_ENTRY,
      fileMtimeIso: "2026-10-29T04:59:00Z",
      stale: false,
      doc: { service: "meta", source: "file-export/meta-laptop", status: "session-expired", measuredAt: null },
    });
    expect(out.status).toBe("session-expired");
    expect(out.needsRelogin).toBe(true);
    expect(out.value?.serviceId).toBe("meta:high-usage");
    expect(out.stale).toBe(true);
  });

  test("a drifted success document (window without usedPct) is unparseable, previous retained", () => {
    const out = collectFileExportAdapter({
      now: NOW,
      previous: PREV_ENTRY,
      fileMtimeIso: "2026-10-29T04:59:00Z",
      stale: false,
      doc: {
        service: "meta",
        source: "file-export/meta-laptop",
        status: "ok",
        measuredAt: "2026-10-29T04:59:00Z",
        windows: [{ name: "5h" }],
      },
    });
    expect(out.status).toBe("unparseable");
    expect(out.value?.measuredAt).toBe("2026-10-29T04:30:00Z");
  });

  test("a stale producer is transport-unavailable; a missing document is fetch-failed with null", () => {
    const stale = collectFileExportAdapter({
      now: NOW,
      previous: PREV_ENTRY,
      fileMtimeIso: "2026-10-29T03:00:00Z",
      stale: true,
      doc: { status: "ok" },
    });
    expect(stale.status).toBe("transport-unavailable");
    expect(stale.value?.serviceId).toBe("meta:high-usage");
    const missing = collectFileExportAdapter({ now: NOW, previous: null, fileMtimeIso: null, stale: false, doc: null });
    expect(missing.status).toBe("fetch-failed");
    expect(missing.value).toBeNull();
  });
});
