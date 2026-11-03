// Pinning test for #3330: backfill-usage-snapshots must reject a missing
// or flag-like --dir value with a usage error (exit 2), before reading
// inputs or creating a store. A valid directory remains accepted.
import { describe, expect, test } from "bun:test";
import { mkdtemp, mkdir, readdir, rm, writeFile } from "node:fs/promises";
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

describe("backfill-usage-snapshots --dir validation (#3330)", () => {
  test("missing or flag-like --dir exits 2 before reading inputs or creating a directory", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "backfill-dir-3330-bad-"));
    try {
      // Deliberately absent inputs prove validation precedes file reads.
      for (const args of [["--dir"], ["--dir", "--print"]]) {
        const { code, stderr } = await runScript(cwd, args);
        expect(code, `args ${JSON.stringify(args)}`).toBe(2);
        expect(stderr).toContain("--dir");
      }
      await expect(readdir(join(cwd, "--print"))).rejects.toMatchObject({ code: "ENOENT" });
    } finally {
      await rm(cwd, { recursive: true, force: true });
    }
  }, 30000);

  test("valid --dir and the absent --dir default still run", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "backfill-dir-3330-ok-"));
    try {
      await writeFile(join(cwd, "quota-view.md"), "");
      await writeFile(join(cwd, "registry.md"), "");
      const store = join(cwd, "custom-store");
      await mkdir(store);
      const valid = await runScript(cwd, ["--dir", store]);
      expect(valid.code).toBe(0);
      expect(valid.stdout).toContain(store);

      const defaultRun = await runScript(cwd, []);
      expect(defaultRun.code).toBe(0);
    } finally {
      await rm(cwd, { recursive: true, force: true });
    }
  }, 30000);
});
