// Dispatch worktrees (code#12213): a launch into a primary clone gets
// its own detached worktree at the remote default branch, and the
// primary's branch, HEAD and working tree are left exactly as they were.
import { afterAll, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { createDispatchWorktree, dispatchWorktreeName, worktreeNotice, WorktreeError } from "../src/worktree.ts";
import { cleanupDispatchWorktree, WorktreeRegistry, type WorktreeRecord } from "../src/worktree_cleanup.ts";

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

describe("dispatch worktree cleanup (#12217)", () => {
  const root = join(tmp, "cleanup-fleet");
  const registry = new WorktreeRegistry(join(tmp, "state", "worktrees.json"));
  let nextId = 1;

  async function registeredTree(): Promise<WorktreeRecord> {
    const id = 22000 + nextId++;
    const wt = await createDispatchWorktree({
      primary,
      requestedDirectory: primary,
      root,
      repo: "developing-today/code",
      taskId: id,
    });
    return registry.add({
      taskId: id,
      sessionId: `ses_${id}`,
      repo: "developing-today/code",
      primary,
      worktree: wt.worktree,
      base: wt.base,
      createdAt: new Date().toISOString(),
    });
  }

  const emptyList = async () => [];
  const goneSession = async () => null;
  const unoccupied = () => false;

  test("removes a clean tree only after the session is confirmed gone; repeat is idempotent", async () => {
    const record = await registeredTree();
    const result = await cleanupDispatchWorktree({
      record,
      registry,
      getSession: goneSession,
      listSessions: emptyList,
      isOccupied: unoccupied,
    });
    expect(result.removed).toBe(true);
    expect(existsSync(record.worktree)).toBe(false);
    const again = await cleanupDispatchWorktree({
      record: { ...record, state: "removed" },
      registry,
      getSession: goneSession,
      listSessions: emptyList,
      isOccupied: unoccupied,
    });
    expect(again.removed).toBe(true);
    expect(again.reason).toBe("already removed");
  });

  test("retains an active session and an ambiguous session lookup", async () => {
    const active = await registeredTree();
    const activeResult = await cleanupDispatchWorktree({
      record: active,
      registry,
      getSession: async () => ({ id: active.sessionId, directory: active.worktree }),
      listSessions: emptyList,
      isOccupied: unoccupied,
    });
    expect(activeResult.removed).toBe(false);
    expect(existsSync(active.worktree)).toBe(true);

    const unknown = await registeredTree();
    const unknownResult = await cleanupDispatchWorktree({
      record: unknown,
      registry,
      getSession: async () => { throw new Error("server unavailable"); },
      listSessions: emptyList,
      isOccupied: unoccupied,
    });
    expect(unknownResult.removed).toBe(false);
    expect(unknownResult.reason).toContain("state unknown");
    expect(existsSync(unknown.worktree)).toBe(true);
  });

  test("retains uncommitted, untracked, and unpublished work", async () => {
    const dirty = await registeredTree();
    writeFileSync(join(dirty.worktree, "untracked.txt"), "keep me\n");
    const dirtyResult = await cleanupDispatchWorktree({
      record: dirty,
      registry,
      getSession: goneSession,
      listSessions: emptyList,
      isOccupied: unoccupied,
    });
    expect(dirtyResult.removed).toBe(false);
    expect(existsSync(join(dirty.worktree, "untracked.txt"))).toBe(true);

    const unpublished = await registeredTree();
    writeFileSync(join(unpublished.worktree, "commit.txt"), "keep commit\n");
    git("-C", unpublished.worktree, "add", "commit.txt");
    git("-C", unpublished.worktree, "commit", "-qm", "unpublished");
    const unpublishedResult = await cleanupDispatchWorktree({
      record: unpublished,
      registry,
      getSession: goneSession,
      listSessions: emptyList,
      isOccupied: unoccupied,
    });
    expect(unpublishedResult.removed).toBe(false);
    expect(existsSync(unpublished.worktree)).toBe(true);
    expect(unpublishedResult.reason).toContain("HEAD is not the recorded base");
  });

  test("retains on an occupied tree and reports git removal failure without throwing", async () => {
    const occupied = await registeredTree();
    const occupiedResult = await cleanupDispatchWorktree({
      record: occupied,
      registry,
      getSession: goneSession,
      listSessions: emptyList,
      isOccupied: () => true,
    });
    expect(occupiedResult.removed).toBe(false);
    expect(occupiedResult.reason).toContain("process has its cwd");

    const broken = await registeredTree();
    const failed = await cleanupDispatchWorktree({
      record: broken,
      registry,
      getSession: goneSession,
      listSessions: emptyList,
      isOccupied: unoccupied,
      git: async (args) => args.includes("remove")
        ? { code: 1, stdout: "", stderr: "injected removal failure" }
        : await (await import("../src/worktree.ts")).runGit(args),
    });
    expect(failed.removed).toBe(false);
    expect(failed.reason).toContain("injected removal failure");
    expect(existsSync(broken.worktree)).toBe(true);
  });
});
