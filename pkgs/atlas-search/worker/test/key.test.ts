import { describe, expect, it } from "vitest";
import { keyMaterial, normalizeQuery, queryKey } from "../src/key";
import { CEILING, MODEL_REV, VEC_TABLE } from "../src/space";

describe("normalizeQuery", () => {
  it("trims and collapses whitespace runs", () => {
    expect(normalizeQuery("  alpha \t beta\n\n gamma ")).toBe("alpha beta gamma");
  });
  it("applies NFC", () => {
    // e + combining acute must equal the precomposed form.
    expect(normalizeQuery("café")).toBe("café");
  });
  it("preserves case (the fan-out is case-sensitive)", () => {
    expect(normalizeQuery("Fleet QUEUE")).toBe("Fleet QUEUE");
  });
});

describe("queryKey", () => {
  it("matches a hand-built canonical digest", async () => {
    // Oracle: the canonical material constructed literally here,
    // field by field — a drift in key.ts's field order, names, or
    // pinned constants fails this test.
    const canonical =
      `{"v":2,"q":"alpha beta","context":"hosted-per-user","ceiling":"${CEILING}",` +
      `"model_rev":"${MODEL_REV}","vec_table":"${VEC_TABLE}","k":10}`;
    expect(keyMaterial({ q: "alpha beta", k: 10, context: "hosted-per-user" })).toBe(canonical);
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(canonical));
    const expected = [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
    expect(await queryKey({ q: "alpha beta", k: 10, context: "hosted-per-user" })).toBe(expected);
  });

  it("differs across query and across k (no cross-config serving)", async () => {
    const a = await queryKey({ q: "alpha", k: 10, context: "hosted-per-user" });
    const b = await queryKey({ q: "beta", k: 10, context: "hosted-per-user" });
    const c = await queryKey({ q: "alpha", k: 5, context: "hosted-per-user" });
    expect(a).not.toBe(b);
    expect(a).not.toBe(c);
  });

  it("never shares a result key across disclosure contexts", async () => {
    const keyForContext = (context: "hosted-per-user" | "public-surface") =>
      queryKey({ q: "same query", k: 10, context });
    expect(await keyForContext("hosted-per-user")).not.toBe(await keyForContext("public-surface"));
  });
});
