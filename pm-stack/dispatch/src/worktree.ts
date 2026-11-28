// Dispatch worktrees (code#12213). A dispatched session used to run in
// the repo's primary clone (~/git/code for developing-today/code). That
// clone is the deploy checkout pm-git-sync keeps on clean main, so a
// session that branched, committed or edited there stalled the laptop's
// code sync (2026-11-21: "Scheduled Bolt" and "Scheduled Palette" left
// ~/git/code on their PR branches). Each launch into a primary clone
// now gets its own detached worktree at the remote default branch
// instead, and the primary is never checked out, branched or edited.

import { existsSync } from "node:fs";
import { join, relative } from "node:path";

export class WorktreeError extends Error {}

export interface GitResult {
  code: number;
  stdout: string;
  stderr: string;
}

export type GitRunner = (args: string[]) => Promise<GitResult>;

export const runGit: GitRunner = async (args) => {
  const proc = Bun.spawn(["git", ...args], {
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, GIT_TERMINAL_PROMPT: "0" },
  });
  const [stdout, stderr, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ]);
  return { code, stdout, stderr };
};

/** Worktree directory name for one dispatched task: the repo name plus
 * the task id, so two launches never share a tree and the path says
 * which task it belongs to. */
export function dispatchWorktreeName(repo: string, taskId: number | string): string {
  const name = (repo.split("/").pop() ?? "repo").replace(/[^A-Za-z0-9._-]/g, "_");
  return `dispatch-${name}-${String(taskId).replace(/[^A-Za-z0-9._-]/g, "_")}`;
}

/** Create a detached worktree of `primary` at its remote default branch
 * and return the directory the session should run in: the worktree,
 * or the same subdirectory inside it when the caller asked for one
 * inside the primary. The fetch is best effort (a failed fetch still
 * builds from the last fetched remote ref); every other failure throws,
 * which fails the launch before any session exists, so a session never
 * falls back to running in the primary clone. */
export async function createDispatchWorktree(args: {
  primary: string;
  requestedDirectory: string;
  root: string;
  repo: string;
  taskId: number | string;
  git?: GitRunner;
}): Promise<{ directory: string; worktree: string; base: string }> {
  const git = args.git ?? runGit;
  const worktree = join(args.root, dispatchWorktreeName(args.repo, args.taskId));
  if (existsSync(worktree)) {
    throw new WorktreeError(`dispatch worktree ${worktree} already exists; refusing to reuse it`);
  }
  await git(["-C", args.primary, "fetch", "--quiet", "origin"]);
  let base = "";
  for (const ref of ["refs/remotes/origin/HEAD", "refs/remotes/origin/main", "refs/remotes/origin/master"]) {
    const r = await git(["-C", args.primary, "rev-parse", "--verify", "--quiet", `${ref}^{commit}`]);
    if (r.code === 0 && r.stdout.trim()) {
      base = r.stdout.trim();
      break;
    }
  }
  if (!base) {
    throw new WorktreeError(`no remote default branch found in ${args.primary}`);
  }
  const add = await git(["-C", args.primary, "worktree", "add", "--quiet", "--detach", worktree, base]);
  if (add.code !== 0) {
    throw new WorktreeError(`git worktree add ${worktree} failed: ${add.stderr.trim().slice(0, 300)}`);
  }
  const rel = relative(args.primary, args.requestedDirectory);
  const directory = rel && !rel.startsWith("..") ? join(worktree, rel) : worktree;
  return { directory, worktree, base };
}

/** The lines added to a dispatched prompt so the session knows where it
 * is and that the primary clone is off limits. */
export function worktreeNotice(worktree: string, primary: string): string {
  return (
    `Worktree: you are running in ${worktree}, a detached worktree created for this task at the remote default branch. ` +
    `Make every branch, edit and commit here. Never check out, branch, edit or commit in ${primary}: ` +
    `it is the deploy clone kept on clean main, and changes there stall the laptop's code sync.`
  );
}
