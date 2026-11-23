import { afterEach, describe, expect, test } from "bun:test";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ReweightProposal } from "./quota-reweight.ts";
import {
  ProposalPrError,
  applyProposalEdit,
  assertGhAllowed,
  assertGitAllowed,
  openProposalPr,
  proposalBranch,
  type CommandResult,
  type CommandRunner,
} from "./quota-reweight-pr.ts";

// Oracle: design D5's hard rule (#1177, via #12700's acceptance):
// the generator has no write path to configuration except the PR
// and no merge rights. These tests pin the structural form of
// that rule — the allowlist refuses merge/push-to-main BEFORE any
// command runs, and a full openProposalPr drive over a fake runner
// performs exactly the branch+PR sequence and nothing else.

const MARGIN_PROPOSAL: ReweightProposal = {
  key: "caution-margin:NEAR_PCT:85->90",
  proposalClass: "caution-margin",
  title: "proposal(quota-plane): widen the caution margin NEAR_PCT 85 → 90",
  target: {
    description: "the caution margin NEAR_PCT",
    file: "pm-stack/aggregator/src/derive.ts",
    find: "export const NEAR_PCT = 85;",
    replace: "export const NEAR_PCT = 90;",
    currentValue: "85",
    proposedValue: "90",
  },
  rationale: "test rationale",
  evidence: {
    ledgerGeneratedAt: "2026-11-23T00:00:00.000Z",
    ledgerStore: { records: 1, pctRecords: 1, fromIso: null, toIso: null },
    ledgerModel: "test-model",
    rows: [],
    forecastSummary: "no forecast file supplied to this run",
    shadowSummary: "no shadow record supplied to this run",
  },
  expectedEffect: { statement: "s", metric: "m", scoredBy: "S7" },
};

describe("gh allowlist", () => {
  test("permits only pr create and pr list", () => {
    expect(() => assertGhAllowed(["pr", "create", "--title", "x"])).not.toThrow();
    expect(() => assertGhAllowed(["pr", "list", "--state", "open"])).not.toThrow();
  });
  test("refuses merge, close, edit, api, and issue commands", () => {
    for (const args of [
      ["pr", "merge", "123"],
      ["pr", "close", "123"],
      ["pr", "edit", "123"],
      ["api", "repos/x/y/pulls/1/merge"],
      ["issue", "create", "--title", "x"],
    ]) {
      expect(() => assertGhAllowed(args)).toThrow(ProposalPrError);
    }
  });
});

describe("git allowlist", () => {
  test("permits fetch, worktree add, add, commit, and push to a proposal branch", () => {
    expect(() => assertGitAllowed(["fetch", "origin", "main"])).not.toThrow();
    expect(() =>
      assertGitAllowed(["worktree", "add", "-b", "quota-proposal/x", "/tmp/wt", "origin/main"]),
    ).not.toThrow();
    expect(() => assertGitAllowed(["add", "--", "file.ts"])).not.toThrow();
    expect(() => assertGitAllowed(["commit", "-m", "msg"])).not.toThrow();
    expect(() => assertGitAllowed(["push", "origin", "quota-proposal/caution-margin/NEAR_PCT/85-to-90"])).not.toThrow();
  });
  test("refuses push to main or any non-proposal ref, and refuses merge/rebase", () => {
    for (const args of [
      ["push", "origin", "main"],
      ["push", "origin", "felix/some-branch"],
      ["push", "--force", "origin", "quota-proposal/x"],
      ["merge", "origin/main"],
      ["rebase", "main"],
      ["worktree", "remove", "/tmp/wt"],
    ]) {
      expect(() => assertGitAllowed(args)).toThrow(ProposalPrError);
    }
  });
});

