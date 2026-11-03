import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, test } from "bun:test";
import { isSessionRecord, parseLaunchRecords } from "../src/registry.ts";
import { parseRef, refUrl, type SessionRecord } from "../src/model.ts";
import { renderSessions, sessionRecordLine } from "../src/render.ts";
import { fixtureSource } from "../src/sources/fixtures.ts";

function rec(partial: Partial<SessionRecord> & { id: string }): SessionRecord {
  return {
    source: "claude-code",
    label: partial.id,
    agentId: "claude-code",
    agentLabel: "Muse",
    parentId: null,
    status: "succeeded",
    taskRefs: [],
    ...partial,
  } as SessionRecord;
}

describe("registry task-ref validation (#2485)", () => {
  test("isSessionRecord rejects absent refs, accepts well-formed refs, rejects malformed elements", () => {
    expect(isSessionRecord({ id: "sparse", source: "claude-code", label: "Sparse" })).toBe(false);
    expect(isSessionRecord(rec({ id: "good", taskRefs: ["developing-today/code#2485"] }))).toBe(true);
    expect(isSessionRecord(rec({ id: "short", taskRefs: ["code#2480"] }))).toBe(false);
    expect(isSessionRecord(rec({ id: "number", taskRefs: [42] as unknown as string[] }))).toBe(false);
    expect(isSessionRecord(rec({ id: "large", taskRefs: ["owner/repo#12345678901234567890123"] }))).toBe(false);
  });

  test("launch parsing drops malformed refs but preserves well-formed records", () => {
    const jsonl = [
      JSON.stringify({ id: "launch-good", taskRefs: ["developing-today/code#2485"] }),
      JSON.stringify({ id: "launch-short", taskRefs: ["code#2480"] }),
      JSON.stringify({ id: "launch-large", taskRefs: ["owner/repo#12345678901234567890123"] }),
    ].join("\n");

    expect(parseLaunchRecords(jsonl).map((record) => record.id)).toEqual(["launch-good"]);
  });

  test("unsafe task-ref numbers are rejected by parseRef and refUrl", () => {
    expect(parseRef("developing-today/code#2485")).toEqual({ repo: "developing-today/code", number: 2485 });
    expect(parseRef("owner/repo#9007199254740991").number).toBe(Number.MAX_SAFE_INTEGER);
    expect(() => parseRef("owner/repo#9007199254740992")).toThrow();
    expect(() => parseRef("owner/repo#12345678901234567890123")).toThrow();
    expect(() => refUrl("owner/repo#12345678901234567890123")).toThrow();
  });

  test("rendering skips poisoned refs while retaining valid links", () => {
    const poisoned = rec({
      id: "ses_poisoned",
      taskRefs: ["code#2480", "developing-today/code#2485", "owner/repo#12345678901234567890123"],
    });

    expect(() => sessionRecordLine(poisoned)).not.toThrow();
    const line = sessionRecordLine(poisoned);
    expect(line).toContain('href="https://github.com/developing-today/code/issues/2485"');
    expect(line).not.toContain("code#2480");
    expect(line).not.toContain("12345678901234567890123");

    const html = renderSessions([poisoned], fixtureSource().provenance());
    expect(html).toContain('href="https://github.com/developing-today/code/issues/2485"');
    expect(html).not.toContain("code#2480");
  });

  test("launch writer refuses malformed --task values without writing a record", async () => {
    const temp = await mkdtemp(join(tmpdir(), "record-claude-launch-"));
    const file = join(temp, "records.jsonl");
    const script = join(import.meta.dir, "../scripts/record-claude-launch.ts");
    try {
      const proc = Bun.spawn(
        [process.execPath, script, "--label", "Invalid task", "--task", "code#2480", "--file", file],
        { cwd: join(import.meta.dir, ".."), stdout: "pipe", stderr: "pipe" },
      );
      const [exitCode, stderr] = await Promise.all([proc.exited, new Response(proc.stderr).text()]);

      expect(exitCode).not.toBe(0);
      expect(stderr).toContain("expected owner/repo#123");
      await expect(readFile(file, "utf8")).rejects.toThrow();

      const validFile = join(temp, "valid-records.jsonl");
      const validProc = Bun.spawn(
        [
          process.execPath,
          script,
          "--label",
          "Valid task",
          "--task",
          "developing-today/code#2485",
          "--file",
          validFile,
        ],
        { cwd: join(import.meta.dir, ".."), stdout: "pipe", stderr: "pipe" },
      );
      expect(await validProc.exited).toBe(0);
      const record = JSON.parse(await readFile(validFile, "utf8"));
      expect(record.taskRefs).toEqual(["developing-today/code#2485"]);
    } finally {
      await rm(temp, { recursive: true, force: true });
    }
  });
});
