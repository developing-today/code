import { describe, expect, it } from "vitest";
import { keyMaterial, normalizeQuery, queryKey } from "../src/key";
import { CEILING, CONTEXT, MODEL_REV, VEC_TABLE } from "../src/space";

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
      `{"v":2,"q":"alpha beta","ceiling":"${CEILING}","context":"${CONTEXT}",` +
      `"model_rev":"${MODEL_REV}","vec_table":"${VEC_TABLE}","k":10}`;
    expect(keyMaterial({ q: "alpha beta", k: 10, context: CONTEXT })).toBe(canonical);
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(canonical));
    const expected = [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
    expect(await queryKey({ q: "alpha beta", k: 10, context: CONTEXT })).toBe(expected);
  });

  it("differs across query and across k (no cross-config serving)", async () => {
    const a = await queryKey({ q: "alpha", k: 10, context: CONTEXT });
    const b = await queryKey({ q: "beta", k: 10, context: CONTEXT });
    const c = await queryKey({ q: "alpha", k: 5, context: CONTEXT });
    expect(a).not.toBe(b);
    expect(a).not.toBe(c);
  });

  it("differs across disclosure contexts (no cross-context serving, #12622)", async () => {
    // Oracle: #12622 acceptance — a payload cached under one
    // disclosure context must never be served to a request
    // resolved to another. The Durable Object's name IS the
    // key, so keys that differ only in context must differ.
    const hosted = await queryKey({ q: "alpha", k: 10, context: "hosted-per-user" });
    const internal = await queryKey({ q: "alpha", k: 10, context: "fleet-internal" });
    const pub = await queryKey({ q: "alpha", k: 10, context: "public-surface" });
    expect(hosted).not.toBe(internal);
    expect(hosted).not.toBe(pub);
    expect(internal).not.toBe(pub);
  });
});
