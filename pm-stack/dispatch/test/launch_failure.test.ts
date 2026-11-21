import { describe, expect, test } from "bun:test";
import { compensateOrphanedIssue, handleLaunchFailure, withCreatedSession } from "../src/launch_failure.ts";
import { ClaimLostError } from "../src/queue.ts";

// Regression tests for #2901: a wait:false launch failure used to only
// PATCH last_error onto the task, leaving it queued forever. Both wait
// paths now share handleLaunchFailure, which cancels the task and
// notes the tracking issue — these tests pin that behaviour with
// stubbed clients.

function stubs(failCancel = false) {
  const calls: {
    cancels: [number, string][];
    comments: [string, number, string][];
    closes: [string, number, string | undefined][];
  } = {
    cancels: [],
    comments: [],
    closes: [],
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
    async closeIssue(repo: string, issueNumber: number, stateReason?: "not_planned" | "completed"): Promise<void> {
      calls.closes.push([repo, issueNumber, stateReason]);
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
    expect(calls.cancels).toEqual([[4135, "dispatch launch failed: opencode refused the session"]]);
    expect(calls.comments).toHaveLength(1);
    expect(calls.comments[0][0]).toBe("developing-today/code");
    expect(calls.comments[0][1]).toBe(2901);
    expect(calls.comments[0][2]).toContain("queue task #4135 was cancelled");
    expect(calls.comments[0][2]).toContain("opencode refused the session");
    // #4917: the failed launch's tracking issue is closed, not orphaned.
    expect(calls.closes).toEqual([["developing-today/code", 2901, "not_planned"]]);
  });

  test("cancels without an issue note when there is no tracking issue", async () => {
    const { calls, queue, github } = stubs();
    await handleLaunchFailure(queue, github, "developing-today/code", 7, null, new Error("boom"));
    expect(calls.cancels).toHaveLength(1);
    expect(calls.comments).toHaveLength(0);
    expect(calls.closes).toHaveLength(0);
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
    // Even when cancellation fails, the issue is still closed (#4917):
    // the launch definitively failed either way.
    expect(calls.closes).toEqual([["developing-today/code", 9, "not_planned"]]);
  });

  test("notes a deleted session without the may-still-be-live hedge when cleanup succeeds (#10632)", async () => {
    const sessionId = "session/deleted-7";
    const launchError = new Error("prompt failed");
    const calls = {
      deleted: [] as string[],
      cancels: [] as [number, string][],
      comments: [] as [string, number, string][],
      closes: [] as [string, number, string | undefined][],
      events: [] as string[],
    };
    const queue = {
      async cancelTask(id: number, reason: string): Promise<void> {
        calls.events.push("cancel");
        calls.cancels.push([id, reason]);
      },
    };
    const github = {
      async comment(repo: string, issueNumber: number, body: string): Promise<void> {
        calls.events.push("comment");
        calls.comments.push([repo, issueNumber, body]);
      },
      async closeIssue(repo: string, issueNumber: number, stateReason?: "not_planned" | "completed"): Promise<void> {
        calls.events.push("close");
        calls.closes.push([repo, issueNumber, stateReason]);
      },
    };
    const msg = await handleLaunchFailure(
      queue,
      github,
      "developing-today/code",
      4135,
      2901,
      launchError,
      sessionId,
      async (id: string) => {
        calls.events.push("delete");
        calls.deleted.push(id);
      },
    );
    expect(msg).toBe(launchError.message);
    // The cancel reason is written before the DELETE runs, so its hedge
    // is still accurate and stays out of scope for this fix.
    expect(calls.cancels).toEqual([
      [4135, `dispatch launch failed: prompt failed; OpenCode session ${sessionId} may still be live`],
    ]);
    expect(calls.deleted).toEqual([sessionId]);
    expect(calls.comments).toHaveLength(1);
    expect(calls.comments[0][2]).toContain(`OpenCode session ${sessionId} was deleted`);
    expect(calls.comments[0][2]).toContain("session cleanup succeeded");
    expect(calls.comments[0][2]).not.toContain("may still be live");
    expect(calls.events).toEqual(["cancel", "delete", "comment", "close"]);
    expect(calls.closes).toEqual([["developing-today/code", 2901, "not_planned"]]);
  });

  async function assertCreatedSessionFailure(stage: "sendPrompt" | "claim") {
    const sessionId = "session/created-42";
    const launchError = new Error(`${stage} failed`);
    const context = { sessionId: null as string | null, worktree: null as string | null };
    const calls = {
      deleted: [] as string[],
      cancels: [] as [number, string][],
      comments: [] as [string, number, string][],
      closes: [] as [string, number, string | undefined][],
      events: [] as string[],
    };
    const opencode = {
      async createSession(): Promise<{ id: string }> {
        return { id: sessionId };
      },
      async sendPrompt(): Promise<void> {
        if (stage === "sendPrompt") throw launchError;
      },
      async deleteSession(id: string): Promise<void> {
        calls.events.push("delete");
        calls.deleted.push(id);
        throw new Error("delete failed");
      },
    };
    const queue = {
      async ensureWorker(): Promise<void> {},
      async claimTask(): Promise<void> {
        if (stage === "claim") throw launchError;
      },
      async cancelTask(id: number, reason: string): Promise<void> {
        calls.events.push("cancel");
        calls.cancels.push([id, reason]);
      },
    };
    const github = {
      async comment(repo: string, issueNumber: number, body: string): Promise<void> {
        calls.events.push("comment");
        calls.comments.push([repo, issueNumber, body]);
      },
      async closeIssue(repo: string, issueNumber: number, stateReason?: "not_planned" | "completed"): Promise<void> {
        calls.events.push("close");
        calls.closes.push([repo, issueNumber, stateReason]);
      },
    };

    // Model the real order: session creation succeeds before the guarded
    // prompt / worker / claim steps run.
    const session = await opencode.createSession();
    let thrown: unknown;
    try {
      await withCreatedSession(session.id, context, async () => {
        await opencode.sendPrompt();
        await queue.ensureWorker();
        await queue.claimTask();
      });
    } catch (e) {
      thrown = e;
    }

    expect(thrown).toBe(launchError);
    expect(context.sessionId).toBe(sessionId);
    expect(calls.deleted).toEqual([]);

    const msg = await handleLaunchFailure(
      queue,
      github,
      "developing-today/code",
      4135,
      2901,
      thrown,
      context.sessionId,
      (id) => opencode.deleteSession(id),
    );
    expect(msg).toBe(launchError.message);
    expect(calls.cancels).toEqual([
      [4135, `dispatch launch failed: ${launchError.message}; OpenCode session ${sessionId} may still be live`],
    ]);
    expect(calls.comments).toHaveLength(1);
    expect(calls.comments[0][2]).toContain(`OpenCode session ${sessionId} may still be live`);
    expect(calls.comments[0][2]).toContain("session cleanup failed: delete failed");
    expect(calls.comments[0][2]).toContain(launchError.message);
    expect(calls.deleted).toEqual([sessionId]);
    expect(calls.events).toEqual(["cancel", "delete", "comment", "close"]);
    expect(calls.closes).toEqual([["developing-today/code", 2901, "not_planned"]]);
  }

  test("created session is cleaned up and reported after sendPrompt fails", async () => {
    await assertCreatedSessionFailure("sendPrompt");
  });

  test("created session is cleaned up and reported after the claim fails", async () => {
    await assertCreatedSessionFailure("claim");
  });
});

