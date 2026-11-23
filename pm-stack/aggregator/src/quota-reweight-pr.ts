// The PR-only write path for the Stage-3 re-weighting proposal
// generator (issue #12700, slice S6 of epic #12694; design
// doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3 §5, D5).
//
// Acceptance shape: the generator provably cannot merge or push
// to main directly. The proof here is structural, not procedural:
//
// - Every subprocess the generator runs goes through one choke
//   point (runChecked), and every command is validated against an
//   allowlist BEFORE it executes. A command not on the list throws
//   ProposalPrError and never runs.
// - The gh allowlist is exactly two subcommands: `pr create` and
//   `pr list` (the latter for dedupe). There is no `pr merge`, no
//   `pr close`, no `api`, no `issue` — the vocabulary to merge or
//   to edit anything except by opening a PR does not exist here.
// - The git allowlist permits fetch, worktree add, add, commit,
//   and push — and push is validated to target a proposal branch
//   (quota-proposal/…) and nothing else. A push to main (or any
//   other ref) throws before it runs.
// - The config edit itself is an exact find/replace that must
//   match exactly once in the target file, or it throws: the
//   generator cannot produce a fuzzy edit.
//
// The generator runs under the owner's ordinary gh/git identity —
// the enforcement is this code path, stated plainly: within the
// generator, merging is not a forbidden call, it is an
// unrepresentable one. Merge authority stays where it is
// (Felix/Drew); a proposal PR is reviewed and merged (or not) by
// a human through the ordinary path.

import { execFile } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { ReweightProposal } from "./quota-reweight.ts";
import { renderEvidencePack } from "./quota-reweight.ts";

export class ProposalPrError extends Error {}

/** Every proposal branch carries this prefix; pushes to anything
 * else are refused by assertGitAllowed. */
export const PROPOSAL_BRANCH_PREFIX = "quota-proposal/";

export type CommandResult = { code: number; stdout: string; stderr: string };
export type CommandRunner = (cmd: string, args: string[], opts: { cwd: string }) => Promise<CommandResult>;

/** The real runner: execFile, no shell, so an argument can never
 * smuggle a second command. */
export const nodeRunner: CommandRunner = (cmd, args, opts) =>
  new Promise((resolve) => {
    execFile(cmd, args, { cwd: opts.cwd, maxBuffer: 16 * 1024 * 1024 }, (err, stdout, stderr) => {
      resolve({
        code: err ? ((err as { code?: number }).code ?? 1) : 0,
        stdout: String(stdout),
        stderr: String(stderr),
      });
    });
  });

const GH_ALLOWED: ReadonlyArray<readonly [string, string]> = [
  ["pr", "create"],
  ["pr", "list"],
];

/** Validate a gh invocation against the PR-only allowlist. Throws
 * ProposalPrError naming the refused command; returns void when
 * the invocation is one the generator may make. */
export function assertGhAllowed(args: readonly string[]): void {
  const [a, b] = args;
  if (!GH_ALLOWED.some(([x, y]) => x === a && y === b)) {
    throw new ProposalPrError(
      `refused gh invocation 'gh ${args.join(" ")}': the proposal generator's gh surface is ` +
        `only 'pr create' and 'pr list' (design D5, PR-only write path)`,
    );
  }
}

const GIT_ALLOWED_SUBCOMMANDS: ReadonlySet<string> = new Set([
  "fetch",
  "worktree",
  "add",
  "commit",
  "push",
  "rev-parse",
  "status",
]);

/** Validate a git invocation. Push is restricted to proposal
 * branches; worktree is restricted to `add`. */
export function assertGitAllowed(args: readonly string[]): void {
  const [sub, ...rest] = args;
  if (sub === undefined || !GIT_ALLOWED_SUBCOMMANDS.has(sub)) {
    throw new ProposalPrError(
      `refused git invocation 'git ${args.join(" ")}': subcommand '${sub ?? ""}' is not in the ` +
        `proposal generator's allowlist`,
    );
  }
  if (sub === "worktree" && rest[0] !== "add") {
    throw new ProposalPrError(`refused git invocation 'git ${args.join(" ")}': only 'worktree add' is allowed`);
  }
  if (sub === "push") {
    const target = rest[rest.length - 1] ?? "";
    if (rest[0] !== "origin" || !target.startsWith(PROPOSAL_BRANCH_PREFIX)) {
      throw new ProposalPrError(
        `refused git push 'git ${args.join(" ")}': the proposal generator may push only to ` +
          `${PROPOSAL_BRANCH_PREFIX}… branches — never to main or any other ref (design D5)`,
      );
    }
  }
}

/** Run one command through the choke point: validate, then run,
 * then require success. */
