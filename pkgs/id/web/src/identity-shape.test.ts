// Regression coverage for #7838: the /api/identity/* response shapes
// that used to corrupt the stored identity. Oracle = the acceptance on
// #7838: a wrong-shape body persists nothing and wipes nothing; a
// well-formed body behaves exactly as before.
import { describe, expect, it } from "vitest";
import { parseMeBody, parseNameBody, parseRegisterBody } from "./identity-shape";

describe("parseRegisterBody (#7838)", () => {
  it("rejects empty, non-object, and null bodies — these persisted 'undefined' pre-fix", () => {
    expect(parseRegisterBody({})).toBeNull();
    expect(parseRegisterBody("abc")).toBeNull();
    expect(parseRegisterBody(null)).toBeNull();
    expect(parseRegisterBody([])).toBeNull();
  });

  it("rejects non-string and empty token/client_id — these were coerced pre-fix", () => {
    expect(parseRegisterBody({ token: 123, client_id: 456 })).toBeNull();
    expect(parseRegisterBody({ token: "", client_id: "c" })).toBeNull();
    expect(parseRegisterBody({ token: "t", client_id: "" })).toBeNull();
    expect(parseRegisterBody({ token: "t" })).toBeNull();
  });

  it("rejects a wrong-typed name but accepts string, null, or absent", () => {
    expect(parseRegisterBody({ token: "t", client_id: "c", name: 42 })).toBeNull();
    expect(parseRegisterBody({ token: "t", client_id: "c", name: "Ada" })).toEqual({
      token: "t",
      clientId: "c",
      name: "Ada",
    });
    expect(parseRegisterBody({ token: "t", client_id: "c", name: null })).toEqual({
      token: "t",
      clientId: "c",
      name: null,
    });
    expect(parseRegisterBody({ token: "t", client_id: "c" })).toEqual({
      token: "t",
      clientId: "c",
      name: null,
    });
  });
});

describe("parseMeBody (#7838)", () => {
  it("a {} body updates nothing — pre-fix it wiped the stored name", () => {
    expect(parseMeBody({})).toEqual({ token: null, nameOk: false, name: null });
  });

  it("a non-object body updates nothing instead of throwing", () => {
    expect(parseMeBody(null)).toEqual({ token: null, nameOk: false, name: null });
    expect(parseMeBody("abc")).toEqual({ token: null, nameOk: false, name: null });
  });

  it("applies a token refresh only for a non-empty string token", () => {
    expect(parseMeBody({ token: "fresh" }).token).toBe("fresh");
    expect(parseMeBody({ token: "" }).token).toBeNull();
    expect(parseMeBody({ token: 123 }).token).toBeNull();
  });

  it("treats the name as authoritative only when string or explicit null", () => {
    expect(parseMeBody({ name: "Ada" })).toEqual({ token: null, nameOk: true, name: "Ada" });
    expect(parseMeBody({ name: null })).toEqual({ token: null, nameOk: true, name: null });
    expect(parseMeBody({ name: 42 }).nameOk).toBe(false);
  });
});

describe("parseNameBody (#7838)", () => {
  it("a missing or wrong-typed name is not ok — the stored name must survive", () => {
    expect(parseNameBody({})).toEqual({ ok: false });
    expect(parseNameBody({ name: 42 })).toEqual({ ok: false });
    expect(parseNameBody(null)).toEqual({ ok: false });
  });

  it("accepts a string name and an explicit null", () => {
    expect(parseNameBody({ name: "Ada" })).toEqual({ ok: true, name: "Ada" });
    expect(parseNameBody({ name: null })).toEqual({ ok: true, name: null });
  });
});
