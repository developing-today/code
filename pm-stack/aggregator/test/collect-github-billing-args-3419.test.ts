// Pinning test for #3419 (and #6922 for --by): collect-github-billing must validate explicit
// --year/--month/--user values BEFORE any gh call. Malformed values used
// to flow straight into the billing API URL (month=NaN, month=13,
// users/--print) and the failure was then misreported as a gh
// token-scope problem. Invalid values are a usage error (exit 2) with
// zero gh calls; omitted flags keep their defaults and a valid control
// calls the expected API path.
import { describe, expect, test } from "bun:test";
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/collect-github-billing.ts");

async function setupStubGh(dir: string): Promise<string> {
  const binDir = join(dir, "bin");
  const logPath = join(dir, "gh-calls.log");
  const { mkdir } = await import("node:fs/promises");
  await mkdir(binDir, { recursive: true });
  const stub = join(binDir, "gh");
  await writeFile(stub, '#!/bin/sh\necho "$@" >> "$GH_STUB_LOG"\nprintf \'{"usageItems":[]}\'\n');
  await chmod(stub, 0o755);
  return logPath;
}

async function runScript(
  dir: string,
  logPath: string,
  args: string[],
): Promise<{ code: number; stderr: string; stdout: string }> {
  const env = { ...process.env };
  delete env.GH_BILLING_USER;
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: {
      ...env,
      PATH: `${join(dir, "bin")}:${process.env.PATH}`,
      GH_STUB_LOG: logPath,
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

describe("collect-github-billing argument validation (#3419)", () => {
  test("invalid explicit values exit 2 with zero gh calls and no token-scope blame", async () => {
    const dir = await mkdtemp(join(tmpdir(), "billing-args-3419-"));
    try {
      const logPath = await setupStubGh(dir);
      const badArgSets: string[][] = [
        ["--year", "banana"],
        ["--month", "13"],
        ["--month", "0"],
        ["--year", "2026.5"],
        ["--month", "2.5"],
        ["--month", "--print"],
        ["--user", "--print"],
        ["--user"],
        ["--year"],
        ["--month"],
        ["--year", "1999"],
        ["--user", "   "],
        ["--by"],
        ["--by", ""],
        ["--by", "   "],
        ["--by", "--print"],
      ];
      for (const args of badArgSets) {
        const { code, stderr } = await runScript(dir, logPath, args);
        expect(code, `args ${JSON.stringify(args)}`).toBe(2);
        expect(stderr, `args ${JSON.stringify(args)}`).toContain("collect-github-billing:");
        expect(stderr, `args ${JSON.stringify(args)}`).not.toContain("scope");
        expect(await ghCalls(logPath), `args ${JSON.stringify(args)}`).toBe("");
      }
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 60000);

  test("valid control calls the expected API path and derives records", async () => {
    const dir = await mkdtemp(join(tmpdir(), "billing-args-3419-ok-"));
    try {
      const logPath = await setupStubGh(dir);
      const { code, stdout } = await runScript(dir, logPath, [
        "--year",
        "2026",
        "--month",
        "10",
        "--user",
        "octocat",
        "--print",
      ]);
      expect(code).toBe(0);
      expect(await ghCalls(logPath)).toContain("api users/octocat/settings/billing/usage?year=2026&month=10");
      expect(stdout).toContain("0 record(s) (dry run, outbox untouched)");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 60000);

  test("a valid --by is recorded verbatim as observedBy (#6922)", async () => {
    const dir = await mkdtemp(join(tmpdir(), "billing-args-6922-by-"));
    try {
      const binDir = join(dir, "bin");
      const { mkdir } = await import("node:fs/promises");
      await mkdir(binDir, { recursive: true });
      const stub = join(binDir, "gh");
      await writeFile(
        stub,
        '#!/bin/sh\nprintf \'{"usageItems":[{"date":"2026-10-15","product":"actions","unitType":"minutes","quantity":10,"grossAmount":1,"netAmount":1}]}\n\'',
      );
      await chmod(stub, 0o755);
      const env = { ...process.env };
      delete env.GH_BILLING_USER;
      const proc = Bun.spawn([process.execPath, script, "--by", "stewardship-cycle", "--print"], {
        env: { ...env, PATH: `${binDir}:${process.env.PATH}` },
        stdout: "pipe",
        stderr: "pipe",
      });
      const [stdout] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text()]);
      expect(await proc.exited).toBe(0);
      expect(stdout).toContain('\"observedBy\":\"stewardship-cycle\"');
      expect(stdout).not.toContain('\"observedBy\":\"collect-github-billing\"');
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 60000);

  test("omitted flags keep their defaults", async () => {
    const dir = await mkdtemp(join(tmpdir(), "billing-args-3419-defaults-"));
    try {
      const logPath = await setupStubGh(dir);
      const { code } = await runScript(dir, logPath, ["--print"]);
      expect(code).toBe(0);
      const now = new Date();
      const expected =
        `api users/dezren39/settings/billing/usage?year=${now.getUTCFullYear()}` + `&month=${now.getUTCMonth() + 1}`;
      expect(await ghCalls(logPath)).toContain(expected);
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 60000);
});
