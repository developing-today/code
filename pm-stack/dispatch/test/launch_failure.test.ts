import { describe, expect, test } from "bun:test";
import { handleLaunchFailure } from "../src/launch_failure.ts";

// Regression tests for #2901: a wait:false launch failure used to only
// PATCH last_error onto the task, leaving it queued forever. Both wait
// paths now share handleLaunchFailure, which cancels the task and
// notes the tracking issue — these tests pin that behaviour with
// stubbed clients.

function stubs(failCancel = false) {
  const calls: { cancels: [number, string][]; comments: [string, number, string][] } = {
    cancels: [],
    comments: [],
  };
  const queue = {
    async cancelTask(id: number, reason: string): Promise<void> {
      calls.cancels.push([id, reason]);
      if (failCancel) throw new Error("queue unreachable");
    },
  };
  const github = {
    async comment(repo: string, issueNumber: number, body: string): Promise<void> {
      calls.comments.push([repo, issueNumber, body]);
    },
  };
  return { calls, queue, github };
}

describe("handleLaunchFailure", () => {
  test("cancels the task and notes the issue on a launch failure", async () => {
    const { calls, queue, github } = stubs();
    const msg = await handleLaunchFailure(
      queue,
      github,
      "developing-today/code",
      4135,
      2901,
      new Error("opencode refused the session"),
    );
    expect(msg).toBe("opencode refused the session");
    expect(calls.cancels).toEqual([
      [4135, "dispatch launch failed: opencode refused the session"],
    ]);
    expect(calls.comments).toHaveLength(1);
    expect(calls.comments[0][0]).toBe("developing-today/code");
    expect(calls.comments[0][1]).toBe(2901);
    expect(calls.comments[0][2]).toContain("queue task #4135 was cancelled");
    expect(calls.comments[0][2]).toContain("opencode refused the session");
  });

  test("cancels without an issue note when there is no tracking issue", async () => {
    const { calls, queue, github } = stubs();
    await handleLaunchFailure(queue, github, "developing-today/code", 7, null, new Error("boom"));
    expect(calls.cancels).toHaveLength(1);
    expect(calls.comments).toHaveLength(0);
  });

  test("still notes the issue when the cancellation itself fails", async () => {
    const { calls, queue, github } = stubs(true);
    const msg = await handleLaunchFailure(queue, github, "developing-today/code", 7, 9, "string failure");
    expect(msg).toBe("string failure");
    expect(calls.comments).toHaveLength(1);
    expect(calls.comments[0][2]).toContain("string failure");
    // The note must report the cancellation's actual outcome (#2917):
    // cancellation FAILED, the task may still be queued/claimed and
    // needs cleanup, with the cancellation error named — and it must
    // never claim the task was cancelled.
    expect(calls.comments[0][2]).toContain("cancelling queue task #7 FAILED");
    expect(calls.comments[0][2]).toContain("may still be queued or claimed and needs cleanup");
    expect(calls.comments[0][2]).toContain("Cancellation error: queue unreachable");
    expect(calls.comments[0][2]).not.toContain("was cancelled");
  });
});