describe("compensateOrphanedIssue (#4917)", () => {
  test("notes and closes the issue a failed pre-enqueue dispatch created", async () => {
    const { calls, github } = stubs();
    const msg = await compensateOrphanedIssue(
      github,
      "developing-today/code",
      4917,
      "enqueue",
      new Error("queue insert failed"),
    );
    expect(msg).toBe("queue insert failed");
    expect(calls.comments).toHaveLength(1);
    expect(calls.comments[0][0]).toBe("developing-today/code");
    expect(calls.comments[0][1]).toBe(4917);
    expect(calls.comments[0][2]).toContain("FAILED at enqueue");
    expect(calls.comments[0][2]).toContain("queue insert failed");
    expect(calls.closes).toEqual([["developing-today/code", 4917, "not_planned"]]);
  });

  test("is a no-op when no issue was created", async () => {
    const { calls, github } = stubs();
    const msg = await compensateOrphanedIssue(github, "developing-today/code", null, "enqueue", new Error("boom"));
    expect(msg).toBe("boom");
    expect(calls.comments).toHaveLength(0);
    expect(calls.closes).toHaveLength(0);
  });

  test("still closes when the note fails, and never throws when the close fails", async () => {
    const closes: number[] = [];
    const flaky = {
      async comment(): Promise<void> {
        throw new Error("github down");
      },
      async closeIssue(_repo: string, issueNumber: number): Promise<void> {
        closes.push(issueNumber);
      },
    };
    const msg = await compensateOrphanedIssue(flaky, "developing-today/code", 5, "packet validation", "bad packet");
    expect(msg).toBe("bad packet");
    expect(closes).toEqual([5]);

    const broken = {
      async comment(): Promise<void> {},
      async closeIssue(): Promise<void> {
        throw new Error("github down");
      },
    };
    await expect(compensateOrphanedIssue(broken, "developing-today/code", 6, "enqueue", new Error("x"))).resolves.toBe(
      "x",
    );
  });
});

// #10737: a launch that lost the claim race must leave the task and its
// issue to the worker that claimed them, and only clean up its own
// session.
describe("handleLaunchFailure on a lost claim (#10737)", () => {
  test("does not cancel, note or close; deletes only the launch's session", async () => {
    const { calls, queue, github } = stubs();
    const deleted: string[] = [];
    const msg = await handleLaunchFailure(
      queue,
      github,
      "developing-today/code",
      4135,
      2901,
      new ClaimLostError(4135, "queued"),
      "ses_lost",
      async (id) => {
        deleted.push(id);
      },
    );
    expect(msg).toContain("no longer queued");
    expect(calls.cancels).toEqual([]);
    expect(calls.comments).toEqual([]);
    expect(calls.closes).toEqual([]);
    expect(deleted).toEqual(["ses_lost"]);
  });
});
