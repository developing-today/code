import { describe, expect, test } from "bun:test";
import { handleEnqueueFailure } from "../src/enqueue_failure.ts";

// Regression tests for #4925: dispatch created the tracking issue
// before the enqueue, and an enqueue (or packet-validation) failure
// returned with the issue left open and unannotated — a spec-less
// orphan. handleEnqueueFailure comments the failure on the created
// issue and closes it as not planned; these tests pin that behaviour
// with a stubbed GitHub client, mirroring launch_failure.test.ts.

function stubs(failComment = false, failClose = false) {
  const calls: { comments: [string, number, string][]; closes: [string, number][]; events: string[] } = {
    comments: [],
    closes: [],
    events: [],
  };
  const github = {
    async comment(repo: string, issueNumber: number, body: string): Promise<void> {
      calls.comments.push([repo, issueNumber, body]);
      calls.events.push("comment");
      if (failComment) throw new Error("github unreachable");
    },
    async closeIssue(repo: string, issueNumber: number): Promise<void> {
      calls.closes.push([repo, issueNumber]);
      calls.events.push("close");
      if (failClose) throw new Error("github unreachable");
    },
  };
  return { calls, github };
}

describe("handleEnqueueFailure", () => {
  test("comments the failure and closes the created issue as not planned", async () => {
    const { calls, github } = stubs();
    const outcome = await handleEnqueueFailure(
      github,
      "developing-today/code",
      4925,
      "queue enqueue",
      new Error("supabase insert failed"),
    );
    expect(outcome).toEqual({ commented: true, closed: true });
    // The evidence lands before the close.
    expect(calls.events).toEqual(["comment", "close"]);
    expect(calls.comments).toHaveLength(1);
    expect(calls.comments[0][0]).toBe("developing-today/code");
    expect(calls.comments[0][1]).toBe(4925);
    expect(calls.comments[0][2]).toContain("queue enqueue");
    expect(calls.comments[0][2]).toContain("supabase insert failed");
    expect(calls.comments[0][2]).toContain("not planned");
    expect(calls.closes).toEqual([["developing-today/code", 4925]]);
  });

  test("still closes when the comment itself fails", async () => {
    const { calls, github } = stubs(true);
    const outcome = await handleEnqueueFailure(github, "developing-today/code", 7, "packet validation", "string failure");
    expect(outcome).toEqual({ commented: false, closed: true });
    expect(calls.closes).toEqual([["developing-today/code", 7]]);
  });

  test("reports honestly when the close fails", async () => {
    const { calls, github } = stubs(false, true);
    const outcome = await handleEnqueueFailure(github, "developing-today/code", 7, "queue enqueue", new Error("boom"));
    expect(outcome).toEqual({ commented: true, closed: false });
    expect(calls.comments).toHaveLength(1);
    expect(calls.closes).toHaveLength(1);
  });
});
