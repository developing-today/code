// Pinning test for #6895: record-claude-launch must refuse an empty or
// whitespace-only flag value with a usage error (exit 2) and NO record
// appended, instead of recording it verbatim with exit 0 — an --id ""
// record has no identity (the registry merge keys by (source, id), so
// every empty-id record folds into one phantom entry), a --parent ""
// is a parent that is not null, and a --file "" died with a raw ENOENT
// stack. Sibling guards (post-alert.ts, collect-fleet-feed.ts) already
// refuse empty values; this script's #3415 guard omitted the case.
import { describe, expect, test } from "bun:test";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/record-claude-launch.ts");

async function runScript(args: string[]): Promise<{ code: number; stderr: string; stdout: string }> {
  const proc = Bun.spawn([process.execPath, script, ...args], {
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

describe("record-claude-launch empty-value refusal (#6895)", () => {
  test("empty and whitespace-only values exit 2 and append nothing", async () => {
    const dir = await mkdtemp(join(tmpdir(), "record-launch-6895-"));
    try {
      const out = join(dir, "records.jsonl");
      const badArgSets: string[][] = [
        ["--file", out, "--label", "Slice", "--id", ""],
        ["--file", out, "--label", "Slice", "--id", " "],
        ["--file", out, "--label", "Slice", "--parent", ""],
        ["--file", out, "--label", "Slice", "--status", ""],
        ["--file", out, "--label", "Slice", "--model", ""],
        ["--file", out, "--label", ""],
        ["--file", out, "--label", " "],
        ["--file", out, "--label", "Slice", "--task", ""],
        ["--file", "", "--label", "Slice"],
      ];
      for (const args of badArgSets) {
        const { code, stderr } = await runScript(args);
        expect(code).toBe(2);
        expect(stderr).toContain("requires a value");
      }
      await expect(readFile(out, "utf8")).rejects.toThrow();
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("control: a normal invocation still records, label with spaces intact", async () => {
    const dir = await mkdtemp(join(tmpdir(), "record-launch-6895-"));
    try {
      const out = join(dir, "records.jsonl");
      const { code } = await runScript(["--file", out, "--label", "Slice 3 builder", "--parent", "claude-parent"]);
      expect(code).toBe(0);
      const lines = (await readFile(out, "utf8")).trim().split("\n");
      expect(lines).toHaveLength(1);
      const record = JSON.parse(lines[0]);
      expect(record.label).toBe("Slice 3 builder");
      expect(record.parentId).toBe("claude-parent");
      expect(record.status).toBe("launched");
      expect(typeof record.id).toBe("string");
      expect(record.id.length).toBeGreaterThan(0);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});
