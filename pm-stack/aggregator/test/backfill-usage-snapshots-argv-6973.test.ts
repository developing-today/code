// Pinning test for #6973: backfill-usage-snapshots must reject every
// argument other than a single separated `--dir <storeDir>` with a usage
// error (exit 2), before reading inputs or creating/writing any store.
// A typo'd `--dirr <dir>` used to be silently ignored and the rebuilt
// history was appended to the DEFAULT store, exit 0. The `--dir` value
// guard itself is pinned by backfill-usage-snapshots-dir-3330.test.ts.
import { describe, expect, test } from "bun:test";
import { mkdtemp, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/backfill-usage-snapshots.ts");

async function runScript(cwd: string, args: string[]): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    cwd,
    env: {
      ...process.env,
      PM_AGG_SNAPSHOTS_DIR: join(cwd, "default-store"),
      QUOTA_VIEW_PATH: join(cwd, "quota-view.md"),
      QUOTA_REGISTRY_PATH: join(cwd, "registry.md"),
    },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

describe("backfill-usage-snapshots strict argv (#6973)", () => {
  test("unknown, misspelled, positional, joined, and duplicate arguments exit 2 before any store is created", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "backfill-argv-6973-"));
    try {
      // Deliberately absent inputs prove validation precedes file reads.
      const cases: { args: string[]; named: string }[] = [
        { args: ["--bogus"], named: "--bogus" },
        { args: ["--dirr", join(cwd, "intended-store")], named: "--dirr" },
        { args: ["somefile"], named: "somefile" },
        { args: [`--dir=${join(cwd, "joined-store")}`], named: "--dir=" },
        { args: ["--dir", join(cwd, "dup-a"), "--dir", join(cwd, "dup-b")], named: "--dir" },
      ];
      for (const { args, named } of cases) {
        const { code, stderr } = await runScript(cwd, args);
        expect(code, `args ${JSON.stringify(args)}`).toBe(2);
        expect(stderr, `args ${JSON.stringify(args)}`).toContain(named);
      }
      // No store directory was created by any rejected invocation: neither
      // the env default nor any operator-named directory.
      for (const dir of ["default-store", "intended-store", "joined-store", "dup-a", "dup-b"]) {
        await expect(readdir(join(cwd, dir))).rejects.toMatchObject({ code: "ENOENT" });
      }
    } finally {
      await rm(cwd, { recursive: true, force: true });
    }
  }, 30000);
});
