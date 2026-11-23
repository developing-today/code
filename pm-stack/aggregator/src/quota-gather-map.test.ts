import { describe, expect, test } from "bun:test";
import {
  CLAUDE_COOLDOWN_CAP_MS,
  CLAUDE_COOLDOWN_FALLBACK_MS,
  CLAUDE_CROSS_TICK_STATE_KEY,
  CLAUDE_RETRY_BASE_MS,
  CLAUDE_RETRY_CAP_MS,
  CLAUDE_RETRY_MAX_ATTEMPTS,
  claudeCooldownMs,
  claudeFailureKind,
  claudeRetryDelayMs,
  isClaudeRateLimitFailure,
  markRetained,
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

describe("claude within-tick retry and failure kind (#8725)", () => {
  test("failure kind: rate limit, credential, and other are distinct", () => {
    expect(claudeFailureKind("HTTP 429 from api.anthropic.com")).toBe("rate_limited");
    expect(claudeFailureKind("Rate limited. Retrying soon.")).toBe("rate_limited");
    expect(claudeFailureKind("Claude session expired")).toBe("credential");
    expect(claudeFailureKind("HTTP 401 unauthorized")).toBe("credential");
    expect(claudeFailureKind("connection reset by peer")).toBe("other");
  });

  test("retry budget is bounded", () => {
    expect(CLAUDE_RETRY_MAX_ATTEMPTS).toBe(3);
    expect(CLAUDE_RETRY_BASE_MS).toBe(5_000);
    expect(CLAUDE_RETRY_CAP_MS).toBe(30_000);
  });

  test("retry delay: exponential from base, jittered to [50%, 100%]", () => {
    const hi = () => 1;
    const lo = () => 0;
    expect(claudeRetryDelayMs(0, "Rate limited.", hi, 5_000)).toBe(5_000);
    expect(claudeRetryDelayMs(1, "Rate limited.", hi, 5_000)).toBe(10_000);
    expect(claudeRetryDelayMs(2, "Rate limited.", hi, 5_000)).toBe(20_000);
    expect(claudeRetryDelayMs(0, "Rate limited.", lo, 5_000)).toBe(2_500);
  });

  test("retry delay: Retry-After wins over the exponential base and is capped", () => {
    const hi = () => 1;
    expect(claudeRetryDelayMs(0, "Retry-After: 12", hi, 5_000)).toBe(12_000);
    expect(claudeRetryDelayMs(0, "Retry-After: 7200", hi, 5_000)).toBe(CLAUDE_RETRY_CAP_MS);
    expect(claudeRetryDelayMs(9, "Rate limited.", hi, 5_000)).toBe(CLAUDE_RETRY_CAP_MS);
  });

  test("markRetained stamps the failure kind only when one is given", () => {
    const tick = "2026-11-17T12:00:00.000Z";
    const [stamped] = markRetained([goodEntry], tick, "rate_limited");
    expect(stamped.retainedFromFailedTick).toBe(tick);
    expect(stamped.retentionFailureKind).toBe("rate_limited");
    const [unstamped] = markRetained([goodEntry], tick);
    expect(unstamped.retentionFailureKind).toBeUndefined();
  });
});
