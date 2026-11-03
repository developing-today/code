// Pinning test for #3295: append-usage-snapshots must reject a missing,
// empty, or flag-like --by value with a usage error (exit 2) BEFORE any
// file is read or written, in both modes — instead of silently deriving
// records whose empty observedBy fails validation and discarding the
// whole run with exit 0. Valid names and the no---by defaults keep
// working as before.
import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/append-usage-snapshots.ts");

async function runScript(
  dir: string,
  args: string[],
): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: {
      ...process.env,
      QUOTA_VIEW_PATH: join(dir, "quota-view.md"),
      QUOTA_REGISTRY_PATH: join(dir, "registry.md"),
      PROVIDER_USAGE_SNAPSHOT_PATH: join(dir, "provider-usage.json"),
      USAGE_SNAPSHOT_OUTBOX_PATH: join(dir, "outbox.jsonl"),
    },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  return { code: await proc.exited, stdout, stderr };
}

describe("append-usage-snapshots --by validation (#3295)", () => {
  test("malformed --by exits 2 and leaves the outbox untouched in both modes", async () => {
    const dir = await mkdtemp(join(tmpdir(), "append-by-3295-"));
    try {
      const outbox = join(dir, "outbox.jsonl");
      await writeFile(outbox, "sentinel\n");
      // Deliberately no quota-view / provider-usage files exist: if the
      // script read its inputs before validating --by, these runs would
      // fail with a file error instead of the --by usage error.
      const badArgSets: string[][] = [
        ["--by"],
        ["--by", ""],
        ["--by", "   "],
        ["--by", "--from-provider-usage"],
        ["--from-provider-usage", "--by"],
        ["--from-provider-usage", "--by", ""],
        ["--from-provider-usage", "--by", "--by"],
      ];
      for (const args of badArgSets) {
        const { code, stderr } = await runScript(dir, args);
        expect(code, `args ${JSON.stringify(args)}`).toBe(2);
        expect(stderr).toContain("--by requires a non-empty name");
        expect(await readFile(outbox, "utf8")).toBe("sentinel\n");
      }
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 30000);

  test("valid --by and the absent---by default still run (empty inputs derive no records)", async () => {
    const dir = await mkdtemp(join(tmpdir(), "append-by-3295-ok-"));
    try {
      await writeFile(join(dir, "quota-view.md"), "");
      for (const args of [[], ["--by", "tester"]]) {
        const { code, stdout } = await runScript(dir, args);
        expect(code, `args ${JSON.stringify(args)}`).toBe(0);
        expect(stdout).toContain("no records derived");
      }
      // A --from-provider-usage run with an empty snapshot object and a
      // valid --by also succeeds without touching the outbox.
      await writeFile(join(dir, "provider-usage.json"), "{}");
      const pu = await runScript(dir, ["--from-provider-usage", "--by", "tester"]);
      expect(pu.code).toBe(0);
      expect(await Bun.file(join(dir, "outbox.jsonl")).exists()).toBe(false);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 30000);
});
