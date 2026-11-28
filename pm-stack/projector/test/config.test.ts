import { describe, test, expect } from "bun:test";
import { DEFAULT_PORT, parsePort } from "../src/config";

describe("parsePort", () => {
  test("uses the existing default only when PORT is unset", () => {
    expect(DEFAULT_PORT).toBe(7801);
    expect(parsePort(undefined)).toBe(7801);
  });

  test("accepts plain decimal ports in the valid range", () => {
    expect(parsePort("1")).toBe(1);
    expect(parsePort("7801")).toBe(7801);
    expect(parsePort("65535")).toBe(65535);
  });

  test("rejects zero, malformed, non-decimal, and out-of-range values", () => {
    for (const raw of ["", "0", "-1", "99999", "4870.5", "abc", "1e2", "0x10", "+5", " 5", "5 "]) {
      let thrown: unknown;
      try {
        parsePort(raw);
      } catch (error) {
        thrown = error;
      }
      expect(thrown).toBeInstanceOf(Error);
      const message = (thrown as Error).message;
      expect(message).toContain("PORT");
      expect(message).toContain(JSON.stringify(raw));
      expect(message).toContain("1-65535");
    }
  });
});
