// Dispatch worktrees (code#12213): a launch into a primary clone gets
// its own detached worktree at the remote default branch, and the
// primary's branch, HEAD and working tree are left exactly as they were.
import { afterAll, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { createDispatchWorktree, dispatchWorktreeName, worktreeNotice, WorktreeError } from "../src/worktree.ts";

const tmp = mkdtempSync(join(tmpdir(), "dispatch-worktree-"));
afterAll(() => rmSync(tmp, { recursive: true, force: true }));

const env = {
  ...process.env,
  GIT_AUTHOR_NAME: "t",
  GIT_AUTHOR_EMAIL: "t@example.invalid",
  GIT_COMMITTER_NAME: "t",
  GIT_COMMITTER_EMAIL: "t@example.invalid",
  GIT_CONFIG_GLOBAL: "/dev/null",
  GIT_CONFIG_NOSYSTEM: "1",
};
function git(...args: string[]): string {
  const r = Bun.spawnSync(["git", ...args], { env });
  if (r.exitCode !== 0) throw new Error(`git ${args.join(" ")}: ${r.stderr.toString()}`);
  return r.stdout.toString().trim();
}

// origin with two commits; the primary clone sits one commit behind,
// on a local branch, with an uncommitted edit.
const origin = join(tmp, "origin.git");
const seed = join(tmp, "seed");
const primary = join(tmp, "primary");
git("init", "-q", "--bare", "-b", "main", origin);
git("clone", "-q", origin, seed);
mkdirSync(join(seed, "sub"));
writeFileSync(join(seed, "sub", "a.txt"), "one\n");
git("-C", seed, "add", ".");
git("-C", seed, "commit", "-qm", "one");
git("-C", seed, "push", "-q", "origin", "main");
git("clone", "-q", origin, primary);
writeFileSync(join(seed, "sub", "a.txt"), "two\n");
git("-C", seed, "commit", "-qam", "two");
git("-C", seed, "push", "-q", "origin", "main");
const originHead = git("-C", seed, "rev-parse", "HEAD");
git("-C", primary, "checkout", "-q", "-b", "local-work");
writeFileSync(join(primary, "sub", "a.txt"), "local edit\n");
const primaryHead = git("-C", primary, "rev-parse", "HEAD");

describe("createDispatchWorktree", () => {
  const root = join(tmp, "fleet");

  test("builds a detached worktree at the fetched remote default branch", async () => {
    const wt = await createDispatchWorktree({
      primary,
      requestedDirectory: primary,
      root,
      repo: "developing-today/code",
      taskId: 20901,
    });
    expect(wt.worktree).toBe(join(root, "dispatch-code-20901"));
    expect(wt.directory).toBe(wt.worktree);
    expect(wt.base).toBe(originHead);
    expect(git("-C", wt.worktree, "rev-parse", "HEAD")).toBe(originHead);
    expect(git("-C", wt.worktree, "branch", "--show-current")).toBe("");
    expect(git("-C", wt.worktree, "status", "--porcelain")).toBe("");
  });

  test("leaves the primary clone untouched", () => {
    expect(git("-C", primary, "branch", "--show-current")).toBe("local-work");
    expect(git("-C", primary, "rev-parse", "HEAD")).toBe(primaryHead);
    expect(git("-C", primary, "status", "--porcelain")).toBe("M sub/a.txt");
  });

  test("maps a requested subdirectory of the primary into the worktree", async () => {
    const wt = await createDispatchWorktree({
      primary,
      requestedDirectory: join(primary, "sub"),
      root,
      repo: "developing-today/code",
      taskId: 20902,
    });
    expect(wt.directory).toBe(join(wt.worktree, "sub"));
    expect(existsSync(join(wt.directory, "a.txt"))).toBe(true);
  });

  test("refuses to reuse an existing worktree path", async () => {
    await expect(
      createDispatchWorktree({ primary, requestedDirectory: primary, root, repo: "developing-today/code", taskId: 20901 }),
    ).rejects.toBeInstanceOf(WorktreeError);
  });

  test("fails, rather than falling back, when the primary is not a git clone", async () => {
    const notRepo = join(tmp, "not-a-repo");
    mkdirSync(notRepo);
    await expect(
      createDispatchWorktree({ primary: notRepo, requestedDirectory: notRepo, root, repo: "x/y", taskId: 1 }),
    ).rejects.toBeInstanceOf(WorktreeError);
  });
});

describe("helpers and config", () => {
  test("worktree names are path-safe", () => {
    expect(dispatchWorktreeName("dezren39/darwin-nix", 7)).toBe("dispatch-darwin-nix-7");
    expect(dispatchWorktreeName("a/b c", "1/2")).toBe("dispatch-b_c-1_2");
  });

  test("the notice names the worktree and puts the primary off limits", () => {
    const n = worktreeNotice("/w/dispatch-code-1", "/home/user/git/code");
    expect(n).toContain("/w/dispatch-code-1");
    expect(n).toContain("Never check out, branch, edit or commit in /home/user/git/code");
  });

  test("PM_DISPATCH_WORKTREE_ROOT defaults to ~/git/fleet, takes a path, and turns off with off", () => {
    expect(loadConfig({}).worktreeRoot).toMatch(/\/git\/fleet$/);
    expect(loadConfig({ PM_DISPATCH_WORKTREE_ROOT: "/srv/wt" }).worktreeRoot).toBe("/srv/wt");
    expect(loadConfig({ PM_DISPATCH_WORKTREE_ROOT: "off" }).worktreeRoot).toBeNull();
  });
});
