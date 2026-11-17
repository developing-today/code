// Strict argv for the projector CLI (#3352, #8650).
//
// Oracle: the process exit code plus the filesystem. A usage error must exit
// 2, name the offending argument on stderr, and leave the store untouched —
// the DB file at DATABASE_URL must not even be created, which proves the
// validation ran before openDb/migrate (and therefore before any fetch,
// write or consumer ack).
import { describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const CLI = join(import.meta.dir, "..", "src", "cli.ts");

function run(argv: string[]) {
  const dir = mkdtempSync(join(tmpdir(), "projector-cli-argv-"));
  const dbPath = join(dir, "projector.db");
  const proc = spawnSync(process.execPath, ["run", CLI, ...argv], {
    env: { ...process.env, DATABASE_URL: `file:${dbPath}`, PM_REPOS: "" },
    encoding: "utf8",
    timeout: 30_000,
  });
  return { code: proc.status, stdout: proc.stdout, stderr: proc.stderr, dbCreated: existsSync(dbPath) };
}

describe("projector CLI strict argv (#8650)", () => {
  test.each([
    [["reconcile", "--extra"], "--extra"],
    [["reconcile", "--no-ack"], "--no-ack"],
    [["queue-view", "--tasks", "/nonexistent.json", "--taks", "/x.json"], "--taks"],
    [["queue-view", "--tasks"], "--tasks"],
    [["queue-view", "--tasks", "/nonexistent.json", "--format"], "--format"],
    [["queue-view", "--tasks", "--format", "json"], "--tasks"],
    [["queue-view", "--tasks", "/nonexistent.json", "stray"], "stray"],
    [["queue-view", "--tasks", "/nonexistent.json", "--format", "yaml"], "--format must be json or html"],
  ])("refuses %j naming %s, exit 2, no store created", (argv, named) => {
    const r = run(argv);
    expect(r.code).toBe(2);
    expect(r.stderr).toContain(named);
    expect(r.dbCreated).toBe(false);
    expect(r.stdout).toBe("");
  });

  test("unknown subcommand is a usage error before any store", () => {
    const r = run(["reconcille"]);
    expect(r.code).toBe(2);
    expect(r.stderr).toContain("reconcille");
    expect(r.dbCreated).toBe(false);
  });
});
