import { describe, expect, test } from "bun:test";
import { classifyAttention, tierEntry } from "../src/derive.ts";
import { FIXTURE_ATTENTION } from "../src/sources/fixtures.ts";

describe("attention classification", () => {
  const now = Date.parse("2026-10-25T02:00:00Z");

  test("retry-loop threshold outranks turn errors, while questions remain first", () => {
    const mapperShaped = classifyAttention(
      {
        sessionId: "retry-with-error",
        lastActivityAt: "2026-10-25T01:00:00Z",
        lastTurnError: "boom",
        repeatedErrorCount: 4,
      },
      now,
    );
    expect(mapperShaped.state).toBe("stuck-retrying");

    expect(
      classifyAttention(
        {
          sessionId: "threshold-minus-one",
          lastActivityAt: "2026-10-25T01:00:00Z",
          lastTurnError: "boom",
          repeatedErrorCount: 2,
        },
        now,
      ).state,
    ).toBe("errored");
    expect(
      classifyAttention(
        { sessionId: "single-error", lastActivityAt: "2026-10-25T01:00:00Z", lastTurnError: "boom" },
        now,
      ).state,
    ).toBe("errored");

    expect(
      classifyAttention(
        {
          sessionId: "question-and-retry",
          lastActivityAt: "2026-10-25T01:00:00Z",
          awaitingQuestion: "which repo first?",
          lastTurnError: "boom",
          repeatedErrorCount: 4,
        },
        now,
      ).state,
    ).toBe("stopped-for-question");

    const credentialRetry = tierEntry(
      classifyAttention(
        {
          sessionId: "credential-retry",
          lastActivityAt: "2026-10-25T01:00:00Z",
          lastTurnError: "verification code required",
          repeatedErrorCount: 4,
        },
        now,
      ),
    );
    expect(credentialRetry.state).toBe("stuck-retrying");
    expect(credentialRetry.tier).toBe("needs-drew");

    const phase13 = FIXTURE_ATTENTION.find((entry) => entry.sessionId === "ses_f_phase13");
    expect(phase13).toBeDefined();
    expect(classifyAttention(phase13!, now).state).toBe("stuck-retrying");
  });
});
