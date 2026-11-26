// Pinning test for #15329: collect-github-billing must validate the
// --user value's SHAPE (and the GH_BILLING_USER fallback's) BEFORE any
// gh call. #3419 checked only presence/non-blank and returned the raw
// untrimmed string, so "a/b" rewrote the gh api path to a different
// endpoint and "foo bar" / " octocat " failed downstream with the
// token-scope misdiagnosis. Invalid shapes are a usage error (exit 2)
// naming the offending value, with zero gh calls; a valid control
// still calls users/octocat/settings/billing/usage.
import { describe, expect, test } from "bun:test";
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const script = join(import.meta.dir, "../scripts/collect-github-billing.ts");

async function setupStubGh(dir: string): Promise<string> {
  const binDir = join(dir, "bin");
  const logPath = join(dir, "gh-calls.log");
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
  envOverride: Record<string, string> = {},
): Promise<{ code: number; stderr: string; stdout: string }> {
  const env = { ...process.env };
  delete env.GH_BILLING_USER;
  const proc = Bun.spawn([process.execPath, script, ...args], {
    env: {
      ...env,
      PATH: `${join(dir, "bin")}:${process.env.PATH}`,
      GH_STUB_LOG: logPath,
      ...envOverride,
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

describe("collect-github-billing --user shape validation (#15329)", () => {
  test("malformed --user values exit 2 with zero gh calls, naming the value", async () => {
    const dir = await mkdtemp(join(tmpdir(), "billing-user-15329-"));
    try {
      const logPath = await setupStubGh(dir);
      const badUsers = ["a/b", "foo bar", " octocat ", "-octocat", "octocat-", "octo_cat", "a".repeat(40)];
      for (const user of badUsers) {
        const { code, stderr } = await runScript(dir, logPath, ["--user", user]);
        expect(code, `--user ${JSON.stringify(user)}`).toBe(2);
        expect(stderr, `--user ${JSON.stringify(user)}`).toContain("collect-github-billing:");
        expect(stderr, `--user ${JSON.stringify(user)}`).toContain(JSON.stringify(user));
        expect(stderr, `--user ${JSON.stringify(user)}`).not.toContain("scope");
        expect(await ghCalls(logPath), `--user ${JSON.stringify(user)}`).toBe("");
      }
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 60000);

  test("malformed GH_BILLING_USER exits 2 with zero gh calls", async () => {
    const dir = await mkdtemp(join(tmpdir(), "billing-user-15329-env-"));
    try {
      const logPath = await setupStubGh(dir);
      const { code, stderr } = await runScript(dir, logPath, [], { GH_BILLING_USER: "a/b" });
      expect(code).toBe(2);
      expect(stderr).toContain("GH_BILLING_USER");
      expect(stderr).toContain(JSON.stringify("a/b"));
      expect(await ghCalls(logPath)).toBe("");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 60000);

  test("valid control still calls users/octocat/settings/billing/usage", async () => {
    const dir = await mkdtemp(join(tmpdir(), "billing-user-15329-ok-"));
    try {
      const logPath = await setupStubGh(dir);
      const { code } = await runScript(dir, logPath, ["--user", "octocat", "--print"]);
      expect(code).toBe(0);
      expect(await ghCalls(logPath)).toContain("api users/octocat/settings/billing/usage");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 60000);

  test("valid GH_BILLING_USER is used when --user is omitted", async () => {
    const dir = await mkdtemp(join(tmpdir(), "billing-user-15329-env-ok-"));
    try {
      const logPath = await setupStubGh(dir);
      const { code } = await runScript(dir, logPath, ["--print"], { GH_BILLING_USER: "octocat" });
      expect(code).toBe(0);
      expect(await ghCalls(logPath)).toContain("api users/octocat/settings/billing/usage");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  }, 60000);
});
