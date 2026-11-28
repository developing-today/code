import { describe, expect, it } from "vitest";
import { parseWorldSeed } from "./world-seed";

describe("parseWorldSeed", () => {
  it("rejects an empty or whitespace-only field instead of silently using seed 0", () => {
    expect(parseWorldSeed("")).toBeNull();
    expect(parseWorldSeed("  ")).toBeNull();
  });

  it("accepts an explicitly typed 0 and other non-negative safe integers", () => {
    expect(parseWorldSeed("0")).toBe(0);
    expect(parseWorldSeed("1")).toBe(1);
    expect(parseWorldSeed(" 42 ")).toBe(42);
    expect(parseWorldSeed(String(Number.MAX_SAFE_INTEGER))).toBe(Number.MAX_SAFE_INTEGER);
  });

  it("rejects negatives, fractions, unsafe integers and non-numeric input", () => {
    expect(parseWorldSeed("-1")).toBeNull();
    expect(parseWorldSeed("2.5")).toBeNull();
    expect(parseWorldSeed(String(Number.MAX_SAFE_INTEGER + 1))).toBeNull();
    expect(parseWorldSeed("abc")).toBeNull();
  });

  it("rejects non-decimal numeric forms instead of reinterpreting them", () => {
    // Number() coercion would silently turn each of these into a seed the
    // admin never saw in the field (#832).
    expect(parseWorldSeed("0x10")).toBeNull();
    expect(parseWorldSeed("0b101")).toBeNull();
    expect(parseWorldSeed("0o17")).toBeNull();
    expect(parseWorldSeed("1e3")).toBeNull();
    expect(parseWorldSeed("+5")).toBeNull();
  });
});
