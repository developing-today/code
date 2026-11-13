// Pinning tests for #7332: record-claude-launch must reject unknown,
// misspelled, joined, duplicate, and stray arguments with a usage error
// (exit 2) BEFORE any file is written. The pre-fix parser only probed
// argv with indexOf, so a typo --task silently dropped the session
// task linkage the script exists to create, a joined --task=... did
// the same, a typo --fil wrote the record to the DEFAULT registry file
// instead of the intended one, a typo --stats recorded the default
// status, and duplicate single-value flags silently used the first
// value -- all exit 0. Values were already hardened in #3415/#6895;
// this is the unknown-token half, mirroring the sibling guards
// (#6961/#7097/#7254/#7289/#7302).
import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/record-claude-launch.ts");

async function runScript(args: string[], home: string): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: { ...process.env, HOME: home },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

describe("record-claude-launch unknown arguments (#7332)", () => {
  test("control: documented invocation records exactly what it names, repeated --task included", async () => {
    const dir = await mkdtemp(join(tmpdir(), "record-launch-7332-"));
    try {
      const out = join(dir, "records.jsonl");
      const { code } = await runScript(
        [
          "--label",
          "Control",
          "--id",
          "claude-ctrl",
          "--parent",
          "sess-1",
          "--model",
          "sonnet",
          "--status",
          "done",
          "--task",
          "developing-today/code#123",
          "--task",
          "developing-today/code#456",
          "--file",
          out,
        ],
        dir,
      );
      expect(code).toBe(0);
      const record = JSON.parse((await readFile(out, "utf8")).trim());
      expect(record).toMatchObject({
        id: "claude-ctrl",
        label: "Control",
        parentId: "sess-1",
        status: "done",
        model: "sonnet",
        taskRefs: ["developing-today/code#123", "developing-today/code#456"],
      });
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("unknown, misspelled, joined, duplicate, and stray arguments exit 2 and append nothing anywhere", async () => {
    const dir = await mkdtemp(join(tmpdir(), "record-launch-7332-"));
    try {
      const out = join(dir, "records.jsonl");
      const defaultFile = join(
        dir,
        "workspace/goals/keep-the-four-repos-healthy/hidden_files/claude-launch-records.jsonl",
      );
      const badArgSets: { args: string[]; names: string }[] = [
        // typo --task used to drop the task linkage silently
        { args: ["--label", "X", "--tas", "developing-today/code#123", "--file", out], names: "--tas" },
        // joined --task= used to drop the task linkage silently
        {
          args: ["--label", "X", "--task=developing-today/code#123", "--file", out],
          names: "--task=developing-today/code#123",
        },
        // typo --fil used to write the record to the default registry file
        { args: ["--label", "X", "--fil", out], names: "--fil" },
        // typo --stats used to record the default status
        { args: ["--label", "X", "--stats", "done", "--file", out], names: "--stats" },
        // typo --modle used to drop the model
        { args: ["--label", "X", "--modle", "sonnet", "--file", out], names: "--modle" },
        // joined --model= used to drop the model
        { args: ["--label", "X", "--model=sonnet", "--file", out], names: "--model=sonnet" },
        // unknown flag
        { args: ["--label", "X", "--bogus", "1", "--file", out], names: "--bogus" },
        // duplicate single-value flags: indexOf silently used the first
        { args: ["--label", "First", "--label", "Second", "--file", out], names: "--label" },
        { args: ["--label", "X", "--id", "a", "--id", "b", "--file", out], names: "--id" },
        // stray positional
        { args: ["--label", "X", "stray", "--file", out], names: "stray" },
      ];
      for (const { args, names } of badArgSets) {
        const { code, stderr } = await runScript(args, dir);
        expect(code).toBe(2);
        expect(stderr).toContain(names);
      }
      // no record was appended to the intended file or the default file
      await expect(readFile(out, "utf8")).rejects.toThrow();
      await expect(readFile(defaultFile, "utf8")).rejects.toThrow();
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
