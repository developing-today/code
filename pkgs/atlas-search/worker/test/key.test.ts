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

  it("never shares a result key across disclosure contexts for generated queries and depths", async () => {
    // Generated context-isolation coverage: for each generated (query, k),
    // all four disclosure contexts must produce distinct cache keys, so a
    // cached entry can never be served across a disclosure boundary.
    // Oracle: the key digest itself — the context is part of the key
    // material, so distinct contexts must yield distinct digests.
    // Deterministic mulberry32 generator (fixed seed); no new dependency.
    const contexts = ["fleet-internal", "hosted-per-user", "public-surface", "local-owner"] as const;
    let seed = 0x21057;
    const next = () => {
      seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    const words = ["alpha", "beta", "fleet", "queue", "café", "naïve", "x", "deep", "search", "42", "MUSE"];
    const spaces = [" ", "  ", "\t", "\n", " \t "];
    const pick = <T>(xs: readonly T[]): T => xs[Math.floor(next() * xs.length)];
    for (let i = 0; i < 64; i++) {
      const wordCount = 1 + Math.floor(next() * 4);
      let q = "";
      for (let w = 0; w < wordCount; w++) {
        if (w > 0) q += pick(spaces);
        const word = pick(words);
        q += next() < 0.25 ? word.toUpperCase() : word;
      }
      if (next() < 0.5) q = pick(spaces) + q;
      if (next() < 0.5) q = q + pick(spaces);
      const k = 1 + Math.floor(next() * 50);
      const keys = await Promise.all(contexts.map((context) => queryKey({ q, k, context })));
      expect(new Set(keys).size).toBe(contexts.length);
    }
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
