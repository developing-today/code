// Pinning test for #3415: record-claude-launch must refuse a value-taking
// option whose next token is missing or is itself a flag — with a usage
// error (exit 2) and NO record appended — instead of accepting the flag
// token as the value, consuming it a second time as a flag, and appending
// a corrupted launch record with exit 0. Control behaviour is unchanged.
import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/record-claude-launch.ts");

async function runScript(
  args: string[],
): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  return { code: await proc.exited, stdout, stderr };
}

describe("record-claude-launch flag-as-value refusal (#3415)", () => {
  test("flag-as-value and missing values exit 2 and append nothing", async () => {
    const dir = await mkdtemp(join(tmpdir(), "record-launch-3415-"));
    try {
      const out = join(dir, "records.jsonl");
      const badArgSets: string[][] = [
        // --task takes the --model flag as its value (taskRefs ['--model'])
        ["--file", out, "--label", "Slice", "--task", "--model", "gpt-x"],
        // --label takes the --id flag as its value (a session named '--id')
        ["--file", out, "--label", "--id", "claude-x"],
        // --model takes the --file flag as its value
        ["--file", out, "--label", "Slice", "--model", "--file", join(dir, "other.jsonl")],
        // a value flag in last position has no value at all
        ["--file", out, "--label", "Slice", "--model"],
        ["--file", out, "--label"],
      ];
      for (const args of badArgSets) {
        const { code, stderr } = await runScript(args);
        expect(code).toBe(2);
        expect(stderr).toContain("requires a value");
      }
      // No invocation above may have appended a record anywhere.
      await expect(readFile(out, "utf8")).rejects.toThrow();
      await expect(readFile(join(dir, "other.jsonl"), "utf8")).rejects.toThrow();
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("control: a normal invocation records exactly the given label and refs", async () => {
    const dir = await mkdtemp(join(tmpdir(), "record-launch-3415-"));
    try {
      const out = join(dir, "records.jsonl");
      const { code } = await runScript([
        "--file", out,
        "--label", "Slice 3 builder",
        "--id", "claude-ctl",
        "--task", "developing-today/code#123",
        "--task", "developing-today/code#456",
        "--model", "claude-x",
      ]);
      expect(code).toBe(0);
      const lines = (await readFile(out, "utf8")).trim().split("\n");
      expect(lines).toHaveLength(1);
      const record = JSON.parse(lines[0]);
      expect(record.label).toBe("Slice 3 builder");
      expect(record.id).toBe("claude-ctl");
      expect(record.model).toBe("claude-x");
      expect(record.taskRefs).toEqual(["developing-today/code#123", "developing-today/code#456"]);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
