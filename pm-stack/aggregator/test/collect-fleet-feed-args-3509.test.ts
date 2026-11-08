// Pinning test for #3509: collect-fleet-feed must strictly validate its
// operator arguments BEFORE any collection begins. A trailing --out used
// to crash at dirname(undefined) with a raw TypeError (exit 1),
// --out --weird wrote a file named --weird (exit 0), and a typo'd --otu
// silently wrote the default feed path (exit 0). Per the #3352
// unknown-means-stop contract, the only accepted forms are no arguments
// or one documented separated --out <path>; everything else is a usage
// error (exit 2) naming the offending token, with no collection or
// file-write side effect. A valid explicit --out and the no-argument
// default keep working.
import { describe, expect, test } from "bun:test";
import { mkdtemp, readdir, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/collect-fleet-feed.ts");

async function runScript(
  home: string,
  cwd: string,
  args: string[],
): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    cwd,
    env: {
      PATH: process.env.PATH ?? "/run/current-system/sw/bin:/usr/bin:/bin",
      HOME: home,
      TMPDIR: home,
      FLEET_FEED_OUT: join(home, "feed.json"),
    },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

async function exists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

describe("collect-fleet-feed argument validation (#3509)", () => {
  test("invalid arguments exit 2 naming the token, with no collection or file writes", async () => {
    const badArgSets: { args: string[]; names: string }[] = [
      { args: ["--out"], names: "--out" },
      { args: ["--out", ""], names: "--out" },
      { args: ["--out", "--weird"], names: "--out" },
      { args: ["--otu", "requested.json"], names: "--otu" },
      { args: ["requested.json"], names: "requested.json" },
      { args: ["--out=requested.json"], names: "--out=requested.json" },
      { args: ["--out", "a.json", "--out", "b.json"], names: "--out" },
      { args: ["--out", "a.json", "extra"], names: "extra" },
    ];
    for (const { args, names } of badArgSets) {
      const home = await mkdtemp(join(tmpdir(), "fleet-feed-args-3509-home-"));
      const cwd = await mkdtemp(join(tmpdir(), "fleet-feed-args-3509-cwd-"));
      try {
        const { code, stderr } = await runScript(home, cwd, args);
        expect(code, `args ${JSON.stringify(args)}`).toBe(2);
        expect(stderr, `args ${JSON.stringify(args)}`).toContain("collect-fleet-feed:");
        expect(stderr, `args ${JSON.stringify(args)}`).toContain(names);
        expect(stderr, `args ${JSON.stringify(args)}`).not.toContain("TypeError");
        // No collection output: the default feed path is untouched and no
        // stray file (e.g. a relative --weird) lands in the working dir.
        expect(await exists(join(home, "feed.json")), `args ${JSON.stringify(args)}`).toBe(false);
        expect(await exists(join(home, "aa-last-good.json")), `args ${JSON.stringify(args)}`).toBe(false);
        expect(await readdir(cwd), `args ${JSON.stringify(args)}`).toEqual([]);
      } finally {
        await rm(home, { recursive: true, force: true });
        await rm(cwd, { recursive: true, force: true });
      }
    }
  }, 60000);

  test("valid explicit --out writes the requested path", async () => {
    const home = await mkdtemp(join(tmpdir(), "fleet-feed-args-3509-ok-"));
    const cwd = await mkdtemp(join(tmpdir(), "fleet-feed-args-3509-okcwd-"));
    try {
      const target = join(home, "requested.json");
      const { code, stdout } = await runScript(home, cwd, ["--out", target]);
      expect(code).toBe(0);
      expect(stdout).toContain(target);
      expect(await exists(target)).toBe(true);
      // The untouched default proves the explicit path was honored.
      expect(await exists(join(home, "feed.json"))).toBe(false);
    } finally {
      await rm(home, { recursive: true, force: true });
      await rm(cwd, { recursive: true, force: true });
    }
  }, 60000);

  test("no arguments keep the FLEET_FEED_OUT default", async () => {
    const home = await mkdtemp(join(tmpdir(), "fleet-feed-args-3509-default-"));
    const cwd = await mkdtemp(join(tmpdir(), "fleet-feed-args-3509-defaultcwd-"));
    try {
      const { code, stdout } = await runScript(home, cwd, []);
      expect(code).toBe(0);
      const fallback = join(home, "feed.json");
      expect(stdout).toContain(fallback);
      expect(await exists(fallback)).toBe(true);
    } finally {
      await rm(home, { recursive: true, force: true });
      await rm(cwd, { recursive: true, force: true });
    }
  }, 60000);
});
