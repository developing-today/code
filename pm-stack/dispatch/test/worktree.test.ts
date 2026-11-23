// Dispatch worktrees (code#12213): a launch into a primary clone gets
// its own detached worktree at the remote default branch, and the
// primary's branch, HEAD and working tree are left exactly as they were.
import { afterAll, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { createDispatchWorktree, dispatchWorktreeName, worktreeNotice, WorktreeError } from "../src/worktree.ts";
import {
  cleanupDispatchWorktree,
  isProcessUsingPath,
  WorktreeRegistry,
  type ProcessOccupancyProbe,
  type WorktreeRecord,
} from "../src/worktree_cleanup.ts";

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
      createDispatchWorktree({
        primary,
        requestedDirectory: primary,
        root,
        repo: "developing-today/code",
        taskId: 20901,
      }),
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

  test("worktree sweep interval fits setInterval's supported millisecond range", () => {
    expect(loadConfig({ PM_DISPATCH_WORKTREE_SWEEP_SECONDS: "2147483" }).worktreeSweepSeconds).toBe(2147483);
    expect(() => loadConfig({ PM_DISPATCH_WORKTREE_SWEEP_SECONDS: "2147484" })).toThrow("2147483");
  });
});

describe("process occupancy permissions", () => {
  test("ignores unreadable root-owned system cwd links but detects user processes in the tree", () => {
    const probe: ProcessOccupancyProbe = {
      listPids: () => ["1", "42"],
      ownerUid: (pid) => (pid === "1" ? 0 : 1000),
      cwd: (pid) => {
        if (pid === "1") throw Object.assign(new Error("permission denied"), { code: "EACCES" });
        return primary;
      },
    };
    expect(isProcessUsingPath(primary, probe)).toBe(true);
  });

  test("does not let root's unreadable cwd block otherwise unoccupied worktrees", () => {
    const probe: ProcessOccupancyProbe = {
      listPids: () => ["1", "42"],
      ownerUid: (pid) => (pid === "1" ? 0 : 1000),
      cwd: (pid) => {
        if (pid === "1") throw Object.assign(new Error("permission denied"), { code: "EACCES" });
        return join(tmp, "outside");
      },
    };
    expect(isProcessUsingPath(primary, probe)).toBe(false);
  });

  test("retains when a non-root process cwd cannot be inspected", () => {
    const probe: ProcessOccupancyProbe = {
      listPids: () => ["42"],
      ownerUid: () => 1000,
      cwd: () => {
        throw Object.assign(new Error("permission denied"), { code: "EACCES" });
      },
    };
    expect(isProcessUsingPath(primary, probe)).toBe(true);
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
      getSession: async () => {
        throw new Error("server unavailable");
      },
      listSessions: emptyList,
      isOccupied: unoccupied,
    });
    expect(unknownResult.removed).toBe(false);
    expect(unknownResult.reason).toContain("state unknown");
    expect(existsSync(unknown.worktree)).toBe(true);
  });

  test("retains an unbound session record without querying the session server", async () => {
    const existing = await registeredTree();
    const record = await registry.add({ ...existing, sessionId: null });
    let queried = false;
    const result = await cleanupDispatchWorktree({
      record,
      registry,
      getSession: async () => {
        queried = true;
        return null;
      },
      listSessions: async () => {
        queried = true;
        return [];
      },
      isOccupied: unoccupied,
    });
    expect(result.removed).toBe(false);
    expect(result.worktree).toBe(record.worktree);
    expect(result.reason).toContain("unbound or uncertain");
    expect(queried).toBe(false);
    expect(existsSync(record.worktree)).toBe(true);
  });

  test("retains a session list containing an entry without a usable directory", async () => {
    const record = await registeredTree();
    const result = await cleanupDispatchWorktree({
      record,
      registry,
      getSession: goneSession,
      listSessions: async () => [{ id: "session-without-directory" }],
      isOccupied: unoccupied,
    });
    expect(result.removed).toBe(false);
    expect(result.reason).toContain("without a usable directory");
    expect(existsSync(record.worktree)).toBe(true);
  });

  test("a corrupt registry produces one retained sweep outcome without touching sessions", async () => {
    const file = join(tmp, "corrupt-registry", "worktrees.json");
    mkdirSync(join(tmp, "corrupt-registry"), { recursive: true });
    writeFileSync(file, "{broken json\n");
    const brokenRegistry = new WorktreeRegistry(file);
    let queried = false;
    const { sweepDispatchWorktrees } = await import("../src/worktree_cleanup.ts");
    const results = await sweepDispatchWorktrees({
      registry: brokenRegistry,
      getSession: async () => {
        queried = true;
        return null;
      },
      listSessions: async () => {
        queried = true;
        return [];
      },
    });
    expect(results).toHaveLength(1);
    expect(results[0].removed).toBe(false);
    expect(results[0].reason).toContain("not valid JSON");
    expect(queried).toBe(false);
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

  test("retains ignored files instead of deleting them with the worktree", async () => {
    const ignored = await registeredTree();
    writeFileSync(join(ignored.worktree, ".gitignore"), ".env\n");
    writeFileSync(join(ignored.worktree, ".env"), "keep this secret\n");
    const result = await cleanupDispatchWorktree({
      record: ignored,
      registry,
      getSession: goneSession,
      listSessions: emptyList,
      isOccupied: unoccupied,
    });
    expect(result.removed).toBe(false);
    expect(result.reason).toContain("uncommitted or untracked work");
    expect(existsSync(join(ignored.worktree, ".env"))).toBe(true);
  });

  test("only one concurrent cleanup can remove a registered tree", async () => {
    const record = await registeredTree();
    let unblock!: () => void;
    const paused = new Promise<void>((resolve) => {
      unblock = resolve;
    });
    let entered!: () => void;
    const inList = new Promise<void>((resolve) => {
      entered = resolve;
    });
    const first = cleanupDispatchWorktree({
      record,
      registry,
      getSession: goneSession,
      listSessions: async () => {
        entered();
        await paused;
        return [];
      },
      isOccupied: unoccupied,
    });
    await inList;
    const second = await cleanupDispatchWorktree({
      record,
      registry,
      getSession: goneSession,
      listSessions: emptyList,
      isOccupied: unoccupied,
    });
    expect(second.removed).toBe(false);
    expect(second.reason).toContain("already in progress");
    unblock();
    const firstResult = await first;
    expect(firstResult.removed).toBe(true);
    expect(existsSync(record.worktree)).toBe(false);
  });

  test("holds the worktree HEAD lock through git worktree removal", async () => {
    const record = await registeredTree();
    const { runGit } = await import("../src/worktree.ts");
    let sawLockDuringRemove = false;
    let blockedExternalHeadMove = false;
    const result = await cleanupDispatchWorktree({
      record,
      registry,
      getSession: goneSession,
      listSessions: emptyList,
      isOccupied: unoccupied,
      git: async (args) => {
        if (args.includes("status")) {
          const attempt = Bun.spawnSync(["git", "-C", record.worktree, "reset", "--hard", "HEAD"], { env });
          blockedExternalHeadMove = attempt.exitCode !== 0;
        }
        if (args.includes("remove")) {
          const head = git("-C", record.worktree, "rev-parse", "--git-path", "HEAD");
          sawLockDuringRemove = existsSync(`${head}.lock`);
        }
        return runGit(args);
      },
    });
    expect(result.removed).toBe(true);
    expect(blockedExternalHeadMove).toBe(true);
    expect(sawLockDuringRemove).toBe(true);
  });

  test("retains an existing HEAD lock without deleting another process's lock", async () => {
    const record = await registeredTree();
    const head = git("-C", record.worktree, "rev-parse", "--git-path", "HEAD");
    const lock = `${head}.lock`;
    writeFileSync(lock, "owned elsewhere\n");
    const result = await cleanupDispatchWorktree({
      record,
      registry,
      getSession: goneSession,
      listSessions: emptyList,
      isOccupied: unoccupied,
    });
    expect(result.removed).toBe(false);
    expect(result.reason).toContain("cannot lock worktree HEAD");
    expect(await Bun.file(lock).text()).toBe("owned elsewhere\n");
    expect(existsSync(record.worktree)).toBe(true);
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
      git: async (args) =>
        args.includes("remove")
          ? { code: 1, stdout: "", stderr: "injected removal failure" }
          : await (await import("../src/worktree.ts")).runGit(args),
    });
    expect(failed.removed).toBe(false);
    expect(failed.reason).toContain("injected removal failure");
    expect(existsSync(broken.worktree)).toBe(true);
  });
});
