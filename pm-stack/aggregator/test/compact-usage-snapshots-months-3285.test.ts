// Pinning test for #3285: compact-usage-snapshots accepted --months 0,
// and SnapshotStore.compact then treated the in-progress month as
// eligible — an --apply run replaced its raw file with a lossy
// min/max/last rollup. Values below 1 are a usage error (exit 2, the
// existing invalid --months message) before the store is read or
// modified, in both dry-run and --apply modes.
import { describe, expect, test } from "bun:test";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/compact-usage-snapshots.ts");

async function runScript(storeDir: string, args: string[]): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: { ...process.env, PM_AGG_SNAPSHOTS_DIR: storeDir },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

describe("compact-usage-snapshots --months validation (#3285)", () => {
  test("--months 0 exits 2 and never touches the store, dry-run or --apply", async () => {
    const dir = await mkdtemp(join(tmpdir(), "snap-cli-"));
    try {
      const month = new Date().toISOString().slice(0, 7);
      const name = `usage-${month}.jsonl`;
      const raw =
        JSON.stringify({
          ts: `${month}-05T12:00:00Z`,
          serviceId: "svc-a",
          metricId: "weekly",
          value: 12,
          unit: "pct",
          source: "api",
          observedBy: "test",
        }) + "\n";
      await writeFile(join(dir, name), raw);

      for (const args of [
        ["--months", "0"],
        ["--months", "0", "--apply"],
      ]) {
        const { code, stderr } = await runScript(dir, args);
        expect(code).toBe(2);
        expect(stderr).toContain("invalid --months '0'");
        expect(await readdir(dir)).toEqual([name]);
        expect(await readFile(join(dir, name), "utf8")).toBe(raw);
      }

      // Control: the smallest accepted retention runs fine and leaves
      // the in-progress month raw.
      const ok = await runScript(dir, ["--months", "1"]);
      expect(ok.code).toBe(0);
      expect(await readdir(dir)).toEqual([name]);
      expect(await readFile(join(dir, name), "utf8")).toBe(raw);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
