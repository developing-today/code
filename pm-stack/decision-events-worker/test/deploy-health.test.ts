import { describe, expect, test } from "bun:test";
import { matchesHealth } from "../scripts/check-health.mjs";

describe("deployment health gate", () => {
  const deployedSha = "0123456789abcdef0123456789abcdef01234567";

  test("accepts only a healthy response reporting the exact deployed SHA", () => {
    expect(matchesHealth({ ok: true, version: deployedSha }, deployedSha)).toBe(true);
  });

  test("rejects a stale, missing, or unhealthy version", () => {
    expect(matchesHealth({ ok: true, version: "abcdef012345" }, deployedSha)).toBe(false);
    expect(matchesHealth({ ok: true, version: "unknown" }, deployedSha)).toBe(false);
    expect(matchesHealth({ ok: false, version: deployedSha }, deployedSha)).toBe(false);
    expect(matchesHealth({}, deployedSha)).toBe(false);
  });
});
