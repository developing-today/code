import { describe, expect, test } from "bun:test";
import {
  CLAUDE_COOLDOWN_CAP_MS,
  CLAUDE_COOLDOWN_FALLBACK_MS,
  CLAUDE_CROSS_TICK_STATE_KEY,
  claudeCooldownMs,
  isClaudeRateLimitFailure,
  parseClaudeCrossTick,
  retryAfterMsFromFailure,
  serializeClaudeCrossTick,
  type ClaudeCrossTickMemory,
} from "./quota-gather-map.ts";
import type { QuotaCurrentEntry } from "./model.ts";

const goodEntry = {
  providerId: "_claude",
  providerName: "Claude",
  serviceId: "claude",
  state: "ok",
  windows: [{ name: "5h", pctOfLimit: 42, resetsAt: null }],
  measuredAt: "2026-11-14T13:00:00.000Z",
} as unknown as QuotaCurrentEntry;

describe("claude cross-tick memory (#5928)", () => {
  test("state key is the reserved v1 key", () => {
    expect(CLAUDE_CROSS_TICK_STATE_KEY).toBe("claudeCrossTick:v1");
  });

  test("parse: missing or unreadable memory is an empty memory", () => {
    expect(parseClaudeCrossTick(undefined)).toEqual({ lastGoodEntry: null, cooldownUntil: 0 });
    expect(parseClaudeCrossTick("not json")).toEqual({ lastGoodEntry: null, cooldownUntil: 0 });
    expect(parseClaudeCrossTick("{}")).toEqual({ lastGoodEntry: null, cooldownUntil: 0 });
    expect(parseClaudeCrossTick('{"lastGoodEntry":42,"cooldownUntil":"soon"}')).toEqual({
      lastGoodEntry: null,
      cooldownUntil: 0,
    });
  });

  test("serialize/parse round-trips a stored last-good entry and cooldown", () => {
    const memory: ClaudeCrossTickMemory = { lastGoodEntry: goodEntry, cooldownUntil: 1760000000000 };
    const parsed = parseClaudeCrossTick(serializeClaudeCrossTick(memory));
    expect(parsed.cooldownUntil).toBe(1760000000000);
    expect(parsed.lastGoodEntry?.providerId).toBe("_claude");
    expect(parsed.lastGoodEntry?.windows[0]?.pctOfLimit).toBe(42);
  });

  test("rate-limit classification: 429 and rate_limit texts engage", () => {
    expect(isClaudeRateLimitFailure("HTTP 429 from api.anthropic.com")).toBe(true);
    expect(isClaudeRateLimitFailure('{"error":{"type":"rate_limit_error"}}')).toBe(true);
    expect(isClaudeRateLimitFailure("Rate limited. Retrying soon.")).toBe(true);
  });

  test("rate-limit classification: the 401 dead-token mapping never engages", () => {
    expect(isClaudeRateLimitFailure("Claude session expired")).toBe(false);
    expect(isClaudeRateLimitFailure("Claude session expired (was rate limited earlier)")).toBe(false);
    expect(isClaudeRateLimitFailure("connection reset by peer")).toBe(false);
  });

  test("retry-after parsing accepts header forms and rejects absence", () => {
    expect(retryAfterMsFromFailure("Retry-After: 120")).toBe(120_000);
    expect(retryAfterMsFromFailure("retry after 45s")).toBe(45_000);
    expect(retryAfterMsFromFailure("Rate limited. Retrying soon.")).toBeNull();
    expect(retryAfterMsFromFailure("Retry-After: soon")).toBeNull();
  });

  test("cooldown: Retry-After wins, fallback is 5 min, cap is 60 min", () => {
    expect(claudeCooldownMs("Retry-After: 120")).toBe(120_000);
    expect(claudeCooldownMs("Rate limited. Retrying soon.")).toBe(CLAUDE_COOLDOWN_FALLBACK_MS);
    expect(CLAUDE_COOLDOWN_FALLBACK_MS).toBe(5 * 60_000);
    expect(claudeCooldownMs("Retry-After: 7200")).toBe(CLAUDE_COOLDOWN_CAP_MS);
    expect(CLAUDE_COOLDOWN_CAP_MS).toBe(60 * 60_000);
  });
});