async function runChecked(runner: CommandRunner, cmd: string, args: string[], cwd: string): Promise<CommandResult> {
  if (cmd.endsWith("gh") || cmd === "gh") assertGhAllowed(args);
  else if (cmd.endsWith("git") || cmd === "git") assertGitAllowed(args);
  else throw new ProposalPrError(`refused command '${cmd}': the generator runs only gh and git`);
  const res = await runner(cmd, args, { cwd });
  if (res.code !== 0) {
    throw new ProposalPrError(
      `'${cmd} ${args.join(" ")}' failed (exit ${res.code}): ${res.stderr.trim() || res.stdout.trim()}`,
    );
  }
  return res;
}

/** The branch a proposal opens under. The key's ':' separators
 * become path segments and '->' becomes '-to-', so the branch is
 * a valid ref and the key is recoverable from it. */
export function proposalBranch(key: string): string {
  const slug = key
    .replace(/:/g, "/")
    .replace(/->/g, "-to-")
    .replace(/[^a-zA-Z0-9._/-]/g, "-");
  return `${PROPOSAL_BRANCH_PREFIX}${slug}`;
}

/** Apply a proposal's edit to file contents: the find text must
 * occur exactly once, or the edit is refused — the generator
 * never produces a fuzzy or partial edit. */
export function applyProposalEdit(content: string, proposal: ReweightProposal): string {
  const { find, replace } = proposal.target;
  const occurrences = content.split(find).length - 1;
  if (occurrences !== 1) {
    throw new ProposalPrError(
      `edit anchor for '${proposal.key}' occurs ${occurrences} times in ${proposal.target.file} ` +
        `(expected exactly 1); refusing to edit`,
    );
  }
  return content.replace(find, replace);
}

/** The URL of an already-open PR for this proposal's branch, if
 * one exists (dedupe: the generator never opens a second PR for
 * the same proposal key). */
export async function findOpenProposalPr(args: {
  runner: CommandRunner;
  ghPath: string;
  repoRoot: string;
  branch: string;
}): Promise<string | null> {
  const res = await runChecked(
    args.runner,
    args.ghPath,
    ["pr", "list", "--state", "open", "--head", args.branch, "--json", "url", "--limit", "1"],
    args.repoRoot,
  );
  try {
    const rows = JSON.parse(res.stdout) as { url?: string }[];
    return rows[0]?.url ?? null;
  } catch {
    throw new ProposalPrError(`could not parse 'gh pr list' output for ${args.branch}: ${res.stdout.slice(0, 200)}`);
  }
}

/**
 * Open the proposal PR: branch from origin/main in a scratch
 * worktree, apply the exact edit, commit, push the proposal
 * branch, and `gh pr create` with the evidence pack as the body.
 * Returns the PR URL. This function contains no merge step and
 * cannot be made to perform one — see the allowlist above.
 */
export async function openProposalPr(args: {
  runner: CommandRunner;
  ghPath: string;
  gitPath?: string;
  repoRoot: string;
  scratchDir: string;
  proposal: ReweightProposal;
  base?: string;
}): Promise<{ branch: string; url: string }> {
  const git = args.gitPath ?? "git";
  const base = args.base ?? "main";
  const branch = proposalBranch(args.proposal.key);
  const existing = await findOpenProposalPr({
    runner: args.runner,
    ghPath: args.ghPath,
    repoRoot: args.repoRoot,
    branch,
  });
  if (existing) return { branch, url: existing };

  await runChecked(args.runner, git, ["fetch", "origin", base], args.repoRoot);
  const wtDir = join(args.scratchDir, branch.replace(/\//g, "-"));
  await mkdir(args.scratchDir, { recursive: true });
  await runChecked(args.runner, git, ["worktree", "add", "-b", branch, wtDir, `origin/${base}`], args.repoRoot);
  const targetPath = join(wtDir, args.proposal.target.file);
  const content = await readFile(targetPath, "utf8");
  await writeFile(targetPath, applyProposalEdit(content, args.proposal));
  const packPath = join(args.scratchDir, `${branch.replace(/\//g, "-")}.md`);
  await writeFile(packPath, renderEvidencePack(args.proposal));
  await runChecked(args.runner, git, ["add", "--", args.proposal.target.file], wtDir);
  await runChecked(
    args.runner,
    git,
    [
      "commit",
      "-m",
      args.proposal.title,
      "-m",
      `Generated by the Stage-3 re-weighting proposal generator (S6, #12700; design D5). ` +
        `Proposal key: ${args.proposal.key}. The full evidence pack is the PR body.`,
    ],
    wtDir,
  );
  await runChecked(args.runner, git, ["push", "origin", branch], wtDir);
  const pr = await runChecked(
    args.runner,
    args.ghPath,
    ["pr", "create", "--base", base, "--head", branch, "--title", args.proposal.title, "--body-file", packPath],
    args.repoRoot,
  );
  const url = pr.stdout.trim().split("\n").filter(Boolean).pop() ?? "";
  if (!url.startsWith("http")) {
    throw new ProposalPrError(`'gh pr create' did not return a PR URL for ${branch}: ${pr.stdout.slice(0, 200)}`);
  }
  return { branch, url };
}
