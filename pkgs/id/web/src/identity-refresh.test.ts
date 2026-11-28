import { describe, expect, it } from "vitest";
import {
  identityNeedsRefresh,
  meResponseCarriesName,
  parseIdentityMeResponse,
  parseIdentityNameResponse,
  parseIdentityRegisterResponse,
} from "./identity-refresh";

const INTERVAL = 24 * 60 * 60 * 1000;
const NOW = 1_800_000_000_000;

describe("identityNeedsRefresh", () => {
  it("needs refresh when nothing is stored", () => {
    expect(identityNeedsRefresh(null, NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh("", NOW, INTERVAL)).toBe(true);
  });

  it("needs refresh for a non-numeric or non-finite stored value (#847)", () => {
    // Number("garbage") is NaN and NaN >= interval is false, so the old
    // inline check returned false forever for these values.
    expect(identityNeedsRefresh("garbage", NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh("12x", NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh("Infinity", NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh("NaN", NOW, INTERVAL)).toBe(true);
  });

  it("needs refresh for a timestamp in the future", () => {
    expect(identityNeedsRefresh(String(NOW + 1), NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh(String(NOW + INTERVAL * 365), NOW, INTERVAL)).toBe(true);
  });

  it("does not need refresh for a recent timestamp", () => {
    expect(identityNeedsRefresh(String(NOW), NOW, INTERVAL)).toBe(false);
    expect(identityNeedsRefresh(String(NOW - INTERVAL + 1), NOW, INTERVAL)).toBe(false);
  });

  it("needs refresh once the interval has elapsed", () => {
    expect(identityNeedsRefresh(String(NOW - INTERVAL), NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh(String(NOW - INTERVAL * 30), NOW, INTERVAL)).toBe(true);
  });
});

// Oracle for the #7839 suites below: a wrong-shape body must THROW, because
// throwing is what routes the caller through its existing error path before
// saveIdentity/updateStoredName can run — i.e. localStorage stays unchanged
// and no identity is returned. A returned value of the wrong type, or a
// coerced string, is the failure these tests pin against.
describe("meResponseCarriesName (#7838)", () => {
  it("is false when the payload carries no name field", () => {
    // The {} case from the #7838 probe: applying it as a name change
    // wiped the stored display name.
    expect(meResponseCarriesName({})).toBe(false);
    expect(meResponseCarriesName({ token: "tok" })).toBe(false);
  });

  it("is true for an explicit name, including null", () => {
    expect(meResponseCarriesName({ name: "Ada" })).toBe(true);
    expect(meResponseCarriesName({ name: null })).toBe(true);
  });

  it("is false for non-object bodies", () => {
    expect(meResponseCarriesName(null)).toBe(false);
    expect(meResponseCarriesName("abc")).toBe(false);
    expect(meResponseCarriesName(42)).toBe(false);
  });
});

describe("parseIdentityRegisterResponse (#7839)", () => {
  it("accepts a well-shaped registration", () => {
    expect(parseIdentityRegisterResponse({ token: "tok", client_id: "cid", name: "Ada" })).toEqual({
      token: "tok",
      clientId: "cid",
      name: "Ada",
    });
    expect(parseIdentityRegisterResponse({ token: "tok", client_id: "cid", name: null })).toEqual({
      token: "tok",
      clientId: "cid",
      name: null,
    });
    expect(parseIdentityRegisterResponse({ token: "tok", client_id: "cid" })).toEqual({
      token: "tok",
      clientId: "cid",
      name: null,
    });
  });

  it("rejects non-object bodies", () => {
    expect(() => parseIdentityRegisterResponse({})).toThrow();
    expect(() => parseIdentityRegisterResponse(null)).toThrow();
    expect(() => parseIdentityRegisterResponse("abc")).toThrow();
    expect(() => parseIdentityRegisterResponse([])).toThrow();
  });

  it("rejects numeric or object token/client_id instead of coercing them", () => {
    expect(() => parseIdentityRegisterResponse({ token: 123, client_id: "cid" })).toThrow();
    expect(() => parseIdentityRegisterResponse({ token: {}, client_id: "cid" })).toThrow();
    expect(() => parseIdentityRegisterResponse({ token: "tok", client_id: 123 })).toThrow();
    expect(() => parseIdentityRegisterResponse({ token: "tok", client_id: {} })).toThrow();
    expect(() => parseIdentityRegisterResponse({ token: "", client_id: "cid" })).toThrow();
    expect(() => parseIdentityRegisterResponse({ token: "tok" })).toThrow();
  });

  it("rejects a non-string name", () => {
    expect(() => parseIdentityRegisterResponse({ token: "tok", client_id: "cid", name: 123 })).toThrow();
  });
});

describe("parseIdentityMeResponse (#7839)", () => {
  it("accepts a rotation, a name-only update, and an empty object", () => {
    expect(parseIdentityMeResponse({ token: "fresh", name: "Ada" })).toEqual({ token: "fresh", name: "Ada" });
    expect(parseIdentityMeResponse({ name: null })).toEqual({ token: null, name: null });
    expect(parseIdentityMeResponse({})).toEqual({ token: null, name: null });
  });

  it("rejects non-object bodies", () => {
    expect(() => parseIdentityMeResponse(null)).toThrow();
    expect(() => parseIdentityMeResponse("abc")).toThrow();
    expect(() => parseIdentityMeResponse([])).toThrow();
  });

  it("rejects a numeric or object token instead of overwriting the stored one", () => {
    expect(() => parseIdentityMeResponse({ token: 123 })).toThrow();
    expect(() => parseIdentityMeResponse({ token: { evil: 1 } })).toThrow();
    expect(() => parseIdentityMeResponse({ token: "" })).toThrow();
    expect(() => parseIdentityMeResponse({ token: null })).toThrow();
  });

  it("rejects a non-string name", () => {
    expect(() => parseIdentityMeResponse({ name: 123 })).toThrow();
  });
});

describe("parseIdentityNameResponse (#7839)", () => {
  it("accepts a string or null name", () => {
    expect(parseIdentityNameResponse({ name: "Ada" })).toEqual({ name: "Ada" });
    expect(parseIdentityNameResponse({ name: null })).toEqual({ name: null });
  });

  it("rejects a non-string name and non-object bodies", () => {
    expect(() => parseIdentityNameResponse({ name: 123 })).toThrow();
    expect(() => parseIdentityNameResponse({ name: {} })).toThrow();
    expect(() => parseIdentityNameResponse({})).toThrow();
    expect(() => parseIdentityNameResponse(null)).toThrow();
    expect(() => parseIdentityNameResponse("abc")).toThrow();
  });
});

// ---------------------------------------------------------------------------
// Property coverage for the three parsers (#7839, Codex P1 on PR #8505).
// Oracle: the independent acceptance predicates below, written directly from
// the endpoint contracts — they share no code with the parsers. Generator: a
// seeded PRNG builds random JSON-shaped values (random field subsets, random
// value types), so a failure reproduces from the seed alone.
// ---------------------------------------------------------------------------

function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const SCALARS: unknown[] = ["tok", "cid", "Ada", "", 123, 0, true, false, null, { evil: 1 }, ["x"]];

function randomIdentityPayload(rand: () => number): unknown {
  if (rand() < 0.15) return SCALARS[Math.floor(rand() * SCALARS.length)];
  const record: Record<string, unknown> = {};
  for (const field of ["token", "client_id", "name", "extra"]) {
    if (rand() < 0.6) record[field] = SCALARS[Math.floor(rand() * SCALARS.length)];
  }
  return record;
}

function isPlainRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}
function nonEmptyString(v: unknown): v is string {
  return typeof v === "string" && v.length > 0;
}
function nameOk(v: unknown): boolean {
  return v === undefined || v === null || typeof v === "string";
}

// Independent acceptance predicates (the oracle).
function registerAccepts(v: unknown): boolean {
  return isPlainRecord(v) && nonEmptyString(v.token) && nonEmptyString(v.client_id) && nameOk(v.name);
}
function meAccepts(v: unknown): boolean {
  return isPlainRecord(v) && (v.token === undefined || nonEmptyString(v.token)) && nameOk(v.name);
}
function nameAccepts(v: unknown): boolean {
  return isPlainRecord(v) && "name" in v && nameOk(v.name);
}

describe("identity response parsers — property coverage (#7839)", () => {
  const ITERATIONS = 2000;

  it("parseIdentityRegisterResponse agrees with its predicate on generated inputs, without coercion", () => {
    const rand = mulberry32(0x7839a);
    for (let i = 0; i < ITERATIONS; i++) {
      const input = randomIdentityPayload(rand);
      if (registerAccepts(input)) {
        const rec = input as Record<string, unknown>;
        expect(parseIdentityRegisterResponse(input)).toEqual({
          token: rec.token,
          clientId: rec.client_id,
          name: typeof rec.name === "string" ? rec.name : null,
        });
      } else {
        expect(() => parseIdentityRegisterResponse(input)).toThrow();
      }
    }
  });

  it("parseIdentityMeResponse agrees with its predicate on generated inputs, without coercion", () => {
    const rand = mulberry32(0x7839b);
    for (let i = 0; i < ITERATIONS; i++) {
      const input = randomIdentityPayload(rand);
      if (meAccepts(input)) {
        const rec = input as Record<string, unknown>;
        expect(parseIdentityMeResponse(input)).toEqual({
          token: typeof rec.token === "string" ? rec.token : null,
          name: typeof rec.name === "string" ? rec.name : null,
        });
      } else {
        expect(() => parseIdentityMeResponse(input)).toThrow();
      }
    }
  });

  it("parseIdentityNameResponse agrees with its predicate on generated inputs, without coercion", () => {
    const rand = mulberry32(0x7839c);
    for (let i = 0; i < ITERATIONS; i++) {
      const input = randomIdentityPayload(rand);
      if (nameAccepts(input)) {
        const rec = input as Record<string, unknown>;
        expect(parseIdentityNameResponse(input)).toEqual({
          name: typeof rec.name === "string" ? rec.name : null,
        });
      } else {
        expect(() => parseIdentityNameResponse(input)).toThrow();
      }
    }
  });
});
