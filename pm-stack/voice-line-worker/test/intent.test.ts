import { describe, expect, test } from "bun:test";
import { matchIntent, buildAnswer } from "../src/intent";

describe("matchIntent", () => {
  test("recognises queue-state phrasings", () => {
    expect(matchIntent("How many tasks are in the queue?")).toBe("queue_state");
    expect(matchIntent("what's the queue state")).toBe("queue_state");
    expect(matchIntent("tell me about the backlog")).toBe("queue_state");
  });

  test("anything else is unknown (read-only line, no write intents)", () => {
    expect(matchIntent("merge the pull request")).toBe("unknown");
    expect(matchIntent("what's the weather")).toBe("unknown");
    expect(matchIntent("")).toBe("unknown");
  });
});

describe("buildAnswer", () => {
  test("speaks the snapshot numbers", () => {
    const answer = buildAnswer("queue_state", {
      queued: 203,
      claimed: 6,
      parked: 17,
      blocked: 3,
      at: "2026-11-14T14:00:00Z",
    });
    expect(answer).toContain("203 tasks waiting");
    expect(answer).toContain("6 being worked");
    expect(answer).toContain("17 parked");
    expect(answer).toContain("3 blocked");
  });

  test("no snapshot is an honest spoken fallback, not silence", () => {
    expect(buildAnswer("queue_state", null)).toContain("don't have a fresh queue snapshot");
  });

  test("unknown intent explains the one capability", () => {
    expect(buildAnswer("unknown", null)).toContain("queue state");
  });
});
