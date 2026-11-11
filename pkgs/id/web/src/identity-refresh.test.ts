import { describe, expect, it } from "vitest";
import {
  identityNeedsRefresh,
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
