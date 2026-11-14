// Tests for the primary-checkout guard (#9849): parsing of
// `git status --porcelain=v1 --branch`, the verdict rules (healthy
// only on a clean main; the detail names the dirt), and a live run
// against a scratch checkout that is dirtied on purpose — the
// acceptance demonstration in miniature.
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseGitStatusPorcelain, assessPrimaryCheckout, checkPrimaryCheckout } from "../src/primary-checkout";

describe("parseGitStatusPorcelain", () => {
  test("clean main", () => {
    expect(parseGitStatusPorcelain("## main...origin/main\n")).toEqual({ branch: "main", dirtyPaths: [] });
  });

  test("off-main branch with upstream info", () => {
    const s = parseGitStatusPorcelain("## queue/123-fix...origin/queue/123-fix\n M src/a.ts\n");
    expect(s.branch).toBe("queue/123-fix");
    expect(s.dirtyPaths).toEqual(["src/a.ts"]);
  });

  test("detached HEAD has no branch", () => {
    expect(parseGitStatusPorcelain("## HEAD (no branch)\n").branch).toBeNull();
  });

  test("renames name the new path; untracked files count as dirt", () => {
    const s = parseGitStatusPorcelain("## main\nR  old.ts -> new.ts\n?? scratch/\n");
    expect(s.dirtyPaths).toEqual(["new.ts", "scratch/"]);
  });
});

describe("assessPrimaryCheckout", () => {
  test("clean main is up", () => {
    expect(assessPrimaryCheckout({ branch: "main", dirtyPaths: [] }).state).toBe("up");
  });

  test("off main is down and names the branch", () => {
    const v = assessPrimaryCheckout({ branch: "queue/12607-2319", dirtyPaths: [] });
    expect(v.state).toBe("down");
    expect(v.detail).toContain("queue/12607-2319");
  });

  test("dirty main is down and names the paths, truncating past five", () => {
    const v = assessPrimaryCheckout({
      branch: "main",
      dirtyPaths: ["a", "b", "c", "d", "e", "f", "g"],
    });
    expect(v.state).toBe("down");
    expect(v.detail).toContain("7 uncommitted path(s): a, b, c, d, e, +2 more");
  });

  test("detached HEAD is down", () => {
    expect(assessPrimaryCheckout({ branch: null, dirtyPaths: [] }).state).toBe("down");
  });
});

describe("checkPrimaryCheckout against a scratch checkout", () => {
  let dir: string;

  function git(...args: string[]): void {
    const proc = Bun.spawnSync(["git", "-C", dir, ...args], { stdout: "pipe", stderr: "pipe" });
    if (!proc.success) throw new Error(`git ${args.join(" ")} failed: ${new TextDecoder().decode(proc.stderr)}`);
  }

  beforeAll(() => {
    dir = mkdtempSync(join(tmpdir(), "primary-checkout-test-"));
    git("init", "-b", "main");
    git("config", "user.email", "test@example.invalid");
    git("config", "user.name", "guard test");
    writeFileSync(join(dir, "tracked.txt"), "one\n");
    git("add", "tracked.txt");
    git("commit", "-m", "baseline");
  });

  afterAll(() => {
    rmSync(dir, { recursive: true, force: true });
  });

  test("clean main reads up; a dirtied file reads down naming it; a lane branch reads down naming it", () => {
    expect(checkPrimaryCheckout(dir).state).toBe("up");

    writeFileSync(join(dir, "tracked.txt"), "one\ntwo\n");
    const dirty = checkPrimaryCheckout(dir);
    expect(dirty.state).toBe("down");
    expect(dirty.detail).toContain("tracked.txt");

    git("checkout", "-b", "queue/lane-in-primary");
    const offMain = checkPrimaryCheckout(dir);
    expect(offMain.state).toBe("down");
    expect(offMain.detail).toContain("queue/lane-in-primary");
  });

  test("a non-checkout directory reads down, not up", () => {
    expect(checkPrimaryCheckout(join(dir, "does-not-exist")).state).toBe("down");
  });
});
