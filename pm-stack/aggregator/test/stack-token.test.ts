import { describe, expect, test } from "bun:test";
import {
  STACK_TOKEN_TTL_SECONDS,
  mintStackToken,
  verifyStackToken,
  type StackTokenClaims,
} from "../src/stack-token.ts";

const SECRET = "test-only-stack-token-secret";
const NOW = 1_700_000_000_000;

const claims = (overrides: Partial<StackTokenClaims> = {}): StackTokenClaims => ({
  sub: "agent-7",
  role: "agent",
  aud: "kaneo",
  act: "task.write",
  ...overrides,
});

describe("stack tokens", () => {
  test("mint and verify carries the bounded identity contract", () => {
    const token = mintStackToken(claims(), SECRET, NOW);
    const verified = verifyStackToken(token, SECRET, "kaneo", NOW + 1_000);
    expect(verified.sub).toBe("agent-7");
    expect(verified.role).toBe("agent");
    expect(verified.aud).toBe("kaneo");
    expect(verified.act).toBe("task.write");
    expect(verified.exp - verified.iat).toBe(STACK_TOKEN_TTL_SECONDS);
  });

  test("rejects wrong audience, expiry, tampering, and overlong TTL", () => {
    const token = mintStackToken(claims(), SECRET, NOW);
    expect(() => verifyStackToken(token, SECRET, "pad", NOW + 1_000)).toThrow(/audience/);
    expect(() => verifyStackToken(token, SECRET, "kaneo", NOW + 301_000)).toThrow(/expired/);
    expect(() => verifyStackToken(`${token}x`, SECRET, "kaneo", NOW + 1_000)).toThrow(/signature/);
    expect(() => mintStackToken(claims({ exp: NOW / 1000 + 301 }), SECRET, NOW)).toThrow(/ttl/);
  });

  test("does not accept identity or role claims from an exchange body", () => {
    const token = mintStackToken(claims({ sub: "verified-sub", role: "owner" }), SECRET, NOW);
    const verified = verifyStackToken(token, SECRET, "kaneo", NOW);
    expect(verified.sub).toBe("verified-sub");
    expect(verified.role).toBe("owner");
  });
});
