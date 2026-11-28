// Pinning test for #7254: collect-github-billing must reject unknown,
// duplicated, or positional arguments BEFORE any gh call or outbox
// write. Argv used to be probed token-by-token only (includes/indexOf
// in the resolvers), so an unrecognized token was silently ignored: a
// typo'd --yer fell back to the current year, a typo'd --prnit turned
// a --print dry run into a real outbox append, and a duplicated flag
// silently took the first value. Same harness style as
// collect-github-billing-args-3419.test.ts (stub gh on PATH).
import { describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/collect-github-billing.ts");

async function setupStubGh(dir: string): Promise<string> {
  const binDir = join(dir, "bin");
  const logPath = join(dir, "gh-calls.log");
  await mkdir(binDir, { recursive: true });
  const stub = join(binDir, "gh");
  await writeFile(
    stub,
    "#!/bin/sh\necho \"$@\" >> \"$GH_STUB_LOG\"\nprintf '{\"usageItems\":[{\"date\":\"2026-11-05\",\"product\":\"actions\",\"unitType\":\"minutes\",\"quantity\":42,\"grossAmount\":1.5,\"netAmount\":1.5}]}\\n'",
  );
  await chmod(stub, 0o755);
  return logPath;
}

async function runScript(
  dir: string,
  logPath: string,
  outboxPath: string,
  args: string[],
): Promise<{ code: number; stderr: string; stdout: string }> {
  const env = { ...process.env };
  delete env.GH_BILLING_USER;
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: {
      ...env,
      PATH: `${join(dir, "bin")}:${process.env.PATH}`,
      GH_STUB_LOG: logPath,
      USAGE_SNAPSHOT_OUTBOX_PATH: outboxPath,
    },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
  return { code: await proc.exited, stdout, stderr };
}

async function ghCalls(logPath: string): Promise<string> {
  try {
    return await readFile(logPath, "utf8");
  } catch {
    return "";
  }
}

describe("collect-github-billing unknown arguments (#7254)", () => {
  test("unknown, typo'd, duplicated, and positional arguments exit 2 with zero gh calls and no outbox write", async () => {
    const dir = await mkdtemp(join(tmpdir(), "billing-argv-7254-"));
    try {
      const logPath = await setupStubGh(dir);
      const outboxPath = join(dir, "outbox.jsonl");
      const badArgSets: string[][] = [
        ["--bogus"],
        ["--yer", "2024", "--month", "3", "--print"],
        ["--year", "2024", "--month", "3", "--prnit"],
        ["--year", "2024", "--year", "2025", "--month", "3", "--print"],
        ["--print", "--print"],
        ["2024"],
        ["--year", "2024", "--month", "3", "--print", "extra"],
      ];
      for (const args of badArgSets) {
        const { code, stderr } = await runScript(dir, logPath, outboxPath, args);
        expect(code, `args ${JSON.stringify(args)}`).toBe(2);
        expect(stderr, `args ${JSON.stringify(args)}`).toContain("collect-github-billing:");
        expect(await ghCalls(logPath), `args ${JSON.stringify(args)}`).toBe("");
        expect(existsSync(outboxPath), `args ${JSON.stringify(args)}`).toBe(false);
      }
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 60000);

  test("valid control still collects the requested month as a dry run", async () => {
    const dir = await mkdtemp(join(tmpdir(), "billing-argv-7254-ok-"));
    try {
      const logPath = await setupStubGh(dir);
      const outboxPath = join(dir, "outbox.jsonl");
      const { code, stdout } = await runScript(dir, logPath, outboxPath, [
        "--year",
        "2024",
        "--month",
        "3",
        "--print",
      ]);
      expect(code).toBe(0);
      expect(await ghCalls(logPath)).toContain("api users/dezren39/settings/billing/usage?year=2024&month=3");
      expect(stdout).toContain("3 record(s) (dry run, outbox untouched)");
      expect(existsSync(outboxPath)).toBe(false);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 60000);
});