describe("edit anchoring", () => {
  test("applies an exactly-once anchor", () => {
    expect(applyProposalEdit("a\nexport const NEAR_PCT = 85;\nb", MARGIN_PROPOSAL)).toContain("NEAR_PCT = 90;");
  });
  test("refuses zero or multiple occurrences", () => {
    expect(() => applyProposalEdit("nothing here", MARGIN_PROPOSAL)).toThrow(ProposalPrError);
    expect(() =>
      applyProposalEdit("export const NEAR_PCT = 85;\nexport const NEAR_PCT = 85;", MARGIN_PROPOSAL),
    ).toThrow(ProposalPrError);
  });
});

describe("openProposalPr over a fake runner", () => {
  let scratch: string | null = null;
  afterEach(async () => {
    if (scratch) await rm(scratch, { recursive: true, force: true });
    scratch = null;
  });

  function fakeRunner(calls: { cmd: string; args: string[] }[], prListStdout = "[]"): CommandRunner {
    return async (cmd, args): Promise<CommandResult> => {
      calls.push({ cmd, args: [...args] });
      if (cmd.endsWith("gh") && args[0] === "pr" && args[1] === "list")
        return { code: 0, stdout: prListStdout, stderr: "" };
      if (cmd.endsWith("gh") && args[0] === "pr" && args[1] === "create")
        return { code: 0, stdout: "https://github.com/developing-today/code/pull/99999\n", stderr: "" };
      return { code: 0, stdout: "", stderr: "" };
    };
  }

  test("performs exactly the branch + edit + push + pr-create sequence, with no merge anywhere", async () => {
    scratch = await mkdtemp(join(tmpdir(), "reweight-pr-"));
    const repoRoot = join(scratch, "repo");
    const branch = proposalBranch(MARGIN_PROPOSAL.key);
    const wtDir = join(scratch, "out", branch.replace(/\//g, "-"));
    await mkdir(join(wtDir, "pm-stack", "aggregator", "src"), { recursive: true });
    await writeFile(join(wtDir, "pm-stack", "aggregator", "src", "derive.ts"), "export const NEAR_PCT = 85;\n");

    const calls: { cmd: string; args: string[] }[] = [];
    const res = await openProposalPr({
      runner: fakeRunner(calls),
      ghPath: "/usr/bin/gh",
      repoRoot,
      scratchDir: join(scratch, "out"),
      proposal: MARGIN_PROPOSAL,
    });
    expect(res.url).toBe("https://github.com/developing-today/code/pull/99999");
    expect(res.branch).toBe("quota-proposal/caution-margin/NEAR_PCT/85-to-90");
    // the edit landed in the scratch worktree
    const edited = await readFile(join(wtDir, "pm-stack", "aggregator", "src", "derive.ts"), "utf8");
    expect(edited).toContain("NEAR_PCT = 90;");
    // the whole drive used only allowlisted shapes
    for (const c of calls) {
      if (c.cmd.endsWith("gh")) expect(() => assertGhAllowed(c.args)).not.toThrow();
      else expect(() => assertGitAllowed(c.args)).not.toThrow();
      expect(c.args).not.toContain("merge");
    }
    const pushes = calls.filter((c) => c.args[0] === "push");
    expect(pushes).toHaveLength(1);
    expect(pushes[0]?.args).toEqual(["push", "origin", branch]);
    const creates = calls.filter((c) => c.cmd.endsWith("gh") && c.args[1] === "create");
    expect(creates).toHaveLength(1);
  });

  test("dedupes: an already-open PR for the branch is returned, nothing else runs", async () => {
    scratch = await mkdtemp(join(tmpdir(), "reweight-pr-"));
    const calls: { cmd: string; args: string[] }[] = [];
    const res = await openProposalPr({
      runner: fakeRunner(calls, JSON.stringify([{ url: "https://github.com/developing-today/code/pull/88888" }])),
      ghPath: "/usr/bin/gh",
      repoRoot: join(scratch, "repo"),
      scratchDir: join(scratch, "out"),
      proposal: MARGIN_PROPOSAL,
    });
    expect(res.url).toBe("https://github.com/developing-today/code/pull/88888");
    expect(calls).toHaveLength(1); // the pr list only
  });
});
