import { describe, expect, test } from "bun:test";
import { annotateLaunch } from "../src/launch_annotations.ts";

// Regression tests for #2660 (design #2668 phase 2): post-claim GitHub
// annotations used to share one try/catch in launchSession, so a failed
// issue comment silently skipped the label attempt and a wait:true
// caller saw a clean success. annotateLaunch is the single helper both
// wait modes reach through launchSession: each annotation is guarded
// independently, each failure is noted on the task and returned as a
// warning, and the remaining steps still run. The launch itself is
// never rejected — these stubs pin that contract.

function stubs(
  opts: {
    failComment?: boolean;
    failLabels?: boolean;
    failNote?: boolean;
    failGet?: boolean;
    preExisting?: string | null;
  } = {},
) {
  const calls: {
    comments: [string, number, string][];
    labels: [string, number, string[]][];
    patches: [number, string][];
    gets: number[];
  } = { comments: [], labels: [], patches: [], gets: [] };
  const github = {
    async comment(repo: string, issueNumber: number, body: string): Promise<void> {
      calls.comments.push([repo, issueNumber, body]);
      if (opts.failComment) throw new Error("github comment 500");
    },
    async addLabels(repo: string, issueNumber: number, labels: string[]): Promise<void> {
      calls.labels.push([repo, issueNumber, labels]);
      if (opts.failLabels) throw new Error("github labels 500");
    },
  };
  const queue = {
    async noteTask(id: number, note: string): Promise<void> {
      calls.patches.push([id, note]);
      if (opts.failNote) throw new Error("queue unreachable");
    },
    async getTask(id: number): Promise<{ last_error?: unknown } | null> {
      calls.gets.push(id);
      if (opts.failGet) throw new Error("queue read unreachable");
      return { last_error: opts.preExisting ?? null };
    },
  };
  return { calls, github, queue };
}

function run(github: unknown, queue: unknown) {
  return annotateLaunch(
    github as Parameters<typeof annotateLaunch>[0],
    queue as Parameters<typeof annotateLaunch>[1],
    "developing-today/code",
    2660,
    5414,
    "Dispatch launch confirmed.",
    ["queue/claimed", "work/assigned"],
  );
}

describe("annotateLaunch", () => {
  test("comments and labels run, no warnings, when both succeed", async () => {
    const { calls, github, queue } = stubs();
    const warnings = await run(github, queue);
    expect(warnings).toEqual([]);
    expect(calls.comments).toHaveLength(1);
    expect(calls.labels).toEqual([["developing-today/code", 2660, ["queue/claimed", "work/assigned"]]]);
    expect(calls.patches).toHaveLength(0);
  });

  test("a failed comment does not skip the label attempt and is warned + noted", async () => {
    const { calls, github, queue } = stubs({ failComment: true });
    const warnings = await run(github, queue);
    expect(calls.labels).toHaveLength(1);
    expect(warnings).toHaveLength(1);
    expect(warnings[0]).toContain("launch annotation failed (launch unaffected)");
    expect(warnings[0]).toContain("issue comment");
    expect(warnings[0]).toContain("github comment 500");
    expect(calls.patches).toHaveLength(1);
    expect(calls.patches[0][0]).toBe(5414);
    expect(calls.patches[0][1]).toContain("github comment 500");
  });

  test("a failed label attempt is warned + noted and the comment still ran", async () => {
    const { calls, github, queue } = stubs({ failLabels: true });
    const warnings = await run(github, queue);
    expect(calls.comments).toHaveLength(1);
    expect(warnings).toHaveLength(1);
    expect(warnings[0]).toContain("issue labels");
    expect(warnings[0]).toContain("github labels 500");
    expect(calls.patches).toHaveLength(1);
    expect(calls.patches[0][1]).toContain("github labels 500");
  });

  test("when both fail, both warnings are returned and the final task note names both", async () => {
    const { calls, github, queue } = stubs({ failComment: true, failLabels: true });
    const warnings = await run(github, queue);
    expect(warnings).toHaveLength(2);
    expect(calls.patches).toHaveLength(2);
    const finalNote = calls.patches[1][1];
    expect(finalNote).toContain("github comment 500");
    expect(finalNote).toContain("github labels 500");
  });

  test("a failing task note does not reject the annotation pass", async () => {
    const { calls, github, queue } = stubs({ failComment: true, failNote: true });
    const warnings = await run(github, queue);
    expect(warnings).toHaveLength(1);
    expect(calls.labels).toHaveLength(1);
  });

  test("the happy path issues no task read", async () => {
    const { calls, github, queue } = stubs();
    const warnings = await run(github, queue);
    expect(warnings).toEqual([]);
    expect(calls.patches).toHaveLength(0);
    expect(calls.gets).toHaveLength(0);
  });

  // #10087 (PR #4231 residual): a lane-ledger failure note already on
  // the task must survive an annotation failure — the note is
  // prepended, not overwritten.
  test("a pre-existing last_error survives a failing annotation (#10087)", async () => {
    const { calls, github, queue } = stubs({
      failComment: true,
      preExisting: "lane assignment failed (launch unaffected): lane rpc 500",
    });
    const warnings = await run(github, queue);
    expect(warnings).toHaveLength(1);
    expect(calls.gets).toEqual([5414]);
    expect(calls.patches).toHaveLength(1);
    const note = calls.patches[0][1];
    expect(note).toContain("lane assignment failed");
    expect(note).toContain("github comment 500");
  });

  test("a pre-existing last_error plus both annotations failing names all three (#10087)", async () => {
    const { calls, github, queue } = stubs({
      failComment: true,
      failLabels: true,
      preExisting: "lane assignment failed (launch unaffected): lane rpc 500",
    });
    const warnings = await run(github, queue);
    expect(warnings).toHaveLength(2);
    expect(calls.patches).toHaveLength(2);
    const finalNote = calls.patches[1][1];
    expect(finalNote).toContain("lane assignment failed");
    expect(finalNote).toContain("github comment 500");
    expect(finalNote).toContain("github labels 500");
  });

  test("a failed task read still notes the annotation failure alone (#10087)", async () => {
    const { calls, github, queue } = stubs({ failComment: true, failGet: true });
    const warnings = await run(github, queue);
    expect(warnings).toHaveLength(1);
    expect(calls.patches).toHaveLength(1);
    expect(calls.patches[0][1]).toContain("github comment 500");
  });
});
